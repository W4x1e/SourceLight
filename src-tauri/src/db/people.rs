//! People: turning People on and off, grouping faces into people, and
//! keeping the user's changes through a regroup.

use super::*;

/// The setting that says whether People is on.
const PEOPLE_ON: &str = "people_on";

/// Whether People — recognising who is in photos and grouping them — is on.
/// Face *detection* always runs, since culling needs the boxes and
/// eyes. Recognition builds a template of every face, which for a working
/// photographer is their clients' biometric data, so it waits for a yes. A
/// catalogue that already has templates was using People before anyone was
/// asked, and stays on.
pub fn people_enabled(conn: &Connection) -> Result<bool> {
    if let Some(v) = get_setting(conn, PEOPLE_ON)? {
        return Ok(v == "1");
    }
    let had: bool = conn.query_row(
        "SELECT EXISTS (SELECT 1 FROM face_embeddings)
             OR EXISTS (SELECT 1 FROM people)",
        [],
        |r| r.get(0),
    )?;
    set_setting(conn, PEOPLE_ON, if had { "1" } else { "0" })?;
    Ok(had)
}

/// Turns People on or off. Off alone keeps what was built; see
/// [`erase_face_data`] for taking it away.
pub fn set_people_enabled(conn: &Connection, on: bool) -> Result<()> {
    set_setting(conn, PEOPLE_ON, if on { "1" } else { "0" })
}

/// What [`erase_face_data`] deletes, in a catalogue or in a backup of one:
/// every face template, person, name and decision about who is who, and the
/// faces added by hand. Detected boxes stay, without their templates: culling
/// reads focus and eyes from them.
const ERASE_FACE_DATA: &str = "
    DELETE FROM face_embeddings;
    DELETE FROM face_links;
    DELETE FROM people_apart;
    DELETE FROM people;
    DELETE FROM faces WHERE manual = 1;
    UPDATE faces SET person_id = NULL, detached = 0, cover_pick = 0
     WHERE person_id IS NOT NULL OR detached != 0 OR cover_pick != 0;
";

/// Turns People off and erases what it built, so that it is really
/// gone: deleted with `secure_delete` on, which overwrites what it frees, then
/// the file rebuilt by `VACUUM` and the write-ahead log truncated, so no copy
/// lingers in free pages or old log frames. [`scrub_face_data`] does the same
/// to a backup; the face portraits on disk are the caller's.
pub fn erase_face_data(conn: &mut Connection) -> Result<()> {
    conn.pragma_update(None, "secure_delete", "ON")?;
    let tx = conn.transaction()?;
    tx.execute_batch(ERASE_FACE_DATA)?;
    set_people_enabled(&tx, false)?;
    tx.commit()?;
    conn.execute_batch("VACUUM;")?;
    conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()))?;
    conn.pragma_update(None, "secure_delete", "OFF")?;
    Ok(())
}

/// [`erase_face_data`] for a backup file: the same deletes, then `VACUUM`.
pub fn scrub_face_data(path: &Path) -> Result<()> {
    let conn = Connection::open(path)?;
    conn.pragma_update(None, "secure_delete", "ON")?;
    conn.execute_batch(ERASE_FACE_DATA)?;
    conn.execute(
        "INSERT INTO settings (key, value) VALUES (?1, '0')
         ON CONFLICT(key) DO UPDATE SET value = '0'",
        params![PEOPLE_ON],
    )?;
    conn.execute_batch("VACUUM;")?;
    Ok(())
}

/// The named, shown people in each of `ids`, for Highlights to cover.
/// Unnamed and hidden people don't count: a trip full of strangers should not
/// force a photo of each.
pub fn named_people_by_photo(
    conn: &Connection,
    ids: &[i32],
) -> Result<std::collections::HashMap<i32, Vec<i32>>> {
    let mut stmt = conn.prepare(
        "SELECT DISTINCT f.photo_id, f.person_id FROM faces f JOIN people pe ON pe.id = f.person_id
          WHERE f.photo_id = ?1 AND pe.name IS NOT NULL AND pe.hidden = 0",
    )?;
    let mut out = std::collections::HashMap::new();
    for id in ids {
        let people = stmt
            .query_map(params![id], |r| r.get::<_, i32>(1))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        if !people.is_empty() {
            out.insert(*id, people);
        }
    }
    Ok(out)
}

/// Re-cluster every embedded face into people and rewrite the `people` table and
/// each face's `person_id`. A person's user-given `name` is carried over by
/// membership — the new cluster inherits the most common old named person among
/// its faces — so naming survives a regroup after new photos are analysed.
/// Returns the number of people found.
///
/// Three things the clustering itself does not know about are applied here:
/// faces too small or too weakly detected to trust are kept out of the vote
/// entirely; faces the user has detached stay their own person; and the user's
/// own "same person" links are forced on afterwards, so a merge done by hand
/// survives every future regroup whatever the embeddings say.
pub fn regroup_people(conn: &mut Connection) -> Result<usize> {
    Ok(regroup_people_with(conn, &|| false, &mut |_, _| {})?.expect("never asked to stop"))
}

/// [`regroup_people`], stoppable while it clusters; `None` if stopped.
/// Nothing is written until clustering is done, so a stopped regroup leaves
/// the people exactly as they were. `progress` is [`crate::people::cluster_with`]'s.
pub fn regroup_people_with(
    conn: &mut Connection,
    stop: &dyn Fn() -> bool,
    progress: &mut dyn FnMut(usize, usize),
) -> Result<Option<usize>> {
    struct Row {
        id: i64,
        sharpness: f32,
        old_person: Option<i64>,
        emb: Vec<f32>,
        /// How good a cover this face makes: see `cover_quality`.
        cover: f32,
    }
    // Faces that may join a person but not found or link one: a turned head,
    // a weak detection (people::votes).
    let mut attachers: Vec<Row> = Vec::new();
    // Faces the user pulled out by hand each become their own person, because
    // that is what detaching one means. Faces rejected by the quality gate do
    // *not*: turning every unembeddable face into a one-photo "person" buried
    // the real clusters under a wall of singletons, which is a worse answer
    // than leaving them out. Those keep `person_id = NULL` — they still count
    // toward the photo's face count and its eye scores, they just have no
    // identity to claim.
    let mut detached_rows: Vec<Row> = Vec::new();
    let mut ungroupable = 0usize;
    // Every "same person" link the user has made, between faces.
    let all_links: Vec<(i64, i64)> = {
        let mut stmt = conn.prepare("SELECT a, b FROM face_links")?;
        let mapped = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
        mapped.collect::<rusqlite::Result<_>>()?
    };
    let linked: std::collections::HashSet<i64> =
        all_links.iter().flat_map(|&(a, b)| [a, b]).collect();
    // Faces that cannot vote but that the user merged into someone: they go
    // where their links say, as hand-added faces do, rather than being left
    // out (too weak to trust) or attached by an embedding the user overruled.
    let mut weak_linked: Vec<(i64, Option<i64>)> = Vec::new();
    let rows: Vec<Row> = {
        let mut stmt = conn.prepare(
            "SELECT f.id, COALESCE(f.sharpness, 0.0), f.person_id, e.embedding, f.score, f.w, f.h,
                    f.detached, f.pose
               FROM faces f JOIN face_embeddings e ON e.face_id = f.id",
        )?;
        let mapped = stmt.query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, f64>(1)? as f32,
                r.get::<_, Option<i64>>(2)?,
                r.get::<_, Vec<u8>>(3)?,
                r.get::<_, Option<f64>>(4)?,
                r.get::<_, f64>(5)?,
                r.get::<_, f64>(6)?,
                r.get::<_, i64>(7)?,
                r.get::<_, Option<f64>>(8)?,
            ))
        })?;
        let mut out = Vec::new();
        for m in mapped {
            let (id, sharpness, old_person, blob, score, w, h, detached, pose) = m?;
            let Some(emb) = crate::search::decode_embedding(&blob) else {
                continue;
            };
            let score = score.map(|s| s as f32);
            let row = Row {
                id,
                sharpness,
                old_person,
                emb,
                cover: crate::people::cover_quality(
                    score,
                    w as f32,
                    h as f32,
                    pose.map(|p| p as f32),
                ),
            };
            let trusted = crate::people::is_groupable(score, w as f32, h as f32);
            let votes = crate::people::votes(score, w as f32, h as f32, pose.map(|p| p as f32));
            // A face pulled out of its person stays alone — unless the user
            // has since said who it is, which is a link like any other.
            if detached != 0 && !linked.contains(&id) {
                detached_rows.push(row);
            } else if (!votes || detached != 0) && linked.contains(&id) {
                weak_linked.push((id, old_person));
            } else if !trusted {
                ungroupable += 1;
            } else if votes {
                out.push(row);
            } else {
                attachers.push(row);
            }
        }
        out
    };

    let old_names: std::collections::HashMap<i64, String> = {
        let mut stmt = conn.prepare("SELECT id, name FROM people WHERE name IS NOT NULL")?;
        let mapped = stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?;
        mapped
            .collect::<rusqlite::Result<Vec<_>>>()?
            .into_iter()
            .collect()
    };

    let old_hidden: std::collections::HashSet<i64> = {
        let mut stmt = conn.prepare("SELECT id FROM people WHERE hidden = 1")?;
        let mapped = stmt.query_map([], |r| r.get::<_, i64>(0))?;
        mapped.collect::<rusqlite::Result<_>>()?
    };

    // Faces the user added by hand, with their person before this run, and
    // every link touching one. They have no embedding to cluster; the links
    // say where they go (see `place_manual`).
    let manual: Vec<(i64, Option<i64>)> = {
        let mut stmt = conn.prepare("SELECT id, person_id FROM faces WHERE manual = 1")?;
        let mapped = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
        mapped.collect::<rusqlite::Result<_>>()?
    };
    // Everything placed by its links alone: hand-added faces, then the weak
    // faces the user merged. `place_manual` handles both the same way.
    let by_link: Vec<(i64, Option<i64>)> = manual.into_iter().chain(weak_linked).collect();
    let by_link_links: Vec<(i64, i64)> = {
        let ids: std::collections::HashSet<i64> = by_link.iter().map(|m| m.0).collect();
        all_links
            .iter()
            .copied()
            .filter(|(a, b)| ids.contains(a) || ids.contains(b))
            .collect()
    };

    // The user's own merges, as index pairs into `rows`. Links naming a face
    // that no longer exists (its photo was removed) are simply dropped.
    let links: Vec<(usize, usize)> = {
        let index: std::collections::HashMap<i64, usize> =
            rows.iter().enumerate().map(|(i, r)| (r.id, i)).collect();
        all_links
            .iter()
            .filter_map(|(a, b)| Some((*index.get(a)?, *index.get(b)?)))
            .collect()
    };

    let thresholds = crate::people::Thresholds::default();
    let embs: Vec<Vec<f32>> = rows.iter().map(|r| r.emb.clone()).collect();
    let Some(labels) = crate::people::cluster_with(&embs, thresholds, stop, progress) else {
        return Ok(None);
    };
    let labels = crate::people::apply_links(&labels, &links);
    let grouped_people = crate::people::count(&labels);
    let mut members: Vec<Vec<usize>> = vec![Vec::new(); grouped_people];
    for (i, &g) in labels.iter().enumerate() {
        members[g].push(i);
    }

    // Faces that could not vote join the person they agree with, if any.
    let attached = {
        let cand: Vec<Vec<f32>> = attachers.iter().map(|r| r.emb.clone()).collect();
        crate::people::attach(&embs, &labels, &cand, thresholds.merge)
    };
    let mut rows = rows;
    for (row, place) in attachers.into_iter().zip(attached) {
        match place {
            Some(g) => {
                rows.push(row);
                members[g].push(rows.len() - 1);
            }
            None => ungroupable += 1,
        }
    }

    // Detached faces become one person each, appended after the real clusters.
    // Fold them into `rows`/`members` so the write loop below stays one path.
    for row in detached_rows {
        rows.push(row);
        members.push(vec![rows.len() - 1]);
    }
    if ungroupable > 0 {
        eprintln!("people: {ungroupable} face(s) too small, weak or turned away to place");
    }

    // Each group as the faces to write — (id, person before this run) — and
    // its cover: the biggest, most face-on, clearest face in the group,
    // sharpness breaking ties. It used to be the sharpest alone, which put the
    // back of someone's head on the tile (a head is sharp).
    let mut groups: Vec<(Vec<FaceAndOldPerson>, Option<i64>)> = members
        .iter()
        .map(|group| {
            let cover = group
                .iter()
                .copied()
                .max_by(|&a, &b| {
                    rows[a]
                        .cover
                        .total_cmp(&rows[b].cover)
                        .then(rows[a].sharpness.total_cmp(&rows[b].sharpness))
                })
                .map(|i| rows[i].id);
            let faces = group
                .iter()
                .map(|&m| (rows[m].id, rows[m].old_person))
                .collect();
            (faces, cover)
        })
        .collect();

    // Hand-added and merged weak faces join the group their links reach;
    // those whose links reach no voting face make people of their own.
    let group_of: std::collections::HashMap<i64, usize> = groups
        .iter()
        .enumerate()
        .flat_map(|(g, (faces, _))| faces.iter().map(move |&(id, _)| (id, g)))
        .collect();
    let by_link_ids: Vec<i64> = by_link.iter().map(|m| m.0).collect();
    let first_own = groups.len();
    for (&(id, old), place) in
        by_link
            .iter()
            .zip(place_manual(&by_link_ids, &by_link_links, &group_of))
    {
        let g = match place {
            ManualPlace::Group(g) => g,
            ManualPlace::Own(k) => {
                if groups.len() <= first_own + k {
                    groups.push((Vec::new(), None));
                }
                first_own + k
            }
        };
        groups[g].0.push((id, old));
        // A person only the user has seen is shown by the first photo they
        // put them in; a detected cover always wins over it.
        groups[g].1.get_or_insert(id);
    }
    let n_people = groups.len();

    let tx = conn.transaction()?;
    // A cover the user chose wins over the one picked by quality, for as long
    // as its face stays in the group.
    let picked: std::collections::HashSet<i64> = {
        let mut stmt = tx.prepare("SELECT id FROM faces WHERE cover_pick = 1")?;
        let ids = stmt.query_map([], |r| r.get(0))?;
        ids.collect::<rusqlite::Result<_>>()?
    };
    for (faces, cover) in groups.iter_mut() {
        if let Some(&(id, _)) = faces.iter().find(|(id, _)| picked.contains(id)) {
            *cover = Some(id);
        }
    }

    // Each group keeps the id of the person most of its faces were in,
    // so a person's id survives a regroup — a saved search for them, the
    // sheet open on them — and only faces whose person really changed are
    // written. Clearing and rewriting every face row, 2 KB embedding and all,
    // held the catalogue's write lock for seconds in a big library.
    let old_ids: Vec<i64> = {
        let mut stmt = tx.prepare("SELECT id FROM people")?;
        let ids = stmt.query_map([], |r| r.get(0))?;
        ids.collect::<rusqlite::Result<_>>()?
    };
    let keep = keep_person_ids(&groups, &old_ids);
    let current: std::collections::HashMap<i64, i64> = {
        let mut stmt = tx.prepare("SELECT id, person_id FROM faces WHERE person_id IS NOT NULL")?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    {
        let mut ins = tx.prepare(
            "INSERT INTO people (name, cover_face_id, size, hidden) VALUES (?1, ?2, ?3, ?4)",
        )?;
        let mut set = tx.prepare(
            "UPDATE people SET name = ?2, cover_face_id = ?3, size = ?4, hidden = ?5 WHERE id = ?1",
        )?;
        let mut upd = tx.prepare("UPDATE faces SET person_id = ?2 WHERE id = ?1")?;
        let mut placed: std::collections::HashSet<i64> = std::collections::HashSet::new();
        for ((faces, cover), keep) in groups.iter().zip(&keep) {
            // Inherit the most common old named person among the members, and
            // stay hidden if most of the members that had a person were in a
            // hidden one — so a stranger's new photos stay out of the way too.
            let mut tally: std::collections::HashMap<&str, usize> =
                std::collections::HashMap::new();
            let (mut known, mut hidden) = (0usize, 0usize);
            for &(_, old) in faces {
                let Some(op) = old else { continue };
                known += 1;
                if old_hidden.contains(&op) {
                    hidden += 1;
                }
                if let Some(name) = old_names.get(&op) {
                    *tally.entry(name.as_str()).or_default() += 1;
                }
            }
            let name = tally
                .into_iter()
                .max_by_key(|(_, c)| *c)
                .map(|(n, _)| n.to_string());
            let hide = hidden * 2 > known;
            let id = match *keep {
                Some(id) => {
                    set.execute(params![id, name, cover, faces.len() as i64, hide])?;
                    id
                }
                None => {
                    ins.execute(params![name, cover, faces.len() as i64, hide])?;
                    tx.last_insert_rowid()
                }
            };
            for &(face, _) in faces {
                placed.insert(face);
                if current.get(&face) != Some(&id) {
                    upd.execute(params![face, id])?;
                }
            }
        }
        let mut clear = tx.prepare("UPDATE faces SET person_id = NULL WHERE id = ?1")?;
        for face in current.keys().filter(|f| !placed.contains(f)) {
            clear.execute(params![face])?;
        }
        let kept: std::collections::HashSet<i64> = keep.iter().flatten().copied().collect();
        let mut gone = tx.prepare("DELETE FROM people WHERE id = ?1")?;
        for id in old_ids.iter().filter(|id| !kept.contains(id)) {
            gone.execute(params![id])?;
        }
    }
    tx.commit()?;
    Ok(Some(n_people))
}

/// Which person id each new group keeps: the person most of its faces
/// were in before, if a bigger group has not kept it already; `None` for a
/// group that is someone new. Biggest groups choose first, so a person split
/// in two stays themselves in the larger half; ties go to the lower id, so
/// two runs agree.
fn keep_person_ids(
    groups: &[(Vec<FaceAndOldPerson>, Option<i64>)],
    old: &[i64],
) -> Vec<Option<i64>> {
    let exists: std::collections::HashSet<i64> = old.iter().copied().collect();
    let mut order: Vec<usize> = (0..groups.len()).collect();
    order.sort_by_key(|&g| (std::cmp::Reverse(groups[g].0.len()), g));
    let mut claimed: std::collections::HashSet<i64> = std::collections::HashSet::new();
    let mut keep = vec![None; groups.len()];
    for g in order {
        let mut tally: std::collections::HashMap<i64, usize> = std::collections::HashMap::new();
        for &(_, old) in &groups[g].0 {
            if let Some(p) = old.filter(|p| exists.contains(p) && !claimed.contains(p)) {
                *tally.entry(p).or_default() += 1;
            }
        }
        if let Some((p, _)) = tally
            .into_iter()
            .max_by(|a, b| a.1.cmp(&b.1).then(b.0.cmp(&a.0)))
        {
            claimed.insert(p);
            keep[g] = Some(p);
        }
    }
    keep
}

/// How many people there are.
pub(super) fn person_count(conn: &Connection) -> Result<usize> {
    Ok(conn.query_row("SELECT COUNT(*) FROM people", [], |r| r.get::<_, i64>(0))? as usize)
}

/// Recounts one person and picks their cover again, as a regroup
/// would: a face the user picked, else the best detected face by
/// `cover_quality` with sharpness breaking ties, else the first face added by
/// hand. A person left with no faces goes.
pub(super) fn recover_person(conn: &Connection, id: i64) -> Result<()> {
    struct Face {
        id: i64,
        manual: bool,
        picked: bool,
        quality: f32,
        sharpness: f32,
    }
    let faces: Vec<Face> = {
        let mut stmt = conn.prepare(
            "SELECT id, manual, cover_pick, score, w, h, pose, COALESCE(sharpness, 0.0)
               FROM faces WHERE person_id = ?1 ORDER BY id",
        )?;
        let rows = stmt.query_map(params![id], |r| {
            let score: Option<f64> = r.get(3)?;
            let (w, h): (f64, f64) = (r.get(4)?, r.get(5)?);
            let pose: Option<f64> = r.get(6)?;
            Ok(Face {
                id: r.get(0)?,
                manual: r.get::<_, i64>(1)? != 0,
                picked: r.get::<_, i64>(2)? != 0,
                quality: crate::people::cover_quality(
                    score.map(|s| s as f32),
                    w as f32,
                    h as f32,
                    pose.map(|p| p as f32),
                ),
                sharpness: r.get::<_, f64>(7)? as f32,
            })
        })?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    if faces.is_empty() {
        conn.execute("DELETE FROM people WHERE id = ?1", params![id])?;
        return Ok(());
    }
    let cover = faces
        .iter()
        .find(|f| f.picked)
        .or_else(|| {
            faces.iter().filter(|f| !f.manual).max_by(|a, b| {
                a.quality
                    .total_cmp(&b.quality)
                    .then(a.sharpness.total_cmp(&b.sharpness))
            })
        })
        .unwrap_or(&faces[0])
        .id;
    conn.execute(
        "UPDATE people SET size = ?2, cover_face_id = ?3 WHERE id = ?1",
        params![id, faces.len() as i64, cover],
    )?;
    Ok(())
}

/// Folds people into one in place: their faces move to the person
/// that keeps its id, the others go, and the one left is recounted and
/// re-covered. The links `merge_people` has just stored make a regroup join
/// them the same way, so this is its answer without clustering the whole
/// library again, which cost seconds of every core per click in a big one.
///
/// The name most of their faces were under wins, as in a regroup, and the
/// biggest person with it keeps their id; with no names, the biggest.
pub(super) fn fold_people(tx: &Connection, ids: &[i32]) -> Result<()> {
    let mut people: Vec<(i64, Option<String>, i64)> = Vec::new();
    {
        let mut stmt = tx.prepare("SELECT id, name, size FROM people WHERE id = ?1")?;
        for &id in ids {
            if people.iter().any(|p| p.0 == id as i64) {
                continue;
            }
            if let Some(p) = stmt
                .query_row(params![id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
                .optional()?
            {
                people.push(p);
            }
        }
    }
    if people.len() < 2 {
        return Ok(());
    }
    let name: Option<String> = {
        let mut by_name: std::collections::HashMap<&str, i64> = std::collections::HashMap::new();
        for (_, name, size) in &people {
            if let Some(n) = name {
                *by_name.entry(n.as_str()).or_default() += size;
            }
        }
        by_name
            .into_iter()
            .max_by(|a, b| a.1.cmp(&b.1).then(b.0.cmp(a.0)))
            .map(|(n, _)| n.to_string())
    };
    let keep = people
        .iter()
        .filter(|p| p.1.as_deref() == name.as_deref())
        .max_by(|a, b| a.2.cmp(&b.2).then(b.0.cmp(&a.0)))
        .map_or(people[0].0, |p| p.0);
    {
        let mut move_faces = tx.prepare("UPDATE faces SET person_id = ?2 WHERE person_id = ?1")?;
        let mut gone = tx.prepare("DELETE FROM people WHERE id = ?1")?;
        for &(id, _, _) in people.iter().filter(|p| p.0 != keep) {
            move_faces.execute(params![id, keep])?;
            gone.execute(params![id])?;
        }
    }
    tx.execute(
        "UPDATE people SET name = ?2, hidden = 0 WHERE id = ?1",
        params![keep, name],
    )?;
    recover_person(tx, keep)
}

/// Makes a face its own person, unnamed, as "Not them" does: the next
/// regroup leaves a detached face alone, so this is its answer now.
pub(super) fn own_person(tx: &Connection, face: i64) -> Result<()> {
    tx.execute(
        "INSERT INTO people (name, cover_face_id, size) VALUES (NULL, ?1, 1)",
        params![face],
    )?;
    let own = tx.last_insert_rowid();
    tx.execute(
        "UPDATE faces SET person_id = ?2 WHERE id = ?1",
        params![face, own],
    )?;
    Ok(())
}

/// Puts the faces a face pass found into the people already there,
/// without clustering the library again: that was O(faces²) after every
/// import, 10–20 s of every core at 200,000 photos, then a rewrite of every
/// face row. A new face that votes joins the person whose voting faces it
/// agrees with on average, as `people::attach` places a face in a regroup;
/// those that join nobody are clustered among themselves into new people;
/// the faces that may not vote then attach to either, or stay unplaced.
///
/// `None`, with nothing written, when only a regroup gives the right answer:
/// no people yet, new voting faces more than a tenth of those placed (enough
/// to change who the people already there are), or a new face the user has
/// already linked to someone. Otherwise the number of people.
/// A face row as `place_new_faces` reads it: id, person, embedding, score,
/// width, height, pose.
type FaceRow = (
    i64,
    Option<i64>,
    Vec<u8>,
    Option<f64>,
    f64,
    f64,
    Option<f64>,
);

pub fn place_new_faces(conn: &mut Connection) -> Result<Option<usize>> {
    struct Fresh {
        id: i64,
        emb: Vec<f32>,
        votes: bool,
    }
    let linked: std::collections::HashSet<i64> = {
        let mut stmt = conn.prepare("SELECT a FROM face_links")?;
        let ids = stmt.query_map([], |r| r.get(0))?;
        ids.collect::<rusqlite::Result<_>>()?
    };
    let face_row = |r: &Row| -> rusqlite::Result<FaceRow> {
        Ok((
            r.get(0)?,
            r.get(1)?,
            r.get(2)?,
            r.get(3)?,
            r.get(4)?,
            r.get(5)?,
            r.get(6)?,
        ))
    };
    let mut fresh: Vec<Fresh> = Vec::new();
    {
        let mut stmt = conn.prepare(
            "SELECT f.id, f.person_id, e.embedding, f.score, f.w, f.h, f.pose
               FROM faces f JOIN face_embeddings e ON e.face_id = f.id
              WHERE f.person_id IS NULL AND f.detached = 0 AND f.manual = 0",
        )?;
        for row in stmt.query_map([], face_row)? {
            let (id, _, blob, score, w, h, pose) = row?;
            if linked.contains(&id) {
                return Ok(None);
            }
            let Some(emb) = crate::search::decode_embedding(&blob) else {
                continue;
            };
            let score = score.map(|s| s as f32);
            if !crate::people::is_groupable(score, w as f32, h as f32) {
                continue;
            }
            let votes = crate::people::votes(score, w as f32, h as f32, pose.map(|p| p as f32));
            fresh.push(Fresh { id, emb, votes });
        }
    }
    if fresh.is_empty() {
        return Ok(Some(person_count(conn)?));
    }

    // The people there, as the sums of their voting faces.
    let mut person_ids: Vec<i64> = Vec::new();
    let mut sums: Vec<Vec<f32>> = Vec::new();
    let mut sizes: Vec<usize> = Vec::new();
    {
        let mut index: std::collections::HashMap<i64, usize> = std::collections::HashMap::new();
        let mut stmt = conn.prepare(
            "SELECT f.id, f.person_id, e.embedding, f.score, f.w, f.h, f.pose
               FROM faces f JOIN face_embeddings e ON e.face_id = f.id
              WHERE f.person_id IS NOT NULL AND f.detached = 0 AND f.manual = 0",
        )?;
        for row in stmt.query_map([], face_row)? {
            let (_, person, blob, score, w, h, pose) = row?;
            let Some(person) = person else { continue };
            if !crate::people::votes(
                score.map(|s| s as f32),
                w as f32,
                h as f32,
                pose.map(|p| p as f32),
            ) {
                continue;
            }
            let Some(emb) = crate::search::decode_embedding(&blob) else {
                continue;
            };
            let g = *index.entry(person).or_insert_with(|| {
                person_ids.push(person);
                sums.push(vec![0.0; emb.len()]);
                sizes.push(0);
                person_ids.len() - 1
            });
            if sums[g].len() != emb.len() {
                continue;
            }
            for (s, v) in sums[g].iter_mut().zip(&emb) {
                *s += v;
            }
            sizes[g] += 1;
        }
    }
    let new_voters = fresh.iter().filter(|f| f.votes).count();
    let placed_voters: usize = sizes.iter().sum();
    if person_ids.is_empty() || new_voters * 10 > placed_voters {
        return Ok(None);
    }

    let thresholds = crate::people::Thresholds::default();
    let (voters, attachers): (Vec<Fresh>, Vec<Fresh>) = fresh.into_iter().partition(|f| f.votes);
    let voter_embs: Vec<Vec<f32>> = voters.iter().map(|f| f.emb.clone()).collect();
    let joined = crate::people::attach_to_sums(&sums, &sizes, &voter_embs, thresholds.merge);
    // Those who join nobody are someone new: grouped among themselves.
    let rest: Vec<usize> = (0..voters.len()).filter(|&i| joined[i].is_none()).collect();
    let rest_embs: Vec<Vec<f32>> = rest.iter().map(|&i| voter_embs[i].clone()).collect();
    let labels = crate::people::cluster(&rest_embs, thresholds);
    let known = person_ids.len();
    let new_people = crate::people::count(&labels);

    // Where each face goes: a group of `sums`, then the new groups after them.
    let mut place: Vec<(i64, usize)> = Vec::new();
    for (f, g) in voters.iter().zip(&joined) {
        if let Some(g) = g {
            place.push((f.id, *g));
        }
    }
    for (k, &i) in rest.iter().enumerate() {
        place.push((voters[i].id, known + labels[k]));
    }
    let (new_sums, new_sizes) = crate::people::group_sums(&rest_embs, &labels);
    sums.extend(new_sums);
    sizes.extend(new_sizes);
    let attacher_embs: Vec<Vec<f32>> = attachers.iter().map(|f| f.emb.clone()).collect();
    for (f, g) in attachers.iter().zip(crate::people::attach_to_sums(
        &sums,
        &sizes,
        &attacher_embs,
        thresholds.merge,
    )) {
        if let Some(g) = g {
            place.push((f.id, g));
        }
    }

    let tx = conn.transaction()?;
    let mut new_ids: Vec<i64> = Vec::with_capacity(new_people);
    for _ in 0..new_people {
        tx.execute("INSERT INTO people (size) VALUES (0)", [])?;
        new_ids.push(tx.last_insert_rowid());
    }
    let mut touched: std::collections::BTreeSet<i64> = std::collections::BTreeSet::new();
    {
        let mut upd = tx.prepare("UPDATE faces SET person_id = ?2 WHERE id = ?1")?;
        for (face, g) in place {
            let id = if g < known {
                person_ids[g]
            } else {
                new_ids[g - known]
            };
            upd.execute(params![face, id])?;
            touched.insert(id);
        }
    }
    for id in touched {
        recover_person(&tx, id)?;
    }
    tx.commit()?;
    Ok(Some(person_count(conn)?))
}

/// Changes whenever anything people suggestions depend on may have:
/// who is in whom, names, hiding, and the "not the same" answers. Cheap — an
/// index scan — where the suggestions are not.
pub fn people_fingerprint(conn: &Connection) -> Result<String> {
    Ok(conn.query_row(
        "SELECT (SELECT COUNT(*) || ':' || COALESCE(SUM(id * person_id), 0)
                   FROM faces WHERE person_id IS NOT NULL)
             || '/' || (SELECT COUNT(*) || ':' || COALESCE(SUM(hidden * id), 0) || ':'
                            || COALESCE(SUM(length(name) * id), 0) FROM people)
             || '/' || (SELECT COUNT(*) FROM people_apart)",
        [],
        |r| r.get(0),
    )?)
}

/// A name read from a sidecar before its photo was analysed waits as a face
/// added by hand, with the region's box (see [`import_people`]). The face the
/// detector finds there takes it over — its person, its links, its place as a
/// cover — as a re-detected face takes over from the old one, and the stand-in
/// goes. `fresh` is the photo's new faces, each with its box as fractions and
/// whether it is free to: one that already has a person keeps it, and only a
/// face that votes is sure of a person after the next regroup — a weaker one
/// would lose the name there, so the stand-in stays for it.
pub(super) fn take_over_waiting_faces(
    tx: &Connection,
    photo_id: i32,
    fresh: &[(i64, [f32; 4], bool)],
) -> Result<()> {
    let waiting: Vec<(i64, Vec<f32>, Option<i64>)> = {
        let mut stmt = tx.prepare(
            "SELECT id, x, y, w, h, img_w, img_h, person_id FROM faces
              WHERE photo_id = ?1 AND manual = 1 AND w > 0",
        )?;
        let rows = stmt.query_map(params![photo_id], |r| {
            Ok((
                r.get(0)?,
                box_fraction(
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    (r.get(5)?, r.get(6)?),
                ),
                r.get(7)?,
            ))
        })?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    let mut taken: Vec<i64> = Vec::new();
    for (stand_in, frac, person) in waiting {
        let [x, y, w, h] = frac[..] else { continue };
        let Some(found) = fresh
            .iter()
            .filter(|(id, _, free)| *free && !taken.contains(id))
            .map(|(id, b, _)| (*id, crate::faces::iou(*b, [x, y, w, h])))
            .filter(|&(_, v)| v >= REGION_MATCH)
            .max_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(id, _)| id)
        else {
            continue;
        };
        taken.push(found);
        tx.execute(
            "UPDATE faces SET person_id = ?2 WHERE id = ?1",
            params![found, person],
        )?;
        tx.execute(
            "UPDATE OR IGNORE face_links SET a = ?2 WHERE a = ?1",
            params![stand_in, found],
        )?;
        tx.execute(
            "UPDATE OR IGNORE face_links SET b = ?2 WHERE b = ?1",
            params![stand_in, found],
        )?;
        tx.execute(
            "DELETE FROM face_links WHERE a = ?1 OR b = ?1",
            params![stand_in],
        )?;
        tx.execute(
            "UPDATE people SET cover_face_id = ?2 WHERE cover_face_id = ?1",
            params![stand_in, found],
        )?;
        tx.execute("DELETE FROM faces WHERE id = ?1", params![stand_in])?;
    }
    Ok(())
}

/// A face id and the person it was in before a regroup.
type FaceAndOldPerson = (i64, Option<i64>);

/// Where a hand-added face goes on a regroup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ManualPlace {
    /// Into this detected group.
    Group(usize),
    /// Into the `k`th person made only of hand-added faces.
    Own(usize),
}

/// Places faces that go by their links alone — hand-added ones, and detected
/// ones too weak to vote that the user merged into someone. Faces linked to
/// each other go together; together they join the detected group most of
/// their links reach, or, when none reach one (a person the detector never
/// found, or whose faces have since gone), become a person of their own.
fn place_manual(
    manual: &[i64],
    links: &[(i64, i64)],
    group_of: &std::collections::HashMap<i64, usize>,
) -> Vec<ManualPlace> {
    use std::collections::HashMap;
    fn root(parent: &mut [usize], mut i: usize) -> usize {
        while parent[i] != i {
            parent[i] = parent[parent[i]];
            i = parent[i];
        }
        i
    }
    let index: HashMap<i64, usize> = manual.iter().enumerate().map(|(i, &id)| (id, i)).collect();
    let mut parent: Vec<usize> = (0..manual.len()).collect();
    let mut reach: Vec<(usize, usize)> = Vec::new();
    for &(a, b) in links {
        match (index.get(&a), index.get(&b)) {
            (Some(&i), Some(&j)) => {
                let (ri, rj) = (root(&mut parent, i), root(&mut parent, j));
                parent[ri] = rj;
            }
            (Some(&i), None) => reach.extend(group_of.get(&b).map(|&g| (i, g))),
            (None, Some(&j)) => reach.extend(group_of.get(&a).map(|&g| (j, g))),
            (None, None) => {}
        }
    }
    let mut votes: HashMap<usize, HashMap<usize, usize>> = HashMap::new();
    for (i, g) in reach {
        let r = root(&mut parent, i);
        *votes.entry(r).or_default().entry(g).or_default() += 1;
    }
    let mut own: HashMap<usize, usize> = HashMap::new();
    (0..manual.len())
        .map(|i| {
            let r = root(&mut parent, i);
            // Most links wins; the lower group breaks a tie, so it is stable.
            let best = votes.get(&r).and_then(|v| {
                v.iter()
                    .max_by(|a, b| a.1.cmp(b.1).then(b.0.cmp(a.0)))
                    .map(|(&g, _)| g)
            });
            match best {
                Some(g) => ManualPlace::Group(g),
                None => {
                    let next = own.len();
                    ManualPlace::Own(*own.entry(r).or_insert(next))
                }
            }
        })
        .collect()
}

/// The clustered people, most-photographed first, each with a cover face for its
/// tile. Singletons are included (ordered last) so nothing analysed is hidden.
pub fn list_people(conn: &Connection) -> Result<Vec<crate::models::Person>> {
    let mut stmt = conn.prepare(
        "SELECT pe.id, pe.name, pe.size,
                (SELECT COALESCE(ph.thumb_path, ph.preview_path)
                   FROM faces fc JOIN photos ph ON ph.id = fc.photo_id
                  WHERE fc.id = pe.cover_face_id),
                (SELECT fc.x FROM faces fc WHERE fc.id = pe.cover_face_id),
                (SELECT fc.y FROM faces fc WHERE fc.id = pe.cover_face_id),
                (SELECT fc.w FROM faces fc WHERE fc.id = pe.cover_face_id),
                (SELECT fc.h FROM faces fc WHERE fc.id = pe.cover_face_id),
                (SELECT fc.img_w FROM faces fc WHERE fc.id = pe.cover_face_id),
                (SELECT fc.img_h FROM faces fc WHERE fc.id = pe.cover_face_id),
                pe.hidden,
                pe.cover_face_id
         FROM people pe
         ORDER BY pe.size DESC, pe.id",
    )?;
    let mapped = stmt.query_map([], |r| {
        let bx: (Option<f64>, Option<f64>, Option<f64>, Option<f64>) =
            (r.get(4)?, r.get(5)?, r.get(6)?, r.get(7)?);
        // A fraction, like `PersonFace::box_frac` — so a person's tile can crop
        // to the face instead of showing whichever square of the photo the face
        // happens to sit in.
        let cover_box = match bx {
            (Some(x), Some(y), Some(w), Some(h)) => {
                let frac = box_fraction(x, y, w, h, (r.get(8)?, r.get(9)?));
                (!frac.is_empty()).then_some(frac)
            }
            _ => None,
        };
        Ok(crate::models::Person {
            id: r.get(0)?,
            name: r.get(1)?,
            count: r.get::<_, i64>(2)? as i32,
            cover_path: r.get(3)?,
            cover_box,
            portrait_path: None,
            hidden: r.get::<_, i64>(10)? != 0,
            cover_face_id: r.get::<_, Option<i64>>(11)?.map(|v| v as i32),
        })
    })?;
    Ok(mapped.collect::<rusqlite::Result<Vec<_>>>()?)
}

/// After faces are deleted: recounts every person, drops people left with no
/// faces, and re-covers a person whose cover face went. Shared by binning
/// rejects and removing a library folder.
pub(super) fn recount_people(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "UPDATE people SET size = (SELECT COUNT(*) FROM faces f WHERE f.person_id = people.id);
         DELETE FROM people WHERE size = 0;
         UPDATE people SET cover_face_id =
             (SELECT f.id FROM faces f WHERE f.person_id = people.id ORDER BY f.manual, f.id LIMIT 1)
          WHERE cover_face_id IS NULL
             OR cover_face_id NOT IN (SELECT id FROM faces);",
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::testing::*;

    /// Cancelling a regroup must leave the people as they were — not an
    /// emptied table, which is what stopping after the `DELETE` would give.
    #[test]
    fn a_stopped_regroup_leaves_the_people_as_they_were() {
        let mut conn = open_memory().unwrap();
        // Two photos, one clear face each, of the same person.
        for id in 1..=2 {
            conn.execute(
                "INSERT INTO photos (id, path, filename, file_hash, indexed_at)
                 VALUES (?1, ?2, ?2, ?2, '2026-01-01')",
                params![id, format!("p{id}.arw")],
            )
            .unwrap();
            let emb = crate::search::encode_embedding(&[1.0, 0.0, 0.0]);
            conn.execute(
                "INSERT INTO faces (photo_id, x, y, w, h, score)
                 VALUES (?1, 0, 0, 400, 400, 0.99)",
                params![id],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO face_embeddings (face_id, embedding) VALUES (?1, ?2)",
                params![conn.last_insert_rowid(), emb],
            )
            .unwrap();
        }
        assert_eq!(regroup_people(&mut conn).unwrap(), 1);
        let person: i32 = conn
            .query_row("SELECT id FROM people", [], |r| r.get(0))
            .unwrap();
        set_person_name(&conn, person, "Ada").unwrap();

        let stopped = regroup_people_with(&mut conn, &|| true, &mut |_, _| {}).unwrap();
        assert_eq!(stopped, None);
        let people = list_people(&conn).unwrap();
        assert_eq!(people.len(), 1);
        assert_eq!(people[0].name.as_deref(), Some("Ada"));
        let assigned: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM faces WHERE person_id IS NOT NULL",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(assigned, 2);
    }

    /// Re-analysing a photo (a new embedder, a re-detect) must not lose who
    /// the user said anyone was: the new face takes over the old one's person,
    /// and "not the same" answers follow it.
    #[test]
    fn re_analysis_keeps_names_and_not_the_same_answers() {
        let mut conn = open_memory().unwrap();
        add_photos(&conn, 3);
        let ada = unit(&[1.0, 0.0, 0.0]);
        let bo = unit(&[0.0, 1.0, 0.0]);
        store_faces(
            &mut conn,
            1,
            &[face_at(100.0, 100.0, false)],
            std::slice::from_ref(&ada),
            (1600, 1067),
        )
        .unwrap();
        store_faces(
            &mut conn,
            2,
            &[face_at(300.0, 200.0, false)],
            std::slice::from_ref(&ada),
            (1600, 1067),
        )
        .unwrap();
        store_faces(
            &mut conn,
            3,
            &[face_at(500.0, 300.0, false)],
            std::slice::from_ref(&bo),
            (1600, 1067),
        )
        .unwrap();
        assert_eq!(regroup_people(&mut conn).unwrap(), 2);
        let (a, b): (i32, i32) = (
            conn.query_row("SELECT person_id FROM faces WHERE photo_id = 1", [], |r| {
                r.get(0)
            })
            .unwrap(),
            conn.query_row("SELECT person_id FROM faces WHERE photo_id = 3", [], |r| {
                r.get(0)
            })
            .unwrap(),
        );
        set_person_name(&conn, a, "Ada").unwrap();
        keep_people_apart(&conn, a, b).unwrap();

        // Photo 1 again, the box a little off and a new embedding.
        store_faces(
            &mut conn,
            1,
            &[face_at(108.0, 104.0, false)],
            &[unit(&[0.95, 0.05, 0.0])],
            (1600, 1067),
        )
        .unwrap();
        regroup_people(&mut conn).unwrap();
        assert_eq!(person_of_photo(&conn, 1).as_deref(), Some("Ada"));
        assert_eq!(person_of_photo(&conn, 2).as_deref(), Some("Ada"));
        let apart_points_at_live_faces: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM people_apart pa
                   JOIN faces f1 ON f1.id = pa.a JOIN faces f2 ON f2.id = pa.b",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(apart_points_at_live_faces, 1);
    }

    /// A cover the user chose stays the cover through a regroup, which
    /// rebuilds every person.
    #[test]
    fn a_chosen_cover_survives_a_regroup() {
        let mut conn = open_memory().unwrap();
        ada_and_bo(&mut conn, 4);
        let cover_of_ada = |conn: &Connection| {
            list_people(conn)
                .unwrap()
                .into_iter()
                .find(|p| p.name.as_deref() == Some("Ada"))
                .unwrap()
                .cover_face_id
                .unwrap()
        };
        let (one, two) = (face_on(&conn, 1), face_on(&conn, 2));
        let other = if cover_of_ada(&conn) == one { two } else { one };
        set_cover_face(&mut conn, other).unwrap();
        assert_eq!(cover_of_ada(&conn), other);
        regroup_people(&mut conn).unwrap();
        assert_eq!(cover_of_ada(&conn), other, "kept through the regroup");

        // Choosing again replaces the choice rather than adding a second.
        let first = if other == one { two } else { one };
        set_cover_face(&mut conn, first).unwrap();
        regroup_people(&mut conn).unwrap();
        assert_eq!(cover_of_ada(&conn), first);
        let picks: i64 = conn
            .query_row("SELECT COUNT(*) FROM faces WHERE cover_pick = 1", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(picks, 1);
    }

    /// A person put in a photo by hand stays in it through a regroup and a
    /// re-analysis of that photo — neither asks the detector about them.
    #[test]
    fn a_person_added_by_hand_survives_regroup_and_re_analysis() {
        let mut conn = open_memory().unwrap();
        let (a, _) = ada_and_bo(&mut conn, 5);
        // Photo 4: the back of Ada's head, nothing detected.
        store_faces(&mut conn, 4, &[], &[], (1600, 1067)).unwrap();
        assert_eq!(
            add_photos_to_person(&mut conn, &[4, 4, 1], a).unwrap(),
            1,
            "once, and not where she already is"
        );
        assert_eq!(person_of_photo(&conn, 4).as_deref(), Some("Ada"));
        assert_eq!(
            list_people(&conn)
                .unwrap()
                .iter()
                .find(|p| p.id == a)
                .unwrap()
                .count,
            3
        );

        regroup_people(&mut conn).unwrap();
        assert_eq!(person_of_photo(&conn, 4).as_deref(), Some("Ada"));
        store_faces(&mut conn, 4, &[], &[], (1600, 1067)).unwrap();
        regroup_people(&mut conn).unwrap();
        assert_eq!(person_of_photo(&conn, 4).as_deref(), Some("Ada"));
        assert_eq!(
            unembedded_count(&conn).unwrap(),
            0,
            "a hand-added face is never queued for embedding"
        );

        let found = photo_people(&conn, 4).unwrap();
        assert_eq!(found.len(), 1);
        assert!(found[0].manual);

        let a = ada(&conn);
        remove_photos_from_person(&mut conn, &[4], a).unwrap();
        assert_eq!(person_of_photo(&conn, 4), None);
        assert_eq!(
            list_people(&conn)
                .unwrap()
                .iter()
                .find(|p| p.name.as_deref() == Some("Ada"))
                .unwrap()
                .count,
            2
        );
    }

    /// Someone the detector never found can still be a person: made from
    /// photos alone, kept whole and named through a regroup.
    #[test]
    fn a_person_only_the_user_has_seen_survives_a_regroup() {
        let mut conn = open_memory().unwrap();
        ada_and_bo(&mut conn, 6);
        let cy = add_photos_to_new_person(&mut conn, &[99, 4, 5, 6], "Cy").unwrap();
        let listed = list_people(&conn).unwrap();
        let p = listed.iter().find(|p| p.id == cy).unwrap();
        assert_eq!((p.name.as_deref(), p.count), (Some("Cy"), 3));
        assert!(p.cover_path.is_some() || p.cover_box.is_none());

        assert_eq!(regroup_people(&mut conn).unwrap(), 3);
        for photo in 4..=6 {
            assert_eq!(person_of_photo(&conn, photo).as_deref(), Some("Cy"));
        }
        // Merged into Ada, the detected face anchors it and Cy's photos follow.
        let cy = conn
            .query_row("SELECT id FROM people WHERE name = 'Cy'", [], |r| r.get(0))
            .unwrap();
        let a = ada(&conn);
        merge_people(&mut conn, &[cy, a]).unwrap();
        regroup_people(&mut conn).unwrap();
        let names: Vec<Option<String>> = (1..=6)
            .filter(|&p| p != 3)
            .map(|p| person_of_photo(&conn, p))
            .collect();
        assert!(names.iter().all(|n| n == &names[0]), "{names:?}");
    }

    /// People waits for a yes, except where it was already in use.
    #[test]
    fn people_is_off_until_asked_unless_it_was_already_used() {
        let conn = open_memory().unwrap();
        assert!(!people_enabled(&conn).unwrap());
        set_people_enabled(&conn, true).unwrap();
        assert!(people_enabled(&conn).unwrap());

        let mut used = open_memory().unwrap();
        ada_and_bo(&mut used, 3);
        assert!(
            people_enabled(&used).unwrap(),
            "a library with templates was using it"
        );
    }

    /// Erasing takes every template, person, name, link and hand-added face,
    /// and turns People off; the detected boxes stay for culling.
    #[test]
    fn erasing_face_data_leaves_only_what_culling_needs() {
        let mut conn = open_memory().unwrap();
        let (ada, bo) = ada_and_bo(&mut conn, 4);
        merge_people(&mut conn, &[ada, bo]).unwrap();
        add_photos_to_person(&mut conn, &[4], ada).unwrap();
        erase_face_data(&mut conn).unwrap();
        let count = |sql: &str| -> i64 { conn.query_row(sql, [], |r| r.get(0)).unwrap() };
        assert_eq!(count("SELECT COUNT(*) FROM people"), 0);
        assert_eq!(count("SELECT COUNT(*) FROM face_links"), 0);
        assert_eq!(count("SELECT COUNT(*) FROM faces WHERE manual = 1"), 0);
        assert_eq!(
            count("SELECT (SELECT COUNT(*) FROM face_embeddings) + COUNT(person_id) FROM faces"),
            0
        );
        assert_eq!(
            count("SELECT COUNT(*) FROM faces"),
            3,
            "the detected faces stay"
        );
        assert!(!people_enabled(&conn).unwrap());
    }

    /// Re-detecting keeps what the user decided: it promised merges
    /// were kept, then deleted every face, and the cascade took the links.
    #[test]
    fn re_detecting_keeps_merges_and_faces_added_by_hand() {
        let mut conn = open_memory().unwrap();
        let (ada, bo) = ada_and_bo(&mut conn, 4);
        merge_people(&mut conn, &[ada, bo]).unwrap();
        add_photos_to_person(&mut conn, &[4], ada).unwrap();
        let links = |c: &Connection| -> i64 {
            c.query_row("SELECT COUNT(*) FROM face_links", [], |r| r.get(0))
                .unwrap()
        };
        let before = links(&conn);
        assert!(before > 0);

        assert_eq!(reset_faces(&conn).unwrap(), 3);
        // Each photo detected again, its face found in the same place.
        for (photo, e) in [
            (1, [1.0, 0.0, 0.0]),
            (2, [1.0, 0.0, 0.0]),
            (3, [0.0, 1.0, 0.0]),
        ] {
            store_faces(
                &mut conn,
                photo,
                &[face_at(100.0, 100.0, false)],
                &[unit(&e)],
                (1600, 1067),
            )
            .unwrap();
        }
        assert_eq!(
            links(&conn),
            before,
            "the merge's links follow the new faces"
        );
        regroup_people(&mut conn).unwrap();
        assert_eq!(person_of_photo(&conn, 3).as_deref(), Some("Ada"));
        assert_eq!(
            person_of_photo(&conn, 4).as_deref(),
            Some("Ada"),
            "hand-added stays"
        );
    }

    /// A regroup keeps everyone's id: a saved search for someone, or
    /// the sheet open on them, still means them afterwards.
    #[test]
    fn a_regroup_keeps_person_ids() {
        let mut conn = open_memory().unwrap();
        let (ada, bo) = ada_and_bo(&mut conn, 4);
        store_faces(
            &mut conn,
            4,
            &[face_at(100.0, 100.0, false)],
            &[unit(&[1.0, 0.0, 0.0])],
            (1600, 1067),
        )
        .unwrap();
        regroup_people(&mut conn).unwrap();
        assert_eq!(person_id_of(&conn, 1), Some(ada));
        assert_eq!(person_id_of(&conn, 4), Some(ada));
        assert_eq!(person_id_of(&conn, 3), Some(bo));
        assert_eq!(person_size(&conn, ada), 3);
        assert_eq!(person_of_photo(&conn, 4).as_deref(), Some("Ada"));
    }

    /// After a face pass, new faces join the people already there and
    /// strangers become people of their own, without a regroup; a
    /// library with nobody grouped yet, or too many new faces, is regrouped.
    #[test]
    fn new_faces_join_the_people_already_there() {
        let mut conn = open_memory().unwrap();
        add_photos(&conn, 24);
        let ada = unit(&[1.0, 0.0, 0.0]);
        for photo in 1..=22 {
            store_faces(
                &mut conn,
                photo,
                &[face_at(100.0, 100.0, false)],
                std::slice::from_ref(&ada),
                (1600, 1067),
            )
            .unwrap();
        }
        assert_eq!(
            place_new_faces(&mut conn).unwrap(),
            None,
            "nobody grouped yet"
        );
        regroup_people(&mut conn).unwrap();
        let ada_id = person_id_of(&conn, 1).unwrap();

        for (photo, e) in [(23, ada.clone()), (24, unit(&[0.0, 0.0, 1.0]))] {
            store_faces(
                &mut conn,
                photo,
                &[face_at(100.0, 100.0, false)],
                &[e],
                (1600, 1067),
            )
            .unwrap();
        }
        assert_eq!(place_new_faces(&mut conn).unwrap(), Some(2));
        assert_eq!(person_id_of(&conn, 23), Some(ada_id));
        assert_eq!(person_size(&conn, ada_id), 23);
        let cy = person_id_of(&conn, 24).unwrap();
        assert_ne!(cy, ada_id);
        assert_eq!(person_size(&conn, cy), 1);
        assert_eq!(place_new_faces(&mut conn).unwrap(), Some(2), "nothing new");
    }

    #[test]
    fn hand_added_faces_follow_their_links() {
        let group_of: std::collections::HashMap<i64, usize> =
            [(1, 0), (2, 0), (3, 1)].into_iter().collect();
        // 10 → group 0 twice, group 1 once; 11 linked to 10; 12, 13 to each other only.
        let links = [(10, 1), (10, 2), (10, 3), (11, 10), (12, 13)];
        assert_eq!(
            place_manual(&[10, 11, 12, 13, 14], &links, &group_of),
            vec![
                ManualPlace::Group(0),
                ManualPlace::Group(0),
                ManualPlace::Own(0),
                ManualPlace::Own(0),
                ManualPlace::Own(1),
            ]
        );
    }

    /// A profile joins the person it matches but cannot start one on its own.
    #[test]
    fn a_profile_attaches_but_never_founds_a_person() {
        let mut conn = open_memory().unwrap();
        add_photos(&conn, 3);
        let ada = unit(&[1.0, 0.0, 0.0]);
        store_faces(
            &mut conn,
            1,
            &[face_at(100.0, 100.0, false)],
            std::slice::from_ref(&ada),
            (1600, 1067),
        )
        .unwrap();
        store_faces(
            &mut conn,
            2,
            &[face_at(100.0, 100.0, true)],
            &[unit(&[0.9, 0.3, 0.0])],
            (1600, 1067),
        )
        .unwrap();
        // A lone profile of someone else entirely.
        store_faces(
            &mut conn,
            3,
            &[face_at(100.0, 100.0, true)],
            &[unit(&[0.0, 0.0, 1.0])],
            (1600, 1067),
        )
        .unwrap();
        assert_eq!(regroup_people(&mut conn).unwrap(), 1);
        let (p1, p2, p3): (Option<i64>, Option<i64>, Option<i64>) = conn
            .query_row(
                "SELECT (SELECT person_id FROM faces WHERE photo_id = 1),
                        (SELECT person_id FROM faces WHERE photo_id = 2),
                        (SELECT person_id FROM faces WHERE photo_id = 3)",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert!(p1.is_some());
        assert_eq!(p2, p1, "the profile joins the face it matches");
        assert_eq!(
            p3, None,
            "a lone profile is left unplaced, not made a person"
        );
    }
}
