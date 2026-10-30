//! The schema, as an ordered list of migrations, and the code that runs the
//! ones a database is missing.

use super::*;

/// The migrations that moved photos' and faces' vectors to their own tables,
/// after either of which [`open`] compacts the file once.
pub(super) const EMBEDDINGS_MOVED: [u32; 2] = [45, 46];

/// Ordered schema migrations. `PRAGMA user_version` records how many have run,
/// so a database only ever executes the ones it is missing.
///
/// **Append only.** Never edit or reorder an existing entry — databases in the
/// wild have already run it, and the version number is the only record of that.
/// A schema change is a new entry on the end.
///
/// Migration 1 keeps its `IF NOT EXISTS` clauses throughout because databases
/// created before this system existed already have that schema but still report
/// `user_version = 0`; re-running it against them has to be a no-op.
pub(super) const MIGRATIONS: &[&str] = &[
    // 1 — initial schema.
    r#"
        CREATE TABLE IF NOT EXISTS photos (
            id               INTEGER PRIMARY KEY,
            path             TEXT NOT NULL UNIQUE,
            filename         TEXT NOT NULL,
            file_hash        TEXT NOT NULL,
            capture_date     TEXT,
            camera           TEXT,
            lens             TEXT,
            iso              INTEGER,
            aperture         REAL,
            shutter          TEXT,
            focal_length     REAL,
            width            INTEGER NOT NULL DEFAULT 0,
            height           INTEGER NOT NULL DEFAULT 0,
            orientation      INTEGER NOT NULL DEFAULT 1,
            thumb_path       TEXT,
            preview_path     TEXT,
            rating           INTEGER NOT NULL DEFAULT 0,
            -- Phase 5 seam: non-destructive edit stack for the future editor.
            develop_settings TEXT,
            tagged_at        TEXT,
            indexed_at       TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS tags (
            id       INTEGER PRIMARY KEY,
            name     TEXT NOT NULL,
            category TEXT NOT NULL,
            UNIQUE(name, category)
        );

        CREATE TABLE IF NOT EXISTS photo_tags (
            photo_id   INTEGER NOT NULL REFERENCES photos(id) ON DELETE CASCADE,
            tag_id     INTEGER NOT NULL REFERENCES tags(id) ON DELETE CASCADE,
            confidence REAL,
            source     TEXT NOT NULL DEFAULT 'ai',
            PRIMARY KEY (photo_id, tag_id)
        );

        CREATE TABLE IF NOT EXISTS places (
            id   INTEGER PRIMARY KEY,
            name TEXT NOT NULL UNIQUE,
            lat  REAL,
            lon  REAL
        );

        CREATE TABLE IF NOT EXISTS trips (
            id         INTEGER PRIMARY KEY,
            name       TEXT NOT NULL,
            place_id   INTEGER REFERENCES places(id) ON DELETE SET NULL,
            start_date TEXT,
            end_date   TEXT
        );

        -- One place per photo keeps the model simple and matches how people
        -- actually think about where a shot was taken.
        CREATE TABLE IF NOT EXISTS photo_places (
            photo_id INTEGER PRIMARY KEY REFERENCES photos(id) ON DELETE CASCADE,
            place_id INTEGER REFERENCES places(id) ON DELETE CASCADE,
            trip_id  INTEGER REFERENCES trips(id) ON DELETE SET NULL
        );

        CREATE TABLE IF NOT EXISTS settings (
            key   TEXT PRIMARY KEY,
            value TEXT NOT NULL
        );

        -- These three indexes are what keep the combined filter instant.
        CREATE INDEX IF NOT EXISTS idx_photos_capture   ON photos(capture_date);
        CREATE INDEX IF NOT EXISTS idx_photos_tagged    ON photos(tagged_at);
        CREATE INDEX IF NOT EXISTS idx_phototags_tag    ON photo_tags(tag_id);
        CREATE INDEX IF NOT EXISTS idx_phototags_photo  ON photo_tags(photo_id);
        CREATE INDEX IF NOT EXISTS idx_photoplaces_place ON photo_places(place_id);
        "#,
    // 2 — develop presets. `settings` holds an `EditStack` as JSON, the same
    // shape stored in `photos.develop_settings`, so a preset and a photo's
    // edits are interchangeable by construction.
    r#"
        CREATE TABLE IF NOT EXISTS presets (
            id         INTEGER PRIMARY KEY,
            name       TEXT NOT NULL UNIQUE,
            settings   TEXT NOT NULL,
            created_at TEXT NOT NULL
        );
        "#,
    // 3 — library depth: pick/reject flags, colour labels, and collections.
    // `flag` and `color_label` are per-photo columns rather than tag rows: they
    // are single-valued (a photo has one flag, one label) and change constantly
    // during a cull, so a column set is cheaper than an insert/delete pair and
    // filters with a plain index. `flag` is the usual tri-state — reject / none
    // / pick — and `color_label` the usual five labels, 0 meaning none.
    r#"
        ALTER TABLE photos ADD COLUMN flag        INTEGER NOT NULL DEFAULT 0;
        ALTER TABLE photos ADD COLUMN color_label INTEGER NOT NULL DEFAULT 0;

        -- Collections are ordered, named groupings of photos, independent of the
        -- facet filters. A photo can be in many.
        CREATE TABLE IF NOT EXISTS collections (
            id         INTEGER PRIMARY KEY,
            name       TEXT NOT NULL UNIQUE,
            created_at TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS collection_photos (
            collection_id INTEGER NOT NULL REFERENCES collections(id) ON DELETE CASCADE,
            photo_id      INTEGER NOT NULL REFERENCES photos(id) ON DELETE CASCADE,
            added_at      TEXT NOT NULL,
            PRIMARY KEY (collection_id, photo_id)
        );

        CREATE INDEX IF NOT EXISTS idx_photos_flag        ON photos(flag);
        CREATE INDEX IF NOT EXISTS idx_photos_colorlabel  ON photos(color_label);
        CREATE INDEX IF NOT EXISTS idx_collphotos_photo   ON collection_photos(photo_id);
        "#,
    // 4 — smart collections: a named, saved `PhotoFilter`. Stored as the same
    // JSON the frontend already round-trips, so a saved query and a live filter
    // are interchangeable by construction — the same relationship presets have
    // with a photo's edit stack.
    r#"
        CREATE TABLE IF NOT EXISTS smart_collections (
            id         INTEGER PRIMARY KEY,
            name       TEXT NOT NULL UNIQUE,
            filter     TEXT NOT NULL,
            created_at TEXT NOT NULL
        );
        "#,
    // 5 — semantic search: one CLIP image embedding per photo, stored as a raw
    // little-endian f32 blob (see `search::encode_embedding`). Nullable, so a
    // freshly-scanned photo is simply "not indexed yet" until the embed job runs.
    r#"
        ALTER TABLE photos ADD COLUMN embedding BLOB;
        "#,
    // 6 — VLM captions + full-text search. `photos.caption` holds a Qwen-written
    // description (subjects / setting / colour / activity / mood / visible text),
    // nullable until the captioning job runs. `captions_fts` is an FTS5 index
    // *external-content* over photos — it stores no copy of the text, only the
    // inverted index keyed on the photo's rowid, and the triggers keep it in
    // sync so a caption search is a single MATCH. This is the OCR/keyword half of
    // search; CLIP embeddings remain the "looks like / is about" half.
    r#"
        ALTER TABLE photos ADD COLUMN caption TEXT;

        CREATE VIRTUAL TABLE captions_fts USING fts5(
            caption,
            content='photos',
            content_rowid='id',
            tokenize='unicode61 remove_diacritics 2'
        );

        -- Keep the index in step with photos.caption. Guarded on NULL so a photo
        -- with no caption yet never enters the index (and an external-content
        -- 'delete' is only issued for text that was actually indexed).
        CREATE TRIGGER photos_caption_ai AFTER INSERT ON photos BEGIN
            INSERT INTO captions_fts(rowid, caption)
                SELECT new.id, new.caption WHERE new.caption IS NOT NULL;
        END;
        CREATE TRIGGER photos_caption_ad AFTER DELETE ON photos BEGIN
            INSERT INTO captions_fts(captions_fts, rowid, caption)
                SELECT 'delete', old.id, old.caption WHERE old.caption IS NOT NULL;
        END;
        CREATE TRIGGER photos_caption_au AFTER UPDATE OF caption ON photos BEGIN
            INSERT INTO captions_fts(captions_fts, rowid, caption)
                SELECT 'delete', old.id, old.caption WHERE old.caption IS NOT NULL;
            INSERT INTO captions_fts(rowid, caption)
                SELECT new.id, new.caption WHERE new.caption IS NOT NULL;
        END;
        "#,
    // 7 — cull quality: a focus/sharpness score per photo (variance of the
    // Laplacian, see `quality.rs`). Higher is sharper; a *relative* metric for
    // ranking within a burst and flagging soft frames, not an absolute unit.
    // Nullable, so a freshly-scanned photo from before this migration is simply
    // "not scored yet" until the backfill job runs.
    r#"
        ALTER TABLE photos ADD COLUMN sharpness REAL;
        "#,
    // 8 — faces (cull phase 2a). One row per detected face with its box, focus
    // and eye-open score (see `faces.rs`); the box/landmarks are also what a
    // later people-grouping phase re-uses, so detection is stored, not just
    // aggregated. `photos.faces_at` records when a photo was analysed — NULL
    // means "not analysed yet" (the backfill queue), distinct from "analysed,
    // no faces" (set, with face_count 0). The three cached aggregates let the
    // cull UI read a photo's face signals without a join.
    r#"
        CREATE TABLE IF NOT EXISTS faces (
            id       INTEGER PRIMARY KEY,
            photo_id INTEGER NOT NULL,
            x        REAL NOT NULL,
            y        REAL NOT NULL,
            w        REAL NOT NULL,
            h        REAL NOT NULL,
            sharpness REAL,
            eye_open REAL
        );
        CREATE INDEX IF NOT EXISTS idx_faces_photo ON faces(photo_id);

        ALTER TABLE photos ADD COLUMN faces_at       TEXT;
        ALTER TABLE photos ADD COLUMN face_count     INTEGER;
        ALTER TABLE photos ADD COLUMN face_sharpness REAL;
        ALTER TABLE photos ADD COLUMN eyes_open      REAL;
        "#,
    // 9: people grouping (phase 2b). Each face gets an identity embedding
    // (FaceX, raw f32 blob) and, once grouped, a `person_id`; `people` is the
    // clustered identities the People workspace lists. Grouping rebuilds these
    // from the embeddings, so a person's user-given `name` is re-matched by
    // membership on regroup rather than stored on the face.
    r#"
        ALTER TABLE faces ADD COLUMN embedding BLOB;
        ALTER TABLE faces ADD COLUMN person_id INTEGER;
        CREATE INDEX IF NOT EXISTS idx_faces_person ON faces(person_id);

        CREATE TABLE IF NOT EXISTS people (
            id            INTEGER PRIMARY KEY,
            name          TEXT,
            cover_face_id INTEGER,
            size          INTEGER NOT NULL DEFAULT 0
        );
        "#,
    // 10 — trustworthy people grouping, and correctable quality flags.
    //
    // `faces.score` is the detector's confidence. It was computed all along and
    // thrown away on write; grouping needs it, because a low-confidence
    // detection produces an embedding that lands between real identities and
    // chains them together (see `people::is_groupable`).
    //
    // `face_links` records the user's own "these two are the same person"
    // decisions. Between *faces*, not people: `regroup_people` deletes and
    // rebuilds the `people` table on every run, so a person id is not a durable
    // thing to record a decision about. `detached` pins a face the user has
    // pulled out of a person, so a regroup does not simply reabsorb it.
    //
    // `photos.eyes_override` lets a person overrule the eye-open model:
    // NULL means no opinion, 0 "these eyes are not closed" (the downcast gaze
    // it keeps mistaking for a blink), 1 "yes, closed". The cull ranks on this
    // signal, so an unfixable false positive was demoting good frames with no
    // way to say otherwise.
    r#"
        ALTER TABLE faces ADD COLUMN score REAL;
        ALTER TABLE faces ADD COLUMN detached INTEGER NOT NULL DEFAULT 0;

        CREATE TABLE IF NOT EXISTS face_links (
            a INTEGER NOT NULL REFERENCES faces(id) ON DELETE CASCADE,
            b INTEGER NOT NULL REFERENCES faces(id) ON DELETE CASCADE,
            PRIMARY KEY (a, b)
        );

        ALTER TABLE photos ADD COLUMN eyes_override INTEGER;
        "#,
    // 11 — virtual copies. A second take on the same raw file: its own edits,
    // rating, flag and tags, sitting beside the original in the grid.
    //
    // This is the only migration that rebuilds `photos`, and it has to: the
    // `UNIQUE` on `path` is a column constraint, which SQLite implements as an
    // auto-index that `DROP INDEX` refuses to touch. Two rows cannot share a
    // raw file until that constraint is `UNIQUE(path, variant)` instead, and
    // the only way there is the twelve-step table rebuild.
    //
    // Three things make that rebuild delicate, and all three are handled:
    //
    //  * `photo_tags`, `photo_places` and `collection_photos` all cascade from
    //    `photos(id)`. `migrate` turns foreign keys off around the whole loop
    //    for exactly this — with them on, `DROP TABLE photos` would take every
    //    tag, place and collection membership in the library with it.
    //  * The three `captions_fts` triggers are attached to `photos` and vanish
    //    with it, so they are recreated verbatim. The FTS index itself is
    //    external-content keyed on `photos.id`, and ids are copied across
    //    unchanged, so the index stays valid.
    //  * Every index on `photos` is recreated too, including the ones the
    //    combined filter depends on.
    r#"
        CREATE TABLE photos_new (
            id               INTEGER PRIMARY KEY,
            path             TEXT NOT NULL,
            filename         TEXT NOT NULL,
            file_hash        TEXT NOT NULL,
            capture_date     TEXT,
            camera           TEXT,
            lens             TEXT,
            iso              INTEGER,
            aperture         REAL,
            shutter          TEXT,
            focal_length     REAL,
            width            INTEGER NOT NULL DEFAULT 0,
            height           INTEGER NOT NULL DEFAULT 0,
            orientation      INTEGER NOT NULL DEFAULT 1,
            thumb_path       TEXT,
            preview_path     TEXT,
            rating           INTEGER NOT NULL DEFAULT 0,
            develop_settings TEXT,
            tagged_at        TEXT,
            indexed_at       TEXT NOT NULL,
            flag             INTEGER NOT NULL DEFAULT 0,
            color_label      INTEGER NOT NULL DEFAULT 0,
            embedding        BLOB,
            caption          TEXT,
            sharpness        REAL,
            faces_at         TEXT,
            face_count       INTEGER,
            face_sharpness   REAL,
            eyes_open        REAL,
            eyes_override    INTEGER,
            -- 0 is the master row the scanner owns; 1, 2, … are copies.
            variant          INTEGER NOT NULL DEFAULT 0,
            -- The master a copy was made from. Deleting the master takes its
            -- copies with it: they have no file of their own.
            variant_of       INTEGER REFERENCES photos(id) ON DELETE CASCADE,
            -- What the user called this copy, e.g. "Mono". Optional.
            variant_name     TEXT,
            UNIQUE(path, variant)
        );

        INSERT INTO photos_new (
            id, path, filename, file_hash, capture_date, camera, lens, iso, aperture,
            shutter, focal_length, width, height, orientation, thumb_path, preview_path,
            rating, develop_settings, tagged_at, indexed_at, flag, color_label, embedding,
            caption, sharpness, faces_at, face_count, face_sharpness, eyes_open, eyes_override
        )
        SELECT
            id, path, filename, file_hash, capture_date, camera, lens, iso, aperture,
            shutter, focal_length, width, height, orientation, thumb_path, preview_path,
            rating, develop_settings, tagged_at, indexed_at, flag, color_label, embedding,
            caption, sharpness, faces_at, face_count, face_sharpness, eyes_open, eyes_override
        FROM photos;

        DROP TABLE photos;
        ALTER TABLE photos_new RENAME TO photos;

        CREATE INDEX IF NOT EXISTS idx_photos_capture     ON photos(capture_date);
        CREATE INDEX IF NOT EXISTS idx_photos_tagged      ON photos(tagged_at);
        CREATE INDEX IF NOT EXISTS idx_photos_flag        ON photos(flag);
        CREATE INDEX IF NOT EXISTS idx_photos_colorlabel  ON photos(color_label);
        CREATE INDEX IF NOT EXISTS idx_photos_variant_of  ON photos(variant_of);

        CREATE TRIGGER photos_caption_ai AFTER INSERT ON photos BEGIN
            INSERT INTO captions_fts(rowid, caption)
                SELECT new.id, new.caption WHERE new.caption IS NOT NULL;
        END;
        CREATE TRIGGER photos_caption_ad AFTER DELETE ON photos BEGIN
            INSERT INTO captions_fts(captions_fts, rowid, caption)
                SELECT 'delete', old.id, old.caption WHERE old.caption IS NOT NULL;
        END;
        CREATE TRIGGER photos_caption_au AFTER UPDATE OF caption ON photos BEGIN
            INSERT INTO captions_fts(captions_fts, rowid, caption)
                SELECT 'delete', old.id, old.caption WHERE old.caption IS NOT NULL;
            INSERT INTO captions_fts(rowid, caption)
                SELECT new.id, new.caption WHERE new.caption IS NOT NULL;
        END;
        "#,
    // 12 — the size of the image a face box was measured in.
    //
    // Face detection runs on the 1600px preview, so `faces.x/y/w/h` are preview
    // pixels — while `photos.width/height` are the full capture (6000×4000 on
    // this camera). Anything that tried to draw a box on a thumbnail was
    // therefore out by the ratio between them, which put every crop in the
    // top-left corner of the frame.
    //
    // Storing the dimensions the detector actually saw lets the box be handed
    // to the UI as a fraction of the image, which is the only form that is
    // correct against a thumbnail, a preview and a full-size render alike.
    // NULL on rows analysed before this, which the UI treats as "cannot crop"
    // rather than cropping wrongly.
    r#"
        ALTER TABLE faces ADD COLUMN img_w REAL;
        ALTER TABLE faces ADD COLUMN img_h REAL;
        "#,
    // 13 — let the tag filter resolve from the tag, not from every photo.
    //
    // A filter on a genre/subject/condition is the most ordinary click in the
    // app, and it was the one case with no index the planner could start from:
    // `tags` has UNIQUE(name, category), so finding "which tag is 'landscape' in
    // the genre category" had to scan. This lets the membership subquery
    // (`build_where`) begin at the small side — resolve the tag, gather its
    // photos through idx_phototags_tag — instead of scanning `photos` and testing
    // each row.
    r#"
        CREATE INDEX IF NOT EXISTS idx_tags_category_name ON tags(category, name);
        "#,
    // 14 — an index the grid's sort can actually be answered from.
    //
    // The grid orders by `capture_date DESC, COALESCE(variant_of, id) DESC,
    // variant ASC` — a virtual copy sorting next to its master. `idx_photos_capture`
    // covers only the first term, so the planner filled a temp b-tree for the
    // other two before it could return the first row: asking for 200 photos cost
    // almost what asking for 20,000 did, because both sorted the whole library
    // first. SQLite indexes expressions, so this covers all three terms
    // (`id` is the rowid, legal in an index expression) and the sort is read
    // straight from the index — a `LIMIT` now stops after that many rows.
    r#"
        CREATE INDEX IF NOT EXISTS idx_photos_grid
            ON photos(capture_date DESC, COALESCE(variant_of, id) DESC, variant ASC);
        "#,
    // 15 — let a tag filter read its photos from the index alone.
    //
    // Every tag filter asks "which photos carry this tag", and
    // `idx_phototags_tag` answered only half of it: it holds `tag_id` and the
    // rowid, so each matching link cost a second seek into `photo_tags` just to
    // read `photo_id`. Carrying `photo_id` in the index makes that lookup
    // covering. It replaces the old index rather than joining it — any query
    // that could use `(tag_id)` can use `(tag_id, photo_id)` as a prefix. At
    // 50,000 photos this roughly halved the cost of evaluating a tag filter, which
    // the filter rail pays once per facet. About 5.5 MB there.
    r#"
        CREATE INDEX IF NOT EXISTS idx_phototags_tag_photo ON photo_tags(tag_id, photo_id);
        DROP INDEX IF EXISTS idx_phototags_tag;
        "#,
    // 16 — remember which photos' files have gone missing.
    //
    // A photo is identified by its path, and a folder renamed or moved outside
    // the app used to leave every row behind as a ghost — thumbnail still
    // showing, file gone — while the files came back as new photos with none
    // of their work. The scanner now relinks a moved file to its old row, and
    // marks the rows it could not find, so they read as "missing" (a drive
    // not plugged in, say) rather than failing when opened. Missing rows are
    // never deleted: that would cost the user their work over a loose cable.
    r#"
        ALTER TABLE photos ADD COLUMN missing INTEGER NOT NULL DEFAULT 0;
        "#,
    // 17 — a library of several folders.
    //
    // The library used to be one `library_root` setting, and choosing another
    // folder replaced it while keeping the old folder's photos — which then
    // were never rescanned. Folders are now rows, each photo remembers which
    // one it came from (so the filter rail can count by folder as cheaply as
    // by flag), and the old setting becomes the first folder. The separator
    // test stops `C:\Photos` from claiming `C:\Photos Old`.
    r#"
        CREATE TABLE IF NOT EXISTS library_folders (
            id       INTEGER PRIMARY KEY,
            path     TEXT NOT NULL UNIQUE,
            added_at TEXT NOT NULL DEFAULT (datetime('now'))
        );
        ALTER TABLE photos ADD COLUMN folder_id INTEGER
            REFERENCES library_folders(id) ON DELETE SET NULL;
        CREATE INDEX IF NOT EXISTS idx_photos_folder ON photos(folder_id);

        INSERT OR IGNORE INTO library_folders (path)
            SELECT value FROM settings WHERE key = 'library_root' AND value <> '';
        UPDATE photos SET folder_id = (
            SELECT f.id FROM library_folders f
             WHERE substr(photos.path, 1, length(f.path)) = f.path
               AND substr(photos.path, length(f.path) + 1, 1) IN ('\', '/'));
        "#,
    // 18 — where each photo was taken.
    //
    // `lat`/`lon` from EXIF or LibRaw. `gps_read` records that a photo's file
    // has been checked, so the backfill over a library catalogued before this
    // reads each file once, not on every launch. `place_auto` records that
    // automatic placing has considered the photo, so a place the user later
    // takes off is not put straight back.
    r#"
        ALTER TABLE photos ADD COLUMN lat REAL;
        ALTER TABLE photos ADD COLUMN lon REAL;
        ALTER TABLE photos ADD COLUMN gps_read INTEGER NOT NULL DEFAULT 0;
        ALTER TABLE photos ADD COLUMN place_auto INTEGER NOT NULL DEFAULT 0;
        "#,
    // 19 — people grouping, rebuilt (2026-09-24).
    //
    // `pose` is how far each face turns from frontal (people::pose_offset), so
    // profiles can be kept from founding a person. `people_apart` records the
    // user's "not the same person" answers, between faces, as `face_links`
    // records "the same". Photos whose faces were embedded by FaceX go back in
    // the face queue: its 256-d vectors cannot be compared with AuraFace's
    // 512-d ones. Re-analysis carries each face's person, name, links and
    // "not the same" answers over by box overlap, so nothing the user decided
    // is lost.
    r#"
        ALTER TABLE faces ADD COLUMN pose REAL;
        CREATE TABLE IF NOT EXISTS people_apart (
            a INTEGER NOT NULL,
            b INTEGER NOT NULL,
            PRIMARY KEY (a, b)
        );
        UPDATE photos SET faces_at = NULL
         WHERE id IN (SELECT photo_id FROM faces
                       WHERE embedding IS NOT NULL AND length(embedding) != 2048);
        "#,
    // 20 — search embeddings from SigLIP 2.
    //
    // CLIP's 512-d vectors cannot be compared with SigLIP 2's 768-d ones, so
    // they are cleared and the photos re-index in the background. Readers
    // already skip a vector of the wrong length; clearing is what puts the
    // photos back in the indexing queue.
    r#"
        UPDATE photos SET embedding = NULL
         WHERE embedding IS NOT NULL AND length(embedding) != 3072;
        "#,
    // 21 — people the user put in a photo by hand, and people they hid.
    //
    // `faces.manual` marks a row the user added ("Ada is in these photos")
    // rather than one the detector found: no box, no embedding, never
    // replaced by re-analysis. It holds its person through `face_links` to
    // faces of that person, since person ids do not survive a regroup.
    //
    // `people.hidden` is the user's "not someone I care about". Like a name it
    // lives on the person and is carried over by membership on regroup.
    r#"
        ALTER TABLE faces ADD COLUMN manual INTEGER NOT NULL DEFAULT 0;
        ALTER TABLE people ADD COLUMN hidden INTEGER NOT NULL DEFAULT 0;
        "#,
    // 22 — how bright each frame is, so a suggested cull can set aside
    // the frames that failed outright: a lens cap, a flash that didn't fire.
    // Measured by the focus pass from the same preview; photos scored before
    // this pick it up from that pass's backfill, which now also queues a photo
    // whose brightness is missing.
    r#"
        ALTER TABLE photos ADD COLUMN brightness REAL;
        "#,
    // 23 — the People filter's counts read "which photos is each person
    // in" straight off an index. `idx_faces_person` alone made every count fetch
    // each face row for its photo and sort them for the distinct count: ~20 ms
    // per query at 7,000 faces, on every click in the filter rail.
    r#"
        CREATE INDEX IF NOT EXISTS idx_faces_person_photo ON faces(person_id, photo_id);
        "#,
    // 24 — shoots. A card import remembered: what it was called, where
    // from and to, and the files it copied — by path, since the photos are
    // catalogued by the scan that follows, and a path outlives a rescan.
    // `exports` records each photo written out, the last step of a shoot,
    // which nothing recorded before.
    r#"
        CREATE TABLE IF NOT EXISTS shoots (
            id          INTEGER PRIMARY KEY,
            name        TEXT NOT NULL,
            source      TEXT NOT NULL,
            destination TEXT NOT NULL,
            created_at  TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS shoot_files (
            shoot_id INTEGER NOT NULL REFERENCES shoots(id) ON DELETE CASCADE,
            path     TEXT NOT NULL,
            PRIMARY KEY (shoot_id, path)
        );
        CREATE INDEX IF NOT EXISTS idx_shoot_files_path ON shoot_files(path);
        CREATE TABLE IF NOT EXISTS exports (
            photo_id    INTEGER NOT NULL REFERENCES photos(id) ON DELETE CASCADE,
            exported_at TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_exports_photo ON exports(photo_id);
        "#,
    // 25 — people's chosen faces, and faces that are not faces. `cover_pick` is the face the user chose for a
    // person's tile: the people table is rebuilt by every regroup, so the
    // choice has to live on the face to survive one. `dismissed` is "not a
    // face" (a rock, a poster), said from the unplaced-faces queue.
    r#"
        ALTER TABLE faces ADD COLUMN cover_pick INTEGER NOT NULL DEFAULT 0;
        ALTER TABLE faces ADD COLUMN dismissed  INTEGER NOT NULL DEFAULT 0;
        "#,
    // 26 — eyes read with MediaPipe's face mesh and blendshapes. OCEC's
    // readings are dropped rather than kept until re-read: on real candid
    // photos it read most open eyes as shut, so "can't tell" is the honest
    // state in between. `eyes_model` says which faces the new reading has
    // reached; the face pass fills in the rest from the stored boxes.
    r#"
        ALTER TABLE faces ADD COLUMN eyes_model INTEGER NOT NULL DEFAULT 0;
        UPDATE faces SET eye_open = NULL;
        UPDATE photos SET eyes_open = NULL;
        "#,
    // 27 — the file's size and modified time (whole seconds) as the scan last
    // saw them, so a rescan knows an unchanged file by a `stat` alone. Every
    // launch rescans, and it used to open each photo and hash its first 64 KB
    // to learn what the catalogue already knew: a seek per photo on a spinning
    // drive. NULL until a scan has seen the file since.
    r#"
        ALTER TABLE photos ADD COLUMN file_size  INTEGER;
        ALTER TABLE photos ADD COLUMN file_mtime INTEGER;
        "#,
    // 28 — whether the photo's previews were made in the colour space the
    // camera recorded. A camera set to Adobe RGB says so only in its EXIF or
    // maker notes, and previews made before that was read show those photos
    // flat. Scans set it; the backfill checks the rest once (`read_colours`).
    r#"
        ALTER TABLE photos ADD COLUMN colours_read INTEGER NOT NULL DEFAULT 0;
        "#,
    // 29 — the keyword tree. A subject keyword can sit under another:
    // `parent_id` names it, NULL at the top. Names stay unique within their
    // category, so the tagger's "dog" is the user's "dog" wherever they have
    // filed it, and a filter or saved view that names a keyword still means
    // one keyword. `kept` marks one the user made or placed, which the tree
    // shows even before any photo has it; a word of the tagger's that no photo
    // has any more just drops out.
    r#"
        ALTER TABLE tags ADD COLUMN parent_id INTEGER REFERENCES tags(id) ON DELETE SET NULL;
        ALTER TABLE tags ADD COLUMN kept INTEGER NOT NULL DEFAULT 0;
        CREATE INDEX IF NOT EXISTS idx_tags_parent ON tags(parent_id);
        "#,
    // 30 — the tagger's keywords a person turned down. Taking one of
    // its tags off a photo used to delete the link, and the next tagging run
    // put it straight back. A row here keeps it off that photo for good;
    // adding the tag by hand lifts it.
    r#"
        CREATE TABLE IF NOT EXISTS rejected_tags (
            photo_id INTEGER NOT NULL REFERENCES photos(id) ON DELETE CASCADE,
            tag_id   INTEGER NOT NULL REFERENCES tags(id) ON DELETE CASCADE,
            PRIMARY KEY (photo_id, tag_id)
        );
        "#,
    // 31 — capture times the user corrected: a camera clock on the wrong
    // time zone, or a scan with none. `capture_edited` says the time is the
    // user's, so a rescan keeps it; `original_capture_date` is the file's own,
    // to go back to — NULL when the file has none, which is why the flag is a
    // column of its own.
    r#"
        ALTER TABLE photos ADD COLUMN capture_edited INTEGER NOT NULL DEFAULT 0;
        ALTER TABLE photos ADD COLUMN original_capture_date TEXT;
        "#,
    // 32 — every column the rail's camera and kind facets read,
    // in one index, so counting them never touches a photo row. Each row
    // carries its ~3 KB search embedding, and a scan that reads rows read all
    // of them: at 50,000 photos the Camera facet took 200 ms and the new file
    // type, shape and face counts 280 ms more. Read from this index, about 50–75
    // and 60–70 ms, together 115–130 ms (a seeded 50k catalogue, two runs). The
    // status toggles still read rows — the edit stack is a blob — as before.
    r#"
        CREATE INDEX IF NOT EXISTS idx_photos_facets ON photos(
            camera, lens, focal_length, iso, aperture,
            width, height, orientation, face_count, variant_of, path
        );
        "#,
    // 33 — which GPX track placed a photo, and at what offset, so a
    // later change to its capture time can move it along the same track. Null
    // for a position from the camera, or none: those are never moved.
    r#"
        ALTER TABLE photos ADD COLUMN gps_track TEXT;
        ALTER TABLE photos ADD COLUMN gps_offset INTEGER;
        "#,
    // 34 — a photo's own title, caption and rights, and templates of them.
    // Apart from `photos`, whose rows carry a search embedding each,
    // and apart from `photos.caption`, which is the vision model's and is
    // rewritten by a re-run: a caption someone typed is theirs and wins. The
    // title and that caption are searched too, through their own index.
    r#"
        CREATE TABLE IF NOT EXISTS photo_iptc (
            photo_id    INTEGER PRIMARY KEY REFERENCES photos(id) ON DELETE CASCADE,
            title       TEXT,
            caption     TEXT,
            creator     TEXT,
            copyright   TEXT,
            credit      TEXT,
            usage_terms TEXT
        );
        CREATE TABLE IF NOT EXISTS iptc_templates (
            id          INTEGER PRIMARY KEY,
            name        TEXT NOT NULL UNIQUE COLLATE NOCASE,
            title       TEXT,
            caption     TEXT,
            creator     TEXT,
            copyright   TEXT,
            credit      TEXT,
            usage_terms TEXT
        );
        CREATE VIRTUAL TABLE IF NOT EXISTS iptc_fts USING fts5(
            title,
            caption,
            content='photo_iptc',
            content_rowid='photo_id',
            tokenize='unicode61 remove_diacritics 2'
        );
        CREATE TRIGGER IF NOT EXISTS photo_iptc_ai AFTER INSERT ON photo_iptc BEGIN
            INSERT INTO iptc_fts(rowid, title, caption) VALUES (new.photo_id, new.title, new.caption);
        END;
        CREATE TRIGGER IF NOT EXISTS photo_iptc_ad AFTER DELETE ON photo_iptc BEGIN
            INSERT INTO iptc_fts(iptc_fts, rowid, title, caption)
                VALUES ('delete', old.photo_id, old.title, old.caption);
        END;
        CREATE TRIGGER IF NOT EXISTS photo_iptc_au AFTER UPDATE OF title, caption ON photo_iptc BEGIN
            INSERT INTO iptc_fts(iptc_fts, rowid, title, caption)
                VALUES ('delete', old.photo_id, old.title, old.caption);
            INSERT INTO iptc_fts(rowid, title, caption) VALUES (new.photo_id, new.title, new.caption);
        END;
        "#,
    // 35 — collections in groups, and in an order of their own. A group
    // holds collections and saved views, one level deep; deleting it moves
    // them up a level. `position` is a photo's place in its collection, as
    // arranged by hand; existing collections start in the order photos were
    // added to them.
    r#"
        CREATE TABLE IF NOT EXISTS collection_groups (
            id         INTEGER PRIMARY KEY,
            name       TEXT NOT NULL UNIQUE COLLATE NOCASE,
            created_at TEXT NOT NULL
        );
        ALTER TABLE collections ADD COLUMN group_id INTEGER
            REFERENCES collection_groups(id) ON DELETE SET NULL;
        ALTER TABLE smart_collections ADD COLUMN group_id INTEGER
            REFERENCES collection_groups(id) ON DELETE SET NULL;
        ALTER TABLE collection_photos ADD COLUMN position INTEGER;
        UPDATE collection_photos SET position = (
            SELECT COUNT(*) FROM collection_photos o
             WHERE o.collection_id = collection_photos.collection_id
               AND (o.added_at < collection_photos.added_at
                    OR (o.added_at = collection_photos.added_at AND o.photo_id <= collection_photos.photo_id))
        );
        CREATE INDEX IF NOT EXISTS idx_collection_photos_position
            ON collection_photos(collection_id, position);
        "#,
    // 36 — each photo's colours: a bit for each named colour that
    // covers it, for the Palette filter, and a signature of all of them, for
    // finding photos with similar colours (see `palette`). Read from the
    // thumbnail, or from a small render once the photo has been edited;
    // editing clears it, to be read again.
    r#"
        CREATE TABLE IF NOT EXISTS photo_palette (
            photo_id  INTEGER PRIMARY KEY REFERENCES photos(id) ON DELETE CASCADE,
            swatches  INTEGER NOT NULL,
            signature BLOB NOT NULL
        );
        "#,
    // 37 — tags named in the interface's other languages (docs/I18N.md). A tag
    // is stored, filtered on and written into files in English; a vision model
    // writes some freely, which no fixed table can name, so the local model
    // names each once per language and the answer is kept here. Keyed by the
    // tag's text rather than its row, since the same word is one name however
    // many categories hold it.
    r#"
        CREATE TABLE IF NOT EXISTS tag_translations (
            name        TEXT NOT NULL,
            language    TEXT NOT NULL,
            translation TEXT NOT NULL,
            PRIMARY KEY (name, language)
        );
        "#,
    // 38 — what a card import could not do, kept on its shoot: how many
    // photos are not in the library, how many are in it but not yet on the
    // backup, and the backup folder, so the backup can be finished later from
    // the library copies.
    r#"
        ALTER TABLE shoots ADD COLUMN backup TEXT;
        ALTER TABLE shoots ADD COLUMN not_copied INTEGER NOT NULL DEFAULT 0;
        ALTER TABLE shoots ADD COLUMN not_backed_up INTEGER NOT NULL DEFAULT 0;
        "#,
    // 39 — files a scan could not read, so the next one doesn't try again and
    // fail the same way every launch. `attempts` is counted *before* a
    // file is read, so a file being read when the app went down is caught too:
    // the scan cannot record what killed it. See `scan::scan_library`.
    r#"
        CREATE TABLE IF NOT EXISTS scan_attempts (
            path     TEXT PRIMARY KEY,
            size     INTEGER NOT NULL,
            mtime    INTEGER NOT NULL,
            attempts INTEGER NOT NULL DEFAULT 0,
            reason   TEXT
        );
        "#,
    // 40 — a new file in a scan is looked up by its quick hash, to relink a
    // moved photo. With no index that was a scan of every photo per
    // new file: a 2,000-frame import into a 50k library spent minutes on it.
    // Partial, as the lookup is, for originals only.
    r#"
        CREATE INDEX IF NOT EXISTS idx_photos_hash ON photos(file_hash) WHERE variant = 0;
        "#,
    // 41 — the "same person" links and "not the same" answers by their second
    // face. Every face deleted cascades through `face_links.b`, and
    // unlinking a face asks `a = ? OR b = ?`: both scanned the whole table,
    // which a migrated library's named faces grow by about ten rows each.
    r#"
        CREATE INDEX IF NOT EXISTS idx_face_links_b ON face_links(b);
        CREATE INDEX IF NOT EXISTS idx_people_apart_b ON people_apart(b);
        "#,
    // 42 — which flags were accepted suggestions rather than the person's own.
    // Remembered only in the undo stack, they could not be told apart
    // after a restart. Any later change of the flag, by any path, makes it the
    // person's again: the trigger, so no writer can forget.
    r#"
        ALTER TABLE photos ADD COLUMN flag_suggested INTEGER NOT NULL DEFAULT 0;
        CREATE TRIGGER IF NOT EXISTS photos_flag_by_hand
            AFTER UPDATE OF flag ON photos
            WHEN NEW.flag IS NOT OLD.flag
        BEGIN
            UPDATE photos SET flag_suggested = 0 WHERE id = NEW.id;
        END;
        "#,
    // 43 — the rail's counts off indexes again. The rating facet and
    // the Missing count came after the facets were made index-only, and read
    // every photo row, each with its ~3 KB embedding: ~0.2 s at 50,000 photos
    // on every unfiltered reload. The folder counts read two columns from the
    // row per photo, twice per folder, before the first grid load; their
    // index takes over from idx_photos_folder, which it begins with.
    //
    // And two indexes that only cost writes: idx_phototags_photo repeats the
    // first column of photo_tags' own primary key, and idx_faces_person the
    // first of idx_faces_person_photo. Every tag link and face write kept them.
    r#"
        CREATE INDEX IF NOT EXISTS idx_photos_rating ON photos(rating);
        CREATE INDEX IF NOT EXISTS idx_photos_missing ON photos(id) WHERE missing = 1;
        CREATE INDEX IF NOT EXISTS idx_photos_folder_counts ON photos(folder_id, variant, missing);
        DROP INDEX IF EXISTS idx_photos_folder;
        DROP INDEX IF EXISTS idx_phototags_photo;
        DROP INDEX IF EXISTS idx_faces_person;
        "#,
    // 44 — the columns a literal search tests, so it reads them from an
    // index entry rather than from photo rows that each carry a ~3 KB
    // embedding: typed text in the search box cost a full read of the
    // catalogue in every one of the rail's statements.
    r#"
        CREATE INDEX IF NOT EXISTS idx_photos_text ON photos(filename, caption, variant_of);
        "#,
    // 45 — the search embedding off the photo row. Each row carried
    // its ~3 KB vector inline, so a photo filled a 4 KB page by itself and
    // every query not served by a covering index read a page per photo: the
    // multiplier behind the filter rail's slow scans. In a table of their own the
    // vectors are read only by what needs them, and a photo row is a few
    // hundred bytes. DROP COLUMN rewrites the table without it.
    r#"
        CREATE TABLE IF NOT EXISTS photo_embeddings (
            photo_id  INTEGER PRIMARY KEY REFERENCES photos(id) ON DELETE CASCADE,
            embedding BLOB NOT NULL
        );
        INSERT OR IGNORE INTO photo_embeddings (photo_id, embedding)
            SELECT id, embedding FROM photos WHERE embedding IS NOT NULL;
        ALTER TABLE photos DROP COLUMN embedding;
        "#,
    // 46 — and faces' identity embeddings off the face row, for the
    // same reason: ~2 KB inline made each face most of a page, and every
    // count or filter that read faces read them all.
    r#"
        CREATE TABLE IF NOT EXISTS face_embeddings (
            face_id   INTEGER PRIMARY KEY REFERENCES faces(id) ON DELETE CASCADE,
            embedding BLOB NOT NULL
        );
        INSERT OR IGNORE INTO face_embeddings (face_id, embedding)
            SELECT id, embedding FROM faces WHERE embedding IS NOT NULL;
        ALTER TABLE faces DROP COLUMN embedding;
        "#,
    // 47 — the photos waiting for a town from their position. Every
    // scan asked for them, and marked them done, by reading every photo row
    // under the write lock: at launch, once per library folder. Indexed, an
    // unchanged library has none to read.
    r#"
        CREATE INDEX IF NOT EXISTS idx_photos_unplaced ON photos(id)
            WHERE lat IS NOT NULL AND place_auto = 0;
        "#,
    // 48 — the phones paired with this catalogue (docs/MOBILE.md), by the
    // phone's public key: the key the link checks on every request.
    r#"
        CREATE TABLE IF NOT EXISTS paired_phones (
            id        TEXT PRIMARY KEY,
            name      TEXT NOT NULL,
            platform  TEXT NOT NULL,
            paired_at TEXT NOT NULL DEFAULT (datetime('now')),
            last_seen TEXT
        );
        "#,
    // 49 — the change feed phones sync from (docs/MOBILE.md). Every photo row
    // carries the clock reading of its last change that a phone cares about,
    // so a phone asks for "everything after N". Triggers keep it, so no write
    // anywhere in the app has to remember to; deleted photos leave a
    // tombstone at the reading they were deleted. Existing rows are numbered
    // by id, which gives a first sync a stable order to page through.
    r#"
        ALTER TABLE photos ADD COLUMN sync_seq INTEGER NOT NULL DEFAULT 0;
        UPDATE photos SET sync_seq = id;
        CREATE TABLE IF NOT EXISTS sync_clock (
            id  INTEGER PRIMARY KEY CHECK (id = 1),
            seq INTEGER NOT NULL
        );
        INSERT OR IGNORE INTO sync_clock (id, seq)
            VALUES (1, (SELECT COALESCE(MAX(id), 0) FROM photos));
        CREATE TABLE IF NOT EXISTS sync_deleted (
            photo_id INTEGER PRIMARY KEY,
            seq      INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_photos_sync_seq ON photos(sync_seq);
        CREATE INDEX IF NOT EXISTS idx_sync_deleted_seq ON sync_deleted(seq);

        CREATE TRIGGER IF NOT EXISTS sync_photo_insert AFTER INSERT ON photos BEGIN
            UPDATE sync_clock SET seq = seq + 1 WHERE id = 1;
            UPDATE photos SET sync_seq = (SELECT seq FROM sync_clock WHERE id = 1)
                WHERE id = NEW.id;
            DELETE FROM sync_deleted WHERE photo_id = NEW.id;
        END;
        CREATE TRIGGER IF NOT EXISTS sync_photo_update
            AFTER UPDATE OF rating, flag, capture_date, filename, width, height,
                            orientation, thumb_path, preview_path, missing, variant
            ON photos BEGIN
            UPDATE sync_clock SET seq = seq + 1 WHERE id = 1;
            UPDATE photos SET sync_seq = (SELECT seq FROM sync_clock WHERE id = 1)
                WHERE id = NEW.id;
        END;
        CREATE TRIGGER IF NOT EXISTS sync_photo_delete AFTER DELETE ON photos BEGIN
            UPDATE sync_clock SET seq = seq + 1 WHERE id = 1;
            INSERT OR REPLACE INTO sync_deleted (photo_id, seq)
                VALUES (OLD.id, (SELECT seq FROM sync_clock WHERE id = 1));
        END;
        "#,
    // 50 — exports waiting for a phone (Send to phone, docs/MOBILE.md). The
    // files live in outbox/ beside the catalogue until a phone has them.
    r#"
        CREATE TABLE IF NOT EXISTS phone_outbox (
            id       INTEGER PRIMARY KEY,
            path     TEXT NOT NULL,
            name     TEXT NOT NULL,
            size     INTEGER NOT NULL,
            added_at TEXT NOT NULL DEFAULT (datetime('now'))
        );
        "#,
    // 51 — photos phones have added to the library (Add to library,
    // docs/MOBILE.md), by their bytes' SHA-256: the same file sent again,
    // from a phone that forgot it sent it, finds the photo it became.
    r#"
        CREATE TABLE IF NOT EXISTS phone_imports (
            sha256   TEXT PRIMARY KEY,
            photo_id INTEGER NOT NULL,
            added_at TEXT NOT NULL DEFAULT (datetime('now'))
        );
        "#,
    // 52 — edits reach phones (editing library photos on the phone,
    // docs/MOBILE.md section 4): a change to a photo's edits moves it up the
    // change feed. Only a real change: Develop saves after every change, and
    // a save of what was already there tells a phone nothing. Every photo
    // moves up once, so phones fetch them all again and learn where each
    // can be edited.
    r#"
        CREATE TRIGGER IF NOT EXISTS sync_photo_edits
            AFTER UPDATE OF develop_settings ON photos
            WHEN OLD.develop_settings IS NOT NEW.develop_settings
            BEGIN
            UPDATE sync_clock SET seq = seq + 1 WHERE id = 1;
            UPDATE photos SET sync_seq = (SELECT seq FROM sync_clock WHERE id = 1)
                WHERE id = NEW.id;
        END;
        UPDATE photos SET sync_seq = (SELECT seq FROM sync_clock WHERE id = 1) + id;
        UPDATE sync_clock SET seq = seq + (SELECT COALESCE(MAX(id), 0) FROM photos)
            WHERE id = 1;
        "#,
    // 53 — the photo rendered as edited, for the grid and loupe, which
    // showed the camera's preview whatever the edits. A trigger forgets it
    // the moment the edits change, whoever changes them, so a stale render
    // is never shown; the page asks for a new one then.
    r#"
        ALTER TABLE photos ADD COLUMN edited_path TEXT;
        CREATE TRIGGER IF NOT EXISTS edited_path_stale
            AFTER UPDATE OF develop_settings ON photos
            WHEN OLD.develop_settings IS NOT NEW.develop_settings
            BEGIN
            UPDATE photos SET edited_path = NULL WHERE id = NEW.id;
        END;
        "#,
];

/// Brings `conn` up to the newest schema version, running only the migrations it
/// has not seen. Safe to call on every open.
pub(super) fn migrate(conn: &Connection) -> Result<()> {
    let current: u32 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    let target = MIGRATIONS.len() as u32;

    // Refuse rather than corrupt: an older build must not run against a database
    // a newer build has already migrated forward.
    if current > target {
        bail!(
            "library database is schema v{current}, but this build only understands \
             up to v{target} — update the app"
        );
    }

    // Foreign keys off for the duration, and only for the duration.
    //
    // A migration that rebuilds a table has to drop the old one, and every
    // child table cascading from it would go with it — `DROP TABLE photos`
    // with foreign keys on would take every tag, place and collection
    // membership in the library. This is the standard SQLite table-rebuild
    // procedure, and the pragma has to be set outside a transaction, where it
    // is a no-op.
    let had_fk: bool = conn.query_row("PRAGMA foreign_keys", [], |r| r.get(0))?;
    conn.pragma_update(None, "foreign_keys", "OFF")?;

    let result = (|| -> Result<()> {
        for (i, sql) in MIGRATIONS.iter().enumerate().skip(current as usize) {
            let version = i as u32 + 1;
            // Each migration and its version bump land together, so an
            // interrupted upgrade can never leave a half-applied schema
            // claiming to be complete.
            let tx = conn.unchecked_transaction()?;
            tx.execute_batch(sql)?;
            // Catches a rebuild that left a child row pointing at nothing,
            // while there is still a transaction to roll back.
            let violations: i64 = tx
                .prepare("PRAGMA foreign_key_check")?
                .query_map([], |_| Ok(()))?
                .count() as i64;
            if violations > 0 {
                bail!("migration {version} left {violations} broken foreign-key rows");
            }
            tx.pragma_update(None, "user_version", version)?;
            tx.commit()?;
        }
        Ok(())
    })();

    if had_fk {
        conn.pragma_update(None, "foreign_keys", "ON")?;
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::testing::*;

    fn user_version(conn: &Connection) -> u32 {
        conn.query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap()
    }

    /// What every install created before this system existed looks like: the
    /// right tables, but `user_version` still sitting at 0.
    fn legacy_v0_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(MIGRATIONS[0]).unwrap();
        assert_eq!(user_version(&conn), 0);
        conn
    }

    /// A library on the previous schema, carrying the data a migration must not
    /// disturb: a photo, a tag on it, a place, a collection membership, a
    /// caption in the FTS index, and an analysed face.
    fn seeded_db_at(version: usize) -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.pragma_update(None, "foreign_keys", "ON").unwrap();
        for sql in MIGRATIONS.iter().take(version) {
            conn.execute_batch(sql).unwrap();
        }
        conn.pragma_update(None, "user_version", version as u32)
            .unwrap();

        conn.execute_batch(
            "INSERT INTO photos (id, path, filename, file_hash, indexed_at, caption, width, height)
                 VALUES (1, '/lib/a.arw', 'a.arw', 'h1', '2026-01-01', 'a red bicycle', 6000, 4000);
             INSERT INTO tags (id, name, category) VALUES (1, 'bicycle', 'subject');
             INSERT INTO photo_tags (photo_id, tag_id, source) VALUES (1, 1, 'manual');
             INSERT INTO places (id, name) VALUES (1, 'Skye');
             INSERT INTO photo_places (photo_id, place_id) VALUES (1, 1);
             INSERT INTO collections (id, name, created_at) VALUES (1, 'Portfolio', '2026-01-01');
             INSERT INTO collection_photos (collection_id, photo_id, added_at)
                 VALUES (1, 1, '2026-01-01');
             INSERT INTO faces (id, photo_id, x, y, w, h, eye_open)
                 VALUES (1, 1, 10, 10, 200, 200, 0.9);",
        )
        .unwrap();
        conn
    }

    /// Everything a migration must leave standing, checked in one place so each
    /// new migration can assert it the same way.
    fn assert_library_intact(conn: &Connection) {
        let one = |sql: &str| -> i64 { conn.query_row(sql, [], |r| r.get(0)).unwrap() };
        assert_eq!(one("SELECT COUNT(*) FROM photos"), 1, "photo");
        assert_eq!(one("SELECT COUNT(*) FROM photo_tags"), 1, "tag membership");
        assert_eq!(one("SELECT COUNT(*) FROM photo_places"), 1, "place");
        assert_eq!(
            one("SELECT COUNT(*) FROM collection_photos"),
            1,
            "collection membership"
        );
        assert_eq!(one("SELECT COUNT(*) FROM faces"), 1, "face");
        assert_eq!(
            one("SELECT COUNT(*) FROM captions_fts WHERE captions_fts MATCH 'bicycle'"),
            1,
            "caption search index"
        );
    }

    #[test]
    fn migration_10_adds_its_columns_without_disturbing_the_library() {
        let conn = seeded_db_at(9);
        migrate(&conn).unwrap();
        assert_eq!(user_version(&conn), MIGRATIONS.len() as u32);
        assert_library_intact(&conn);

        // The new columns exist and default to "no opinion".
        let score: Option<f64> = conn
            .query_row("SELECT score FROM faces WHERE id = 1", [], |r| r.get(0))
            .unwrap();
        assert!(score.is_none());
        let detached: i64 = conn
            .query_row("SELECT detached FROM faces WHERE id = 1", [], |r| r.get(0))
            .unwrap();
        assert_eq!(detached, 0);
        let eyes: Option<i64> = conn
            .query_row("SELECT eyes_override FROM photos WHERE id = 1", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert!(eyes.is_none());
    }

    /// The one migration that rebuilds `photos`, and therefore the one that
    /// could quietly take the whole library with it.
    ///
    /// Every child table cascades from `photos(id)`, so a `DROP TABLE photos`
    /// with foreign keys on deletes every tag, place and collection membership
    /// in the library — and the three `captions_fts` triggers are attached to
    /// the table and vanish with it, which would leave caption search silently
    /// stale from that day on. Both are asserted here rather than reasoned about.
    #[test]
    fn migration_11_rebuilds_photos_without_taking_the_library_with_it() {
        let conn = seeded_db_at(10);
        conn.execute(
            "UPDATE photos SET rating = 4, flag = 1, color_label = 2, capture_date = '2026-01-02'",
            [],
        )
        .unwrap();
        migrate(&conn).unwrap();
        assert_eq!(user_version(&conn), MIGRATIONS.len() as u32);
        assert_library_intact(&conn);

        // The photo's own columns survived the copy, not just its row.
        let (rating, flag, label, date): (i64, i64, i64, String) = conn
            .query_row(
                "SELECT rating, flag, color_label, capture_date FROM photos WHERE id = 1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .unwrap();
        assert_eq!(
            (rating, flag, label, date.as_str()),
            (4, 1, 2, "2026-01-02")
        );

        // Caption search still works, which means the triggers came back *and*
        // the external-content index still lines up with the new table's ids.
        conn.execute(
            "UPDATE photos SET caption = 'a green tractor' WHERE id = 1",
            [],
        )
        .unwrap();
        let hits: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM captions_fts WHERE captions_fts MATCH 'tractor'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(hits, 1, "the caption triggers must survive the rebuild");

        // And foreign keys are back on afterwards, not left off for the session.
        let fk: bool = conn
            .query_row("PRAGMA foreign_keys", [], |r| r.get(0))
            .unwrap();
        assert!(fk, "migrate must restore the pragma it turned off");
    }

    /// Migrates a **copy of a real library** and asserts nothing was lost.
    ///
    /// Skipped unless `SOURCELIGHT_TEST_DB` points at one, the same way the
    /// render tests skip without a sample raw. Seeded fixtures prove the SQL is
    /// right; only a real database proves it is right about *this* library,
    /// which matters most for the migration that rebuilds `photos` — the copy
    /// is what it is run against, never the original.
    #[test]
    fn migrating_a_real_library_preserves_every_row() {
        use crate::test_support::{fixture_path, skip, Needs};
        let Some(path) = fixture_path("SOURCELIGHT_TEST_DB") else {
            skip(
                Needs::Db,
                "set SOURCELIGHT_TEST_DB to a copy of a real catalogue",
            );
            return;
        };
        let conn = connect(&path).expect("open the copy");
        let count =
            |c: &Connection, sql: &str| -> i64 { c.query_row(sql, [], |r| r.get(0)).unwrap_or(-1) };
        // Deliberately not `COUNT(*) FROM captions_fts`: that is an
        // external-content index, so it counts the content table's rows and
        // would pass whatever happened to the index. The seeded fixtures cover
        // the triggers by inserting a caption and matching it.
        let tally = |c: &Connection| {
            (
                count(c, "SELECT COUNT(*) FROM photos"),
                count(c, "SELECT COUNT(*) FROM photo_tags"),
                count(c, "SELECT COUNT(*) FROM photo_places"),
                count(c, "SELECT COUNT(*) FROM tags"),
                count(c, "SELECT COUNT(*) FROM collection_photos"),
                count(c, "SELECT COUNT(*) FROM photos WHERE caption IS NOT NULL"),
            )
        };
        let before = tally(&conn);
        assert!(before.0 > 0, "point the variable at a library with photos");

        migrate(&conn).expect("migrate the copy");

        assert_eq!(before, tally(&conn), "the migration lost rows");
        assert_eq!(user_version(&conn), MIGRATIONS.len() as u32);
        let dangling = conn
            .prepare("PRAGMA foreign_key_check")
            .and_then(|mut s| Ok(s.query_map([], |_| Ok(()))?.count()))
            .expect("foreign_key_check");
        assert_eq!(dangling, 0, "the rebuild left dangling references");

        // The queries the app actually runs still work against the rebuilt
        // table — a schema that migrates but cannot be read is no better.
        assert!(!query_photos(&conn, &PhotoFilter::default(), 10, 0)
            .expect("query the migrated library")
            .is_empty());
        facets(&conn, &PhotoFilter::default(), None).expect("facets");
    }

    #[test]
    fn a_fresh_database_lands_on_the_newest_version() {
        let conn = open_memory().unwrap();
        assert_eq!(user_version(&conn), MIGRATIONS.len() as u32);
    }

    #[test]
    fn migrating_is_idempotent() {
        let conn = open_memory().unwrap();
        migrate(&conn).unwrap();
        migrate(&conn).unwrap();
        assert_eq!(user_version(&conn), MIGRATIONS.len() as u32);
    }

    /// The upgrade path that actually matters: an existing library must pick up
    /// the version stamp without its rows being rebuilt or dropped.
    #[test]
    fn a_legacy_database_upgrades_without_losing_data() {
        let conn = legacy_v0_db();
        conn.execute(
            "INSERT INTO photos (path, filename, file_hash, indexed_at)
             VALUES ('/a.arw', 'a.arw', 'h', '2026-01-01')",
            [],
        )
        .unwrap();

        migrate(&conn).unwrap();

        assert_eq!(user_version(&conn), MIGRATIONS.len() as u32);
        let n: i64 = conn
            .query_row("SELECT COUNT(*) FROM photos", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 1, "existing rows must survive the upgrade");
    }

    /// Migration 45: a catalogue's search vectors move to their own
    /// table, intact, and the photo rows lose the column, keeping everything
    /// that hangs off them.
    #[test]
    fn search_embeddings_move_off_the_photo_row() {
        // `open` compacts the file after this one, by its number.
        let moved = EMBEDDINGS_MOVED[0] as usize;
        assert!(MIGRATIONS[moved - 1].contains("photo_embeddings"));
        let conn = Connection::open_in_memory().unwrap();
        for sql in &MIGRATIONS[..moved - 1] {
            conn.execute_batch(sql).unwrap();
        }
        conn.pragma_update(None, "user_version", EMBEDDINGS_MOVED[0] - 1)
            .unwrap();
        let blob = crate::search::encode_embedding(&[0.25_f32; 768]);
        conn.execute(
            "INSERT INTO photos (id, path, filename, file_hash, indexed_at, embedding, rating)
             VALUES (7, '/a.arw', 'a.arw', 'h', '2026-01-01', ?1, 4),
                    (8, '/b.arw', 'b.arw', 'i', '2026-01-01', NULL, 2)",
            params![blob],
        )
        .unwrap();
        let face_blob = crate::search::encode_embedding(&[0.5_f32; 512]);
        conn.execute(
            "INSERT INTO faces (id, photo_id, x, y, w, h, embedding) VALUES (11, 7, 0, 0, 9, 9, ?1)",
            params![face_blob],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO tags (name, category) VALUES ('dog', 'subject')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO photo_tags (photo_id, tag_id, source) VALUES (7, 1, 'ai')",
            [],
        )
        .unwrap();

        migrate(&conn).unwrap();

        assert_eq!(get_embedding(&conn, 7).unwrap(), Some(blob.clone()));
        assert_eq!(get_embedding(&conn, 8).unwrap(), None);
        let columns: Vec<String> = conn
            .prepare("SELECT name FROM pragma_table_info('photos')")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert!(!columns.iter().any(|c| c == "embedding"), "{columns:?}");
        let (rating, tags): (i32, i64) = conn
            .query_row(
                "SELECT rating, (SELECT COUNT(*) FROM photo_tags WHERE photo_id = 7)
                   FROM photos WHERE id = 7",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!((rating, tags), (4, 1));
        // The face's vector made the same move (migration 46).
        let face: Vec<u8> = conn
            .query_row(
                "SELECT embedding FROM face_embeddings WHERE face_id = 11",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(face, face_blob);
        let face_columns: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('faces') WHERE name = 'embedding'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(face_columns, 0);

        // A catalogue on disk upgrades the same way through `open`, which
        // backs it up first and compacts it after.
        let dir = crate::test_support::Scratch::new("embeddings-move");
        let path = dir.path().join("catalogue.db");
        {
            let disk = Connection::open(&path).unwrap();
            for sql in &MIGRATIONS[..moved - 1] {
                disk.execute_batch(sql).unwrap();
            }
            disk.pragma_update(None, "user_version", EMBEDDINGS_MOVED[0] - 1)
                .unwrap();
            disk.execute(
                "INSERT INTO photos (id, path, filename, file_hash, indexed_at, embedding)
                 VALUES (3, '/c.arw', 'c.arw', 'j', '2026-01-01', ?1)",
                params![blob],
            )
            .unwrap();
        }
        let disk = open(&path).unwrap();
        assert_eq!(get_embedding(&disk, 3).unwrap(), Some(blob.clone()));

        // And a deleted photo takes its vector with it.
        conn.pragma_update(None, "foreign_keys", "ON").unwrap();
        conn.execute("DELETE FROM photos WHERE id = 7", []).unwrap();
        let left: i64 = conn
            .query_row("SELECT COUNT(*) FROM photo_embeddings", [], |r| r.get(0))
            .unwrap();
        assert_eq!(left, 0);
    }

    /// Migration 15 swaps the tag index for one that also carries `photo_id`, so
    /// a tag-membership lookup never has to leave the index. Checked by plan, not
    /// by name alone: the point is that the lookup is *covering*.
    #[test]
    fn a_tag_lookup_is_answered_from_the_index_alone() {
        let conn = open_memory().unwrap();
        let indexes: Vec<String> = conn
            .prepare(
                "SELECT name FROM sqlite_master WHERE type = 'index' AND tbl_name = 'photo_tags'",
            )
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert!(
            indexes.iter().any(|i| i == "idx_phototags_tag_photo"),
            "{indexes:?}"
        );
        assert!(
            !indexes.iter().any(|i| i == "idx_phototags_tag"),
            "{indexes:?}"
        );

        let plan: String = conn
            .prepare("EXPLAIN QUERY PLAN SELECT photo_id FROM photo_tags WHERE tag_id = 1")
            .unwrap()
            .query_map([], |r| r.get::<_, String>(3))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap()
            .join(" | ");
        assert!(
            plan.contains("COVERING INDEX idx_phototags_tag_photo"),
            "tag lookup is not covering: {plan}"
        );
    }

    /// Stops an older build from writing to a database a newer build has already
    /// migrated forward.
    #[test]
    fn a_newer_schema_is_refused() {
        let conn = open_memory().unwrap();
        conn.pragma_update(None, "user_version", MIGRATIONS.len() as u32 + 1)
            .unwrap();

        let err = migrate(&conn).unwrap_err().to_string();
        assert!(err.contains("update the app"), "unexpected error: {err}");
    }

    /// Unlinking or deleting a face finds its links by their second face
    /// through an index, not a scan of every link.
    #[test]
    fn links_are_found_by_their_second_face_through_an_index() {
        let conn = open_memory().unwrap();
        for (table, index) in [
            ("face_links", "idx_face_links_b"),
            ("people_apart", "idx_people_apart_b"),
        ] {
            let plan: String = conn
                .prepare(&format!(
                    "EXPLAIN QUERY PLAN SELECT a FROM {table} WHERE b = ?1"
                ))
                .unwrap()
                .query_map(params![1], |r| r.get::<_, String>(3))
                .unwrap()
                .collect::<rusqlite::Result<Vec<_>>>()
                .unwrap()
                .join(" | ");
            assert!(plan.contains(index), "{table}: {plan}");
        }
    }

    #[test]
    fn migration_29_adds_the_tree_without_disturbing_the_library() {
        let conn = seeded_db_at(28);
        migrate(&conn).unwrap();
        assert_eq!(user_version(&conn), MIGRATIONS.len() as u32);
        assert_library_intact(&conn);
        let (parent, kept): (Option<i64>, i64) = conn
            .query_row("SELECT parent_id, kept FROM tags WHERE id = 1", [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .unwrap();
        assert_eq!((parent, kept), (None, 0), "every keyword starts at the top");
        assert_eq!(
            keyword_tree(&conn).unwrap(),
            vec![KeywordNode {
                name: "bicycle".into(),
                parent: None,
                kept: false
            }]
        );
    }

    #[test]
    fn migration_30_adds_rejections_without_disturbing_the_library() {
        let conn = seeded_db_at(29);
        migrate(&conn).unwrap();
        assert_eq!(user_version(&conn), MIGRATIONS.len() as u32);
        assert_library_intact(&conn);
        let n: i64 = conn
            .query_row("SELECT COUNT(*) FROM rejected_tags", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 0);
    }

    #[test]
    fn migration_31_adds_corrections_without_disturbing_the_library() {
        let conn = seeded_db_at(30);
        migrate(&conn).unwrap();
        assert_eq!(user_version(&conn), MIGRATIONS.len() as u32);
        assert_library_intact(&conn);
        assert!(!photo_of(&conn, 1).capture_edited);
    }

    #[test]
    fn migration_32_indexes_the_facet_columns_without_disturbing_the_library() {
        let conn = seeded_db_at(31);
        migrate(&conn).unwrap();
        assert_eq!(user_version(&conn), MIGRATIONS.len() as u32);
        assert_library_intact(&conn);
        let plan: String = conn
            .query_row(
                &format!(
                    "EXPLAIN QUERY PLAN SELECT {} FROM photos p",
                    gear_dims().map(|(_, sql)| sql).join(", ")
                ),
                [],
                |r| r.get(3),
            )
            .unwrap();
        assert!(plan.contains("COVERING INDEX idx_photos_facets"), "{plan}");
    }

    #[test]
    fn migration_35_orders_existing_collections_as_they_were_filled() {
        let plain = seeded_db_at(34);
        migrate(&plain).unwrap();
        assert_library_intact(&plain);

        let conn = seeded_db_at(34);
        conn.execute_batch(
            "INSERT INTO photos (id, path, filename, file_hash, indexed_at)
                 VALUES (2, '/lib/b.arw', 'b.arw', 'h2', 'x'), (3, '/lib/c.arw', 'c.arw', 'h3', 'x');
             INSERT INTO collection_photos (collection_id, photo_id, added_at)
                 VALUES (1, 3, '2025-12-01'), (1, 2, '2026-02-01');",
        )
        .unwrap();
        migrate(&conn).unwrap();
        assert_eq!(user_version(&conn), MIGRATIONS.len() as u32);
        assert_eq!(in_order(&conn, 1), vec![3, 1, 2], "by when each was added");
    }
}
