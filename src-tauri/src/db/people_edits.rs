//! The user's changes to people: merging, naming and hiding them, and
//! adding or taking out photos and faces.

use super::*;

/// Records that everyone in `ids` is one person, and makes them one
/// (in place, see [`fold_people`]; it used to regroup the library).
///
/// The links are stored between *faces*, not people, because a regroup can
/// split or fold people — a person id is not a durable thing to record a
/// decision about. Linking one face of each person to
/// one face of the first is enough: the clustering pulls whole groups together
/// through a single link.
pub fn merge_people(conn: &mut Connection, ids: &[i32]) -> Result<usize> {
    Ok(merge_people_undoably(conn, ids)?.0)
}

/// What a merge changed, to put back with [`unmerge_people`]: a merge
/// was one keypress, and its only reverse was "Not them" on face after face.
#[derive(Debug, Clone, Default)]
pub struct MergeUndo {
    /// The "same person" links it made.
    links: Vec<(i64, i64)>,
    /// Faces it took the detached flag off.
    undetached: Vec<i64>,
    /// The person the others were folded into, and its name and hiding before.
    survivor: Option<(i64, Option<String>, bool)>,
    /// The people folded away: their id, name, hiding and faces.
    folded: Vec<(i64, Option<String>, bool, Vec<i64>)>,
}

/// [`merge_people`], and how to put it back. The second is `None` when
/// nothing was merged.
pub fn merge_people_undoably(
    conn: &mut Connection,
    ids: &[i32],
) -> Result<(usize, Option<MergeUndo>)> {
    if ids.len() < 2 {
        return Ok((person_count(conn)?, None));
    }
    // A representative face per person: the one the cover uses if it is still
    // there, else any of them.
    // Each with how much it can anchor: 2 a face that votes, 1 a detected
    // face that does not, 0 one added by hand.
    let mut reps: Vec<(i64, u8)> = Vec::new();
    {
        let mut stmt = conn.prepare(
            "SELECT COALESCE(
                      (SELECT pe.cover_face_id FROM people pe WHERE pe.id = ?1),
                      (SELECT f.id FROM faces f WHERE f.person_id = ?1
                        ORDER BY f.manual, f.id LIMIT 1))",
        )?;
        let mut face = conn.prepare("SELECT manual, score, w, h, pose FROM faces WHERE id = ?1")?;
        for &id in ids {
            let rep: Option<i64> = stmt.query_row(params![id], |r| r.get(0))?;
            if let Some(f) = rep {
                let rank = face.query_row(params![f], |r| {
                    let hand: i64 = r.get(0)?;
                    let score: Option<f64> = r.get(1)?;
                    let (w, h): (f64, f64) = (r.get(2)?, r.get(3)?);
                    let pose: Option<f64> = r.get(4)?;
                    Ok(if hand != 0 {
                        0
                    } else if crate::people::votes(
                        score.map(|s| s as f32),
                        w as f32,
                        h as f32,
                        pose.map(|p| p as f32),
                    ) {
                        2
                    } else {
                        1
                    })
                })?;
                reps.push((f, rank));
            }
        }
    }
    // Anchor on a face that votes when there is one: the clustering only
    // follows links between voting faces, and everything else is placed by
    // its links — so any other anchor would leave the voting people it was
    // meant to join still apart. The first best, so the order is the caller's.
    let Some(anchor) = reps.iter().rev().max_by_key(|r| r.1).map(|r| r.0) else {
        return Ok((person_count(conn)?, None));
    };
    let reps: Vec<i64> = reps.into_iter().map(|r| r.0).collect();

    let tx = conn.transaction()?;
    let mut undo = MergeUndo::default();
    // Everyone as they were, before anything changes.
    let before: Vec<(i64, Option<String>, bool, Vec<i64>)> = {
        let mut person = tx.prepare("SELECT id, name, hidden FROM people WHERE id = ?1")?;
        let mut faces = tx.prepare("SELECT id FROM faces WHERE person_id = ?1")?;
        let mut out = Vec::new();
        for &id in ids {
            if out
                .iter()
                .any(|p: &(i64, Option<String>, bool, Vec<i64>)| p.0 == id as i64)
            {
                continue;
            }
            let Some((pid, name, hidden)) = person
                .query_row(params![id], |r| {
                    Ok((
                        r.get::<_, i64>(0)?,
                        r.get::<_, Option<String>>(1)?,
                        r.get::<_, i64>(2)? != 0,
                    ))
                })
                .optional()?
            else {
                continue;
            };
            let members = faces
                .query_map(params![pid], |r| r.get(0))?
                .collect::<rusqlite::Result<Vec<i64>>>()?;
            out.push((pid, name, hidden, members));
        }
        out
    };
    {
        // Stored both ways round so a later lookup never has to guess the order.
        let mut ins = tx.prepare("INSERT OR IGNORE INTO face_links (a, b) VALUES (?1, ?2)")?;
        for &face in reps.iter().filter(|&&f| f != anchor) {
            for (a, b) in [(anchor, face), (face, anchor)] {
                if ins.execute(params![a, b])? > 0 {
                    undo.links.push((a, b));
                }
            }
        }
        // Merging people says they matter.
        let mut show = tx.prepare("UPDATE people SET hidden = 0 WHERE id = ?1")?;
        for &id in ids {
            show.execute(params![id])?;
        }
        // A face pulled back into a person is no longer detached, or the merge
        // would be undone by the very next regroup.
        let mut clear =
            tx.prepare("UPDATE faces SET detached = 0 WHERE id = ?1 AND detached != 0")?;
        for &face in &reps {
            if clear.execute(params![face])? > 0 {
                undo.undetached.push(face);
            }
        }
    }
    fold_people(&tx, ids)?;
    // Who survived: the one of them still there.
    for (pid, name, hidden, members) in before {
        let still: bool = tx.query_row(
            "SELECT EXISTS (SELECT 1 FROM people WHERE id = ?1)",
            params![pid],
            |r| r.get(0),
        )?;
        if still {
            undo.survivor = Some((pid, name, hidden));
        } else {
            undo.folded.push((pid, name, hidden, members));
        }
    }
    tx.commit()?;
    let merged = undo.survivor.is_some() && !undo.folded.is_empty();
    Ok((person_count(conn)?, merged.then_some(undo)))
}

/// Puts a merge back: the people folded away are people again, with
/// their names, hiding and those of their faces still in the merged person;
/// the merged person has its name back; and the links and flags the merge
/// changed are as they were.
pub fn unmerge_people(conn: &mut Connection, undo: &MergeUndo) -> Result<()> {
    let tx = conn.transaction()?;
    {
        let mut unlink = tx.prepare("DELETE FROM face_links WHERE a = ?1 AND b = ?2")?;
        for &(a, b) in &undo.links {
            unlink.execute(params![a, b])?;
        }
        let mut detach = tx.prepare("UPDATE faces SET detached = 1 WHERE id = ?1")?;
        for &face in &undo.undetached {
            detach.execute(params![face])?;
        }
    }
    let Some((survivor, name, hidden)) = &undo.survivor else {
        return Ok(());
    };
    for (folded_id, folded_name, folded_hidden, faces) in &undo.folded {
        // Under its own id when it is free: anything kept by id — a
        // filter, a saved view — finds the person again. Taken meanwhile, a
        // new one.
        let free: bool = tx.query_row(
            "SELECT NOT EXISTS (SELECT 1 FROM people WHERE id = ?1)",
            params![folded_id],
            |r| r.get(0),
        )?;
        if free {
            tx.execute(
                "INSERT INTO people (id, name, size, hidden) VALUES (?1, ?2, 0, ?3)",
                params![folded_id, folded_name, folded_hidden],
            )?;
        } else {
            tx.execute(
                "INSERT INTO people (name, size, hidden) VALUES (?1, 0, ?2)",
                params![folded_name, folded_hidden],
            )?;
        }
        let id = tx.last_insert_rowid();
        let mut back =
            tx.prepare("UPDATE faces SET person_id = ?2 WHERE id = ?1 AND person_id = ?3")?;
        for &face in faces {
            back.execute(params![face, id, survivor])?;
        }
        recover_person(&tx, id)?;
    }
    tx.execute(
        "UPDATE people SET name = ?2, hidden = ?3 WHERE id = ?1",
        params![survivor, name, hidden],
    )?;
    recover_person(&tx, *survivor)?;
    tx.commit()?;
    Ok(())
}

/// People who may be one person split in two, best first — the near misses
/// the grouping would not merge on its own (people::suggest). A pair the user
/// said is not the same person is never offered again.
pub fn people_suggestions(conn: &Connection, limit: usize) -> Result<Vec<(i64, i64, f32)>> {
    let mut by_person: std::collections::HashMap<i64, usize> = std::collections::HashMap::new();
    let mut ids: Vec<i64> = Vec::new();
    let mut embs: Vec<Vec<f32>> = Vec::new();
    let mut labels: Vec<usize> = Vec::new();
    let mut face_person: std::collections::HashMap<i64, usize> = std::collections::HashMap::new();
    {
        let mut stmt = conn.prepare(
            "SELECT f.id, f.person_id, e.embedding
               FROM faces f JOIN face_embeddings e ON e.face_id = f.id
              WHERE f.person_id IS NOT NULL AND f.detached = 0
                AND f.person_id NOT IN (SELECT id FROM people WHERE hidden = 1)",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, Vec<u8>>(2)?,
            ))
        })?;
        for row in rows {
            let (face, person, blob) = row?;
            let Some(e) = crate::search::decode_embedding(&blob) else {
                continue;
            };
            let next = ids.len();
            let g = *by_person.entry(person).or_insert_with(|| {
                ids.push(person);
                next
            });
            face_person.insert(face, g);
            embs.push(e);
            labels.push(g);
        }
    }
    let apart: std::collections::HashSet<(usize, usize)> = {
        let mut stmt = conn.prepare("SELECT a, b FROM people_apart")?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)))?;
        rows.filter_map(|r| r.ok())
            .filter_map(|(a, b)| {
                let (x, y) = (*face_person.get(&a)?, *face_person.get(&b)?);
                Some((x.min(y), x.max(y)))
            })
            .collect()
    };
    let named: std::collections::HashSet<i64> = {
        let mut stmt = conn.prepare("SELECT id FROM people WHERE name IS NOT NULL")?;
        let rows = stmt.query_map([], |r| r.get(0))?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    let found = crate::people::suggest(
        &embs,
        &labels,
        crate::people::SUGGEST_FLOOR,
        limit,
        &|a, b| apart.contains(&(a.min(b), a.max(b))),
        &|g| named.contains(&ids[g]),
    );
    Ok(found
        .into_iter()
        .map(|(a, b, s)| (ids[a], ids[b], s))
        .collect())
}

/// Records the user's "these are not the same person" about two people, so
/// they are never suggested again. Stored between a face of each, since a
/// regroup can split or fold people.
pub fn keep_people_apart(conn: &Connection, a: i32, b: i32) -> Result<()> {
    let rep = |id: i32| -> Result<Option<i64>> {
        Ok(conn.query_row(
            "SELECT COALESCE(
                      (SELECT pe.cover_face_id FROM people pe WHERE pe.id = ?1),
                      (SELECT f.id FROM faces f WHERE f.person_id = ?1
                        ORDER BY f.manual, f.id LIMIT 1))",
            params![id],
            |r| r.get(0),
        )?)
    };
    if let (Some(x), Some(y)) = (rep(a)?, rep(b)?) {
        conn.execute(
            "INSERT OR IGNORE INTO people_apart (a, b) VALUES (?1, ?2)",
            params![x.min(y), x.max(y)],
        )?;
    }
    Ok(())
}

/// Pulls one face out of the person it was grouped into.
///
/// The face becomes its own person and stays that way: `detached` keeps it out
/// of the next clustering run, which would otherwise put it straight back where
/// the user just said it does not belong. Merging it into another person clears
/// the flag again.
pub fn detach_face(conn: &mut Connection, face_id: i32) -> Result<usize> {
    // A face the user added by hand was only ever their say-so: taking it out
    // is deleting it, and nothing needs reclustering.
    let manual: Option<i64> = conn
        .query_row(
            "SELECT manual FROM faces WHERE id = ?1",
            params![face_id],
            |r| r.get(0),
        )
        .optional()?;
    if manual == Some(1) {
        conn.execute("DELETE FROM faces WHERE id = ?1", params![face_id])?;
        recount_people(conn)?;
        return Ok(
            conn.query_row("SELECT COUNT(*) FROM people", [], |r| r.get::<_, i64>(0))? as usize,
        );
    }
    let Some(old) = manual.and(
        conn.query_row(
            "SELECT person_id FROM faces WHERE id = ?1",
            params![face_id],
            |r| r.get::<_, Option<i64>>(0),
        )
        .optional()?,
    ) else {
        return person_count(conn);
    };
    let tx = conn.transaction()?;
    tx.execute(
        "UPDATE faces SET detached = 1 WHERE id = ?1",
        params![face_id],
    )?;
    // Any link that put it here is stale now — keeping it would fight the flag.
    tx.execute(
        "DELETE FROM face_links WHERE a = ?1 OR b = ?1",
        params![face_id],
    )?;
    // Its own person now, unnamed, rather than after a regroup of the whole
    // library. Not its old person under a new id: a regroup carries
    // a name over by membership, so a face still pointing at "Beth" would
    // come back as a second person called Beth.
    own_person(&tx, face_id as i64)?;
    if let Some(p) = old {
        recover_person(&tx, p)?;
    }
    tx.commit()?;
    person_count(conn)
}

/// Makes `face_id` the cover of the person it belongs to, and keeps it
/// that across regroups.
pub fn set_cover_face(conn: &mut Connection, face_id: i32) -> Result<()> {
    let tx = conn.transaction()?;
    // A detected face only: a hand-added one may have no box to crop to.
    let person: Option<i32> = tx
        .query_row(
            "SELECT person_id FROM faces WHERE id = ?1 AND manual = 0",
            params![face_id],
            |r| r.get(0),
        )
        .optional()?
        .flatten();
    let Some(person) = person else {
        bail!("that face can't be a cover");
    };
    tx.execute(
        "UPDATE faces SET cover_pick = 0 WHERE person_id = ?1 AND cover_pick = 1",
        params![person],
    )?;
    tx.execute(
        "UPDATE faces SET cover_pick = 1 WHERE id = ?1",
        params![face_id],
    )?;
    tx.execute(
        "UPDATE people SET cover_face_id = ?2 WHERE id = ?1",
        params![person, face_id],
    )?;
    tx.commit()?;
    Ok(())
}

/// Says who a detected face is, from the Lightbox. Returns whether a
/// regroup ran.
///
/// - Its person has no name and nobody else has this one: the person is named.
/// - Someone else already has the name: the face's person *is* them — the same
///   decision as merging the two by hand. A face grouping left out (too small or
///   weak to vote) instead joins them by a link, as a merged weak face does.
/// - A face grouping left out, with a name nobody has: it becomes that person,
///   on its own, the way a face pulled out with "Not them" does.
///
/// A face whose person already has another name is not renamed here: "Not
/// them" takes it out first, so one click never renames everyone in a group.
pub fn name_face(conn: &mut Connection, face_id: i32, name: &str) -> Result<bool> {
    let name = name.trim();
    if name.is_empty() {
        bail!("a person needs a name");
    }
    let Some((person, current)): Option<(Option<i32>, Option<String>)> = conn
        .query_row(
            "SELECT f.person_id, pe.name FROM faces f LEFT JOIN people pe ON pe.id = f.person_id
              WHERE f.id = ?1 AND f.manual = 0",
            params![face_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?
    else {
        bail!("that face is no longer in the library");
    };
    if current.is_some() {
        bail!("that face is already someone's — take it out with “Not them” first");
    }
    let known: Option<i32> = conn
        .query_row(
            "SELECT id FROM people WHERE name = ?1 COLLATE NOCASE ORDER BY size DESC, id LIMIT 1",
            params![name],
            |r| r.get(0),
        )
        .optional()?;
    match (person, known) {
        (Some(p), None) => {
            set_person_name(conn, p, name)?;
            Ok(false)
        }
        (Some(p), Some(q)) => {
            merge_people(conn, &[q, p])?;
            Ok(true)
        }
        (None, Some(q)) => {
            let tx = conn.transaction()?;
            for a in person_anchors(&tx, q)? {
                link_faces(&tx, face_id as i64, a)?;
            }
            tx.execute(
                "UPDATE faces SET person_id = ?2, detached = 0 WHERE id = ?1",
                params![face_id, q],
            )?;
            tx.execute(
                "UPDATE people SET size = size + 1, hidden = 0 WHERE id = ?1",
                params![q],
            )?;
            tx.commit()?;
            Ok(false)
        }
        (None, None) => {
            let tx = conn.transaction()?;
            tx.execute(
                "INSERT INTO people (name, cover_face_id, size) VALUES (?1, ?2, 1)",
                params![name, face_id],
            )?;
            let p = tx.last_insert_rowid() as i32;
            // Detached keeps it its own person through a regroup, as "Not
            // them" does; the name comes along with it.
            tx.execute(
                "UPDATE faces SET person_id = ?2, detached = 1 WHERE id = ?1",
                params![face_id, p],
            )?;
            tx.commit()?;
            Ok(false)
        }
    }
}

/// Rename a person (blank clears the name back to unnamed).
pub fn set_person_name(conn: &Connection, id: i32, name: &str) -> Result<()> {
    let name = name.trim();
    conn.execute(
        "UPDATE people SET name = ?2 WHERE id = ?1",
        params![id, (!name.is_empty()).then_some(name)],
    )?;
    Ok(())
}

/// Hides people the user does not care about, or shows them again. A hidden
/// person stays hidden across regroups (see [`regroup_people_with`]) and is
/// never offered as a "same person?" question.
pub fn set_people_hidden(conn: &Connection, ids: &[i32], hidden: bool) -> Result<()> {
    let mut stmt = conn.prepare("UPDATE people SET hidden = ?2 WHERE id = ?1")?;
    for &id in ids {
        stmt.execute(params![id, hidden])?;
    }
    Ok(())
}

/// Which faces a hand-added face of `person_id` should link to, so it finds
/// its person again after a regroup: several of the person's detected faces,
/// so losing one (a detach, a removed photo) does not strand it, or failing
/// those — a person only the user has seen — one of its hand-added ones.
pub(super) fn person_anchors(conn: &Connection, person_id: i32) -> Result<Vec<i64>> {
    let mut detected = conn.prepare(
        "SELECT id FROM faces WHERE person_id = ?1 AND manual = 0
          ORDER BY COALESCE(score, 0) DESC, id LIMIT 5",
    )?;
    let found: Vec<i64> = detected
        .query_map(params![person_id], |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    if !found.is_empty() {
        return Ok(found);
    }
    Ok(conn
        .query_row(
            "SELECT id FROM faces WHERE person_id = ?1 AND manual = 1 ORDER BY id LIMIT 1",
            params![person_id],
            |r| r.get(0),
        )
        .optional()?
        .into_iter()
        .collect())
}

/// Puts `person_id` in each photo by hand, adding a boxless face row per
/// photo they are not already in; returns how many were added.
///
/// This is for the photos the detector could not help with — the back of a
/// head, a face in shadow, someone too small to find — so it does not ask it.
/// Virtual copies resolve to their master, where faces live.
pub fn add_photos_to_person(
    conn: &mut Connection,
    photo_ids: &[i32],
    person_id: i32,
) -> Result<usize> {
    let anchors = person_anchors(conn, person_id)?;
    if anchors.is_empty() {
        bail!("that person is no longer in the library");
    }
    let tx = conn.transaction()?;
    let added = insert_manual_faces(&tx, photo_ids, person_id, &anchors)?;
    // Putting someone in a photo says they matter.
    tx.execute(
        "UPDATE people SET size = size + ?2, hidden = 0 WHERE id = ?1",
        params![person_id, added.len() as i64],
    )?;
    tx.commit()?;
    Ok(added.len())
}

/// A new person made only of photos the user put them in — someone the
/// detector never found at all. Returns the person's id.
pub fn add_photos_to_new_person(
    conn: &mut Connection,
    photo_ids: &[i32],
    name: &str,
) -> Result<i32> {
    let name = name.trim();
    let tx = conn.transaction()?;
    tx.execute(
        "INSERT INTO people (name, size) VALUES (?1, 0)",
        params![(!name.is_empty()).then_some(name)],
    )?;
    let person = tx.last_insert_rowid() as i32;
    // Each face links to the first, which is what holds them together as one
    // person through a regroup.
    let mut added: Vec<i64> = Vec::new();
    for &photo in photo_ids {
        let anchor: Vec<i64> = added.first().copied().into_iter().collect();
        added.extend(insert_manual_faces(&tx, &[photo], person, &anchor)?);
    }
    let Some(&first) = added.first() else {
        bail!("none of those photos are in the library");
    };
    tx.execute(
        "UPDATE people SET size = ?2, cover_face_id = ?3 WHERE id = ?1",
        params![person, added.len() as i64, first],
    )?;
    tx.commit()?;
    Ok(person)
}

/// Adds a hand-added face of `person` to each photo (as its master) that does
/// not already have one of theirs, linked both ways to every face in
/// `anchors`. Returns the new face ids.
fn insert_manual_faces(
    tx: &Connection,
    photo_ids: &[i32],
    person: i32,
    anchors: &[i64],
) -> Result<Vec<i64>> {
    let mut master = tx.prepare("SELECT COALESCE(variant_of, id) FROM photos WHERE id = ?1")?;
    let mut has =
        tx.prepare("SELECT EXISTS (SELECT 1 FROM faces WHERE photo_id = ?1 AND person_id = ?2)")?;
    let mut ins = tx.prepare(
        "INSERT INTO faces (photo_id, x, y, w, h, manual, person_id)
         VALUES (?1, 0, 0, 0, 0, 1, ?2)",
    )?;
    let mut link = tx.prepare("INSERT OR IGNORE INTO face_links (a, b) VALUES (?1, ?2)")?;
    let mut added = Vec::new();
    for &photo in photo_ids {
        let Some(m) = master
            .query_row(params![photo], |r| r.get::<_, i32>(0))
            .optional()?
        else {
            continue;
        };
        if has.query_row(params![m, person], |r| r.get::<_, bool>(0))? {
            continue;
        }
        ins.execute(params![m, person])?;
        let face = tx.last_insert_rowid();
        for &a in anchors {
            link.execute(params![face, a])?;
            link.execute(params![a, face])?;
        }
        added.push(face);
    }
    Ok(added)
}

/// Takes `person_id` out of these photos: hand-added faces are deleted, and a
/// detected face is detached as "Not them" is in the review sheet.
pub fn remove_photos_from_person(
    conn: &mut Connection,
    photo_ids: &[i32],
    person_id: i32,
) -> Result<()> {
    let tx = conn.transaction()?;
    {
        let mut master = tx.prepare("SELECT COALESCE(variant_of, id) FROM photos WHERE id = ?1")?;
        let mut drop_manual =
            tx.prepare("DELETE FROM faces WHERE photo_id = ?1 AND person_id = ?2 AND manual = 1")?;
        let mut detach = tx.prepare(
            "UPDATE faces SET detached = 1, person_id = NULL
              WHERE photo_id = ?1 AND person_id = ?2 AND manual = 0
          RETURNING id",
        )?;
        let mut unlink = tx.prepare("DELETE FROM face_links WHERE a = ?1 OR b = ?1")?;
        for &photo in photo_ids {
            let Some(m) = master
                .query_row(params![photo], |r| r.get::<_, i32>(0))
                .optional()?
            else {
                continue;
            };
            drop_manual.execute(params![m, person_id])?;
            let faces: Vec<i64> = detach
                .query_map(params![m, person_id], |r| r.get(0))?
                .collect::<rusqlite::Result<_>>()?;
            for face in faces {
                unlink.execute(params![face])?;
                // Its own person straight away, as "Not them".
                own_person(&tx, face)?;
            }
        }
    }
    recover_person(&tx, person_id as i64)?;
    tx.commit()?;
    Ok(())
}

/// Who is in a photo (as its master): each person once, and whether the user
/// put them there by hand rather than the detector finding them.
pub fn photo_people(conn: &Connection, photo_id: i32) -> Result<Vec<PhotoPerson>> {
    let mut stmt = conn.prepare(
        "SELECT f.person_id, MIN(f.manual)
           FROM faces f
          WHERE f.person_id IS NOT NULL
            AND f.photo_id = (SELECT COALESCE(variant_of, id) FROM photos WHERE id = ?1)
          GROUP BY f.person_id
          ORDER BY f.person_id",
    )?;
    let rows = stmt.query_map(params![photo_id], |r| {
        Ok(PhotoPerson {
            id: r.get(0)?,
            manual: r.get::<_, i64>(1)? != 0,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// A person in one photo, for the selection panel's People section.
#[derive(Debug, Clone, serde::Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct PhotoPerson {
    pub id: i32,
    /// Put there by hand only — no detected face of theirs in this photo.
    pub manual: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::testing::*;

    /// Near misses are offered; answering "not the same" stops the offer.
    #[test]
    fn suggestions_offer_near_misses_until_told_apart() {
        let mut conn = open_memory().unwrap();
        add_photos(&conn, 4);
        // Two people, two faces each, agreeing at 0.33 across: under the 0.40
        // merge, over the 0.25 suggestion floor.
        let a = unit(&[1.0, 0.0, 0.0]);
        let b = unit(&[0.33, 0.944, 0.0]);
        for (photo, e) in [(1, &a), (2, &a), (3, &b), (4, &b)] {
            store_faces(
                &mut conn,
                photo,
                &[face_at(100.0, 100.0, false)],
                std::slice::from_ref(e),
                (1600, 1067),
            )
            .unwrap();
        }
        regroup_people(&mut conn).unwrap();
        let found = people_suggestions(&conn, 5).unwrap();
        assert!(!found.is_empty(), "expected a suggestion");
        let (x, y, score) = found[0];
        assert!(
            (crate::people::SUGGEST_FLOOR..0.40).contains(&score),
            "{score}"
        );

        keep_people_apart(&conn, x as i32, y as i32).unwrap();
        assert!(
            !people_suggestions(&conn, 5)
                .unwrap()
                .iter()
                .any(|s| (s.0, s.1) == (x, y)),
            "a pair told apart is not offered again"
        );
    }

    /// Naming a face from the Lightbox: an unnamed person takes the
    /// name; a name someone has joins them; a named face is not renamed.
    #[test]
    fn a_face_is_named_from_the_lightbox() {
        let mut conn = open_memory().unwrap();
        ada_and_bo(&mut conn, 4);
        let bo = face_on(&conn, 3);
        assert!(!name_face(&mut conn, bo, "Bo").unwrap());
        assert_eq!(person_of_photo(&conn, 3).as_deref(), Some("Bo"));
        assert!(
            name_face(&mut conn, bo, "Cy").is_err(),
            "Bo is not renamed from one face"
        );

        let ada1 = face_on(&conn, 1);
        assert!(name_face(&mut conn, ada1, "Bo").is_err());
    }

    #[test]
    fn a_face_named_after_someone_joins_them() {
        let mut conn = open_memory().unwrap();
        ada_and_bo(&mut conn, 4);
        let bo = face_on(&conn, 3);
        assert!(name_face(&mut conn, bo, "ada").unwrap(), "a merge regroups");
        assert_eq!(person_of_photo(&conn, 3).as_deref(), Some("Ada"));
    }

    /// Faces grouping left out can be named too, and it lasts: by a link to
    /// someone who has the name, or as a person of their own.
    #[test]
    fn a_face_grouping_left_out_can_be_named_and_stays_named() {
        let mut conn = open_memory().unwrap();
        ada_and_bo(&mut conn, 5);
        let loose = loose_face(&mut conn, 4);
        assert_eq!(person_of_photo(&conn, 4), None, "left out");
        name_face(&mut conn, loose, "Ada").unwrap();
        assert_eq!(person_of_photo(&conn, 4).as_deref(), Some("Ada"));

        let other = loose_face(&mut conn, 5);
        name_face(&mut conn, other, "Cy").unwrap();
        assert_eq!(person_of_photo(&conn, 5).as_deref(), Some("Cy"));

        regroup_people(&mut conn).unwrap();
        assert_eq!(person_of_photo(&conn, 4).as_deref(), Some("Ada"));
        assert_eq!(person_of_photo(&conn, 5).as_deref(), Some("Cy"));
    }

    /// A face taken out with "Not them" and then named after someone joins
    /// them through regroups; before, a detached face stayed alone whatever
    /// it was linked to.
    #[test]
    fn a_face_taken_out_then_named_joins_who_it_was_named_for() {
        let mut conn = open_memory().unwrap();
        ada_and_bo(&mut conn, 4);
        let wrong = face_on(&conn, 2);
        detach_face(&mut conn, wrong).unwrap();
        assert_eq!(person_of_photo(&conn, 2), None, "its own, unnamed");
        name_face(&mut conn, wrong, "Bo").unwrap();
        regroup_people(&mut conn).unwrap();
        assert_eq!(person_of_photo(&conn, 2).as_deref(), Some("Bo"));
    }

    /// Hiding a person keeps them hidden through a regroup, and out of the
    /// "same person?" questions; merging them shows them again.
    #[test]
    fn a_hidden_person_stays_hidden_until_merged() {
        let mut conn = open_memory().unwrap();
        let (_, bo) = ada_and_bo(&mut conn, 4);
        set_people_hidden(&conn, &[bo], true).unwrap();
        // A new photo of Bo joins him and does not outvote the old ones.
        store_faces(
            &mut conn,
            4,
            &[face_at(100.0, 100.0, false)],
            &[unit(&[0.0, 1.0, 0.0])],
            (1600, 1067),
        )
        .unwrap();
        regroup_people(&mut conn).unwrap();
        let hidden: Vec<(i32, i32)> = list_people(&conn)
            .unwrap()
            .iter()
            .filter(|p| p.hidden)
            .map(|p| (p.id, p.count))
            .collect();
        assert_eq!(hidden.len(), 1);
        assert_eq!(hidden[0].1, 2);
        assert!(
            !list_people(&conn)
                .unwrap()
                .iter()
                .find(|p| p.name.as_deref() == Some("Ada"))
                .unwrap()
                .hidden
        );
        assert!(people_suggestions(&conn, 5)
            .unwrap()
            .iter()
            .all(|s| s.0 != hidden[0].0 as i64 && s.1 != hidden[0].0 as i64));

        let a = ada(&conn);
        merge_people(&mut conn, &[a, hidden[0].0]).unwrap();
        assert!(list_people(&conn).unwrap().iter().all(|p| !p.hidden));
    }

    /// "Not them" makes a face its own person — unnamed, not a second copy of
    /// the person it was taken out of.
    #[test]
    fn a_detached_face_does_not_take_the_name_with_it() {
        let mut conn = open_memory().unwrap();
        ada_and_bo(&mut conn, 3);
        let face: i32 = conn
            .query_row("SELECT id FROM faces WHERE photo_id = 2", [], |r| r.get(0))
            .unwrap();
        detach_face(&mut conn, face).unwrap();
        assert_eq!(person_of_photo(&conn, 1).as_deref(), Some("Ada"));
        assert_eq!(person_of_photo(&conn, 2), None);
    }

    /// A merge folds the people into one in place — no regroup of the
    /// library — and a regroup afterwards agrees.
    #[test]
    fn a_merge_folds_people_in_place() {
        let mut conn = open_memory().unwrap();
        let (ada, bo) = ada_and_bo(&mut conn, 4);
        store_faces(
            &mut conn,
            4,
            &[face_at(100.0, 100.0, false)],
            &[unit(&[0.0, 0.0, 1.0])],
            (1600, 1067),
        )
        .unwrap();
        regroup_people(&mut conn).unwrap();
        let cy = person_id_of(&conn, 4).unwrap();
        let before = people_fingerprint(&conn).unwrap();

        assert_eq!(merge_people(&mut conn, &[bo, ada]).unwrap(), 2);
        // Ada has the name, so Ada keeps her id; Cy is untouched.
        assert_eq!(person_id_of(&conn, 3), Some(ada));
        assert_eq!(person_size(&conn, ada), 3);
        assert_eq!(person_id_of(&conn, 4), Some(cy));
        assert_ne!(people_fingerprint(&conn).unwrap(), before);

        regroup_people(&mut conn).unwrap();
        assert_eq!(person_id_of(&conn, 3), Some(ada));
        assert_eq!(person_id_of(&conn, 1), Some(ada));
        assert_eq!(person_id_of(&conn, 4), Some(cy));
    }

    /// A merge can be taken back: both people as they were, names,
    /// hiding and faces, and no link left to merge them at the next regroup.
    #[test]
    fn a_merge_can_be_undone() {
        let mut conn = open_memory().unwrap();
        let (ada, bo) = ada_and_bo(&mut conn, 4);
        set_person_name(&conn, bo, "Bo").unwrap();
        set_people_hidden(&conn, &[bo], true).unwrap();
        let links_before: i64 = conn
            .query_row("SELECT COUNT(*) FROM face_links", [], |r| r.get(0))
            .unwrap();

        let (n, undo) = merge_people_undoably(&mut conn, &[ada, bo]).unwrap();
        assert_eq!(n, 1);
        unmerge_people(&mut conn, &undo.expect("a merge happened")).unwrap();

        assert_eq!(person_of_photo(&conn, 1).as_deref(), Some("Ada"));
        assert_eq!(person_of_photo(&conn, 3).as_deref(), Some("Bo"));
        let bo_now = person_id_of(&conn, 3).unwrap();
        assert_ne!(bo_now, person_id_of(&conn, 1).unwrap());
        // Back under the id it had, whichever one was folded.
        assert!(bo_now == bo || person_id_of(&conn, 1) == Some(bo));
        assert!([ada, bo].contains(&bo_now));
        let hidden: bool = conn
            .query_row(
                "SELECT hidden FROM people WHERE id = ?1",
                params![bo_now],
                |r| r.get(0),
            )
            .unwrap();
        assert!(hidden, "Bo is hidden again");
        let links_after: i64 = conn
            .query_row("SELECT COUNT(*) FROM face_links", [], |r| r.get(0))
            .unwrap();
        assert_eq!(links_after, links_before);
        regroup_people(&mut conn).unwrap();
        assert_ne!(
            person_id_of(&conn, 3),
            person_id_of(&conn, 1),
            "a regroup keeps them apart"
        );
    }

    /// "Not them" makes the face its own person straight away, and the
    /// person it left is recounted and keeps a cover it still has.
    #[test]
    fn not_them_makes_its_own_person_in_place() {
        let mut conn = open_memory().unwrap();
        let (ada, _) = ada_and_bo(&mut conn, 3);
        let face = face_on(&conn, 2);
        detach_face(&mut conn, face).unwrap();
        let own = person_id_of(&conn, 2).unwrap();
        assert_ne!(own, ada);
        assert_eq!(person_size(&conn, ada), 1);
        assert_eq!(person_size(&conn, own), 1);
        let cover: i32 = conn
            .query_row(
                "SELECT cover_face_id FROM people WHERE id = ?1",
                params![ada],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(cover, face_on(&conn, 1));

        regroup_people(&mut conn).unwrap();
        assert_eq!(
            person_id_of(&conn, 2),
            Some(own),
            "a regroup leaves it alone"
        );
        assert_eq!(person_id_of(&conn, 1), Some(ada));
    }

    /// A face that cannot vote — turned away, or too small to trust — joins
    /// whoever the user merges it with. The merge link used to be followed
    /// only between voting faces, so merging one of these into a person took
    /// it out of every person instead.
    #[test]
    fn a_face_too_weak_to_vote_joins_the_person_it_is_merged_into() {
        let mut conn = open_memory().unwrap();
        ada_and_bo(&mut conn, 5);
        // Photo 4 a profile, photo 5 a tiny face: neither looks like Ada.
        let other = unit(&[0.0, 0.0, 1.0]);
        store_faces(
            &mut conn,
            4,
            &[face_at(100.0, 100.0, true)],
            std::slice::from_ref(&other),
            (1600, 1067),
        )
        .unwrap();
        let mut tiny = face_at(100.0, 100.0, false);
        tiny.w = 20.0;
        tiny.h = 20.0;
        store_faces(
            &mut conn,
            5,
            &[tiny],
            std::slice::from_ref(&other),
            (1600, 1067),
        )
        .unwrap();
        for photo in [4, 5] {
            let face: i32 = conn
                .query_row(
                    "SELECT id FROM faces WHERE photo_id = ?1",
                    params![photo],
                    |r| r.get(0),
                )
                .unwrap();
            // "Not them" is how one of these becomes a person of its own.
            detach_face(&mut conn, face).unwrap();
            let own: i32 = conn
                .query_row(
                    "SELECT person_id FROM faces WHERE id = ?1",
                    params![face],
                    |r| r.get(0),
                )
                .unwrap();
            // Listed second: the anchor has to be a face that votes.
            let a = ada(&conn);
            merge_people(&mut conn, &[own, a]).unwrap();
            assert_eq!(
                person_of_photo(&conn, photo).as_deref(),
                Some("Ada"),
                "photo {photo}"
            );
        }
        // And stays there through a regroup.
        regroup_people(&mut conn).unwrap();
        assert_eq!(person_of_photo(&conn, 4).as_deref(), Some("Ada"));
        assert_eq!(person_of_photo(&conn, 5).as_deref(), Some("Ada"));
        assert_eq!(
            person_of_photo(&conn, 3),
            None,
            "Bo is still his own person"
        );
    }

    /// Merging three people with a weak face listed first still makes one
    /// person: anchoring on the weak face linked the other two only through a
    /// face the clustering never saw.
    #[test]
    fn a_merge_anchors_on_a_face_that_votes() {
        let mut conn = open_memory().unwrap();
        ada_and_bo(&mut conn, 4);
        store_faces(
            &mut conn,
            4,
            &[face_at(100.0, 100.0, true)],
            &[unit(&[0.0, 0.0, 1.0])],
            (1600, 1067),
        )
        .unwrap();
        let face: i32 = conn
            .query_row("SELECT id FROM faces WHERE photo_id = 4", [], |r| r.get(0))
            .unwrap();
        detach_face(&mut conn, face).unwrap();
        let weak: i32 = conn
            .query_row(
                "SELECT person_id FROM faces WHERE id = ?1",
                params![face],
                |r| r.get(0),
            )
            .unwrap();
        let bo_now: i32 = conn
            .query_row("SELECT person_id FROM faces WHERE photo_id = 3", [], |r| {
                r.get(0)
            })
            .unwrap();
        let a = ada(&conn);
        merge_people(&mut conn, &[weak, a, bo_now]).unwrap();
        let people: std::collections::HashSet<Option<String>> =
            (1..=4).map(|p| person_of_photo(&conn, p)).collect();
        assert_eq!(people.len(), 1, "{people:?}");
    }
}
