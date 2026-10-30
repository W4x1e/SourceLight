//! Develop settings: each photo's edit stack, and the presets made from
//! parts of one.

use super::*;

/// How many photos' edits use the LUT `name`, for the question before
/// it is deleted: a deleted LUT silently drops out of every one of them.
pub fn lut_usage(conn: &Connection, name: &str) -> Result<usize> {
    Ok(conn.query_row(
        "SELECT COUNT(*) FROM photos
          WHERE develop_settings IS NOT NULL
            AND json_valid(develop_settings)
            AND json_extract(develop_settings, '$.lut.name') = ?1",
        params![name],
        |r| r.get::<_, i64>(0),
    )? as usize)
}

/// Reads a photo's edit stack, or `None` if it has never been developed.
///
/// A parse failure is reported rather than swallowed: silently substituting
/// defaults would quietly discard someone's edits, which is worse than an error.
pub fn get_develop_settings(conn: &Connection, photo_id: i32) -> Result<Option<EditStack>> {
    let json: Option<String> = conn
        .query_row(
            "SELECT develop_settings FROM photos WHERE id = ?1",
            params![photo_id],
            |r| r.get(0),
        )
        .optional()?
        .flatten();

    match json {
        Some(json) => Ok(Some(EditStack::from_json(&json)?)),
        None => Ok(None),
    }
}

/// Every magic-erase fill the edits in the catalogue at `path` name, opened
/// read-only: the live catalogue or a backup of it. An edit that cannot be
/// read is an error, not an empty answer, since the answer decides what is
/// deleted.
pub fn erase_fills_in(path: &Path) -> Result<std::collections::HashSet<String>> {
    let conn = Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let mut stmt = conn.prepare(
        "SELECT develop_settings FROM photos WHERE develop_settings LIKE '%\"erases\":[{%'",
    )?;
    let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
    let mut names = std::collections::HashSet::new();
    for json in rows {
        let stack = EditStack::from_json(&json?)?;
        names.extend(
            crate::develop::erase::fill_names(&stack.erases)
                .into_iter()
                .map(str::to_string),
        );
    }
    Ok(names)
}

/// Writes a photo's edit stack. A stack back at its defaults is stored as NULL,
/// so "never edited" and "edited back to zero" stay distinguishable.
/// Puts photos' develop settings back as they were, all or none:
/// `None` is no edit. A photo forgotten since is skipped.
pub fn restore_develop_settings(
    conn: &mut Connection,
    before: &[(i32, Option<EditStack>)],
) -> Result<()> {
    let tx = conn.transaction()?;
    for (id, stack) in before {
        set_develop_settings(&tx, *id, stack.as_ref().unwrap_or(&EditStack::default()))?;
    }
    tx.commit()?;
    Ok(())
}

pub fn set_develop_settings(conn: &Connection, photo_id: i32, stack: &EditStack) -> Result<()> {
    let json = if stack.is_default() {
        None
    } else {
        Some(stack.to_json()?)
    };
    conn.execute(
        "UPDATE photos SET develop_settings = ?2 WHERE id = ?1",
        params![photo_id, json],
    )?;
    // Its colours are read again, as edited.
    conn.execute(
        "DELETE FROM photo_palette WHERE photo_id = ?1",
        params![photo_id],
    )?;
    Ok(())
}

/// Keeps `path`, a render of the photo as edited, on its row — only while
/// its edits are still `settings`, the ones it was rendered from.
pub fn set_edited_path(conn: &Connection, photo_id: i32, settings: &str, path: &str) -> Result<()> {
    conn.execute(
        "UPDATE photos SET edited_path = ?3 WHERE id = ?1 AND develop_settings = ?2",
        params![photo_id, settings, path],
    )?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Presets
// ---------------------------------------------------------------------------

/// A named, reusable edit stack.
#[derive(Debug, Clone, PartialEq, serde::Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct Preset {
    pub id: i32,
    pub name: String,
    pub stack: EditStack,
    /// The parts of an edit the preset carries; `None` for one saved
    /// before presets had parts, which still applies whole, as it always did.
    pub groups: Option<Vec<crate::develop::stack::Group>>,
}

/// Where a preset's groups sit in its saved JSON, beside the stack's own
/// fields — which the stack's parser ignores.
const PRESET_GROUPS: &str = "presetGroups";

impl Preset {
    /// This preset applied to `current`: its groups, or all of it but the
    /// erases, which are the photo's own.
    pub fn applied_to(&self, current: &EditStack) -> EditStack {
        match &self.groups {
            Some(groups) => self.stack.take_groups(current, groups),
            None => EditStack {
                erases: current.erases.clone(),
                ..self.stack.clone()
            },
        }
    }
}

pub fn list_presets(conn: &Connection) -> Result<Vec<Preset>> {
    let mut stmt = conn.prepare("SELECT id, name, settings FROM presets ORDER BY name")?;
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, i32>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
        ))
    })?;

    let mut out = Vec::new();
    for row in rows {
        let (id, name, settings) = row?;
        // A preset whose JSON no longer parses is skipped rather than fatal:
        // one bad row must not make the whole panel unusable.
        let groups = serde_json::from_str::<serde_json::Value>(&settings)
            .ok()
            .and_then(|v| v.get(PRESET_GROUPS).cloned())
            .and_then(|g| serde_json::from_value(g).ok());
        match EditStack::from_json(&settings) {
            Ok(stack) => out.push(Preset {
                id,
                name,
                stack,
                groups,
            }),
            Err(e) => eprintln!("skipping preset {name:?}: {e}"),
        }
    }
    Ok(out)
}

/// Creates or replaces a preset by name. Re-saving under an existing name
/// overwrites it, which is what "save preset" means everywhere else.
pub fn save_preset(
    conn: &Connection,
    name: &str,
    stack: &EditStack,
    groups: Option<&[crate::develop::stack::Group]>,
) -> Result<i32> {
    let name = name.trim();
    if name.is_empty() {
        bail!("a preset needs a name");
    }
    // Erases fill one photo's own pixels: no preset carries them.
    let stack = EditStack {
        erases: Vec::new(),
        ..stack.clone()
    };
    let mut value: serde_json::Value = serde_json::from_str(&stack.to_json()?)?;
    if let (Some(groups), Some(map)) = (groups, value.as_object_mut()) {
        map.insert(PRESET_GROUPS.into(), serde_json::to_value(groups)?);
    }
    conn.execute(
        "INSERT INTO presets(name, settings, created_at) VALUES(?1, ?2, ?3)
         ON CONFLICT(name) DO UPDATE SET settings = excluded.settings",
        params![name, value.to_string(), chrono::Utc::now().to_rfc3339()],
    )?;
    Ok(conn.query_row(
        "SELECT id FROM presets WHERE name = ?1",
        params![name],
        |r| r.get(0),
    )?)
}

pub fn delete_preset(conn: &Connection, id: i32) -> Result<()> {
    conn.execute("DELETE FROM presets WHERE id = ?1", params![id])?;
    Ok(())
}

/// Writes one edit stack onto many photos in a single transaction.
///
/// All or nothing: a partial apply across a selection would leave the user
/// unsure which photos had been touched.
/// Copies the chosen groups of `stack` onto each photo's own settings,
/// in one transaction. The rest of each photo's edit is kept.
pub fn apply_groups_to(
    conn: &mut Connection,
    photo_ids: &[i32],
    stack: &EditStack,
    groups: &[crate::develop::stack::Group],
) -> Result<usize> {
    let targets: Vec<(i32, EditStack)> = photo_ids
        .iter()
        .map(|&id| Ok((id, get_develop_settings(conn, id)?.unwrap_or_default())))
        .collect::<Result<_>>()?;
    let tx = conn.transaction()?;
    {
        let mut stmt = tx.prepare("UPDATE photos SET develop_settings = ?2 WHERE id = ?1")?;
        for (id, own) in &targets {
            let merged = stack.take_groups(own, groups);
            let json = if merged.is_default() {
                None
            } else {
                Some(merged.to_json()?)
            };
            stmt.execute(params![id, json])?;
        }
    }
    tx.commit()?;
    Ok(targets.len())
}

pub fn apply_stack_to(
    conn: &mut Connection,
    photo_ids: &[i32],
    stack: &EditStack,
) -> Result<usize> {
    let json = if stack.is_default() {
        None
    } else {
        Some(stack.to_json()?)
    };

    let tx = conn.transaction()?;
    {
        let mut stmt = tx.prepare("UPDATE photos SET develop_settings = ?2 WHERE id = ?1")?;
        for id in photo_ids {
            stmt.execute(params![id, json])?;
        }
    }
    tx.commit()?;
    Ok(photo_ids.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::testing::*;

    /// Deleting a LUT asks first, saying how many photos use it.
    #[test]
    fn lut_usage_counts_the_photos_whose_look_uses_it() {
        let conn = open_memory().unwrap();
        for id in 1..=3 {
            photo_at(&conn, id, &format!("/p/{id}.arw"));
        }
        let mut stack = EditStack::default();
        stack.lut.name = "Film.cube".into();
        set_develop_settings(&conn, 1, &stack).unwrap();
        set_develop_settings(&conn, 2, &stack).unwrap();
        stack.lut.name = String::new();
        set_develop_settings(&conn, 3, &stack).unwrap();
        assert_eq!(lut_usage(&conn, "Film.cube").unwrap(), 2);
        assert_eq!(lut_usage(&conn, "Other.cube").unwrap(), 0);
    }

    /// A render as edited is kept only for the edits it was made from, and
    /// forgotten when they change, whoever changes them.
    #[test]
    fn edited_render_goes_stale_when_the_edits_change() {
        let conn = open_memory().unwrap();
        photo_at(&conn, 1, "/p/1.arw");
        let path = |conn: &Connection| -> Option<String> {
            conn.query_row("SELECT edited_path FROM photos WHERE id = 1", [], |r| {
                r.get(0)
            })
            .unwrap()
        };
        let mut stack = EditStack {
            exposure: 0.5,
            ..Default::default()
        };
        set_develop_settings(&conn, 1, &stack).unwrap();
        let settings = stack.to_json().unwrap();
        set_edited_path(&conn, 1, &settings, "/t/1_a.jpg").unwrap();
        assert_eq!(path(&conn).as_deref(), Some("/t/1_a.jpg"));

        // Saved again unchanged, as Develop does: still current.
        set_develop_settings(&conn, 1, &stack).unwrap();
        assert_eq!(path(&conn).as_deref(), Some("/t/1_a.jpg"));

        stack.exposure = 1.0;
        set_develop_settings(&conn, 1, &stack).unwrap();
        assert_eq!(path(&conn), None);
        // A render of the old edits, finishing late, is not kept.
        set_edited_path(&conn, 1, &settings, "/t/1_a.jpg").unwrap();
        assert_eq!(path(&conn), None);
    }

    fn photo_row(conn: &Connection) -> i32 {
        conn.execute(
            "INSERT INTO photos (path, filename, file_hash, indexed_at)
             VALUES ('/a.arw', 'a.arw', 'h', '2026-01-01')",
            [],
        )
        .unwrap();
        conn.last_insert_rowid() as i32
    }

    #[test]
    fn an_undeveloped_photo_has_no_stack() {
        let conn = open_memory().unwrap();
        let id = photo_row(&conn);
        assert!(get_develop_settings(&conn, id).unwrap().is_none());
    }

    #[test]
    fn a_stack_round_trips_through_the_database() {
        let conn = open_memory().unwrap();
        let id = photo_row(&conn);

        let stack = EditStack {
            exposure: -1.25,
            warmth: 0.4,
            ..Default::default()
        };
        set_develop_settings(&conn, id, &stack).unwrap();
        assert_eq!(get_develop_settings(&conn, id).unwrap(), Some(stack));
    }

    /// Resetting every slider must clear the stored stack rather than write a
    /// default-valued one, so the library can still tell edited from untouched.
    #[test]
    fn resetting_to_defaults_clears_the_stored_stack() {
        let conn = open_memory().unwrap();
        let id = photo_row(&conn);

        set_develop_settings(
            &conn,
            id,
            &EditStack {
                exposure: 1.0,
                ..Default::default()
            },
        )
        .unwrap();
        assert!(get_develop_settings(&conn, id).unwrap().is_some());

        set_develop_settings(&conn, id, &EditStack::default()).unwrap();
        assert!(get_develop_settings(&conn, id).unwrap().is_none());
    }

    fn sample_stack() -> EditStack {
        EditStack {
            exposure: 0.8,
            contrast: 0.25,
            ..Default::default()
        }
    }

    #[test]
    fn presets_round_trip() {
        let conn = open_memory().unwrap();
        assert!(list_presets(&conn).unwrap().is_empty());

        let id = save_preset(&conn, "Punchy", &sample_stack(), None).unwrap();
        let presets = list_presets(&conn).unwrap();
        assert_eq!(presets.len(), 1);
        assert_eq!(presets[0].id, id);
        assert_eq!(presets[0].name, "Punchy");
        assert_eq!(presets[0].stack, sample_stack());

        delete_preset(&conn, id).unwrap();
        assert!(list_presets(&conn).unwrap().is_empty());
    }

    /// Saving under an existing name replaces it rather than erroring or
    /// silently creating a duplicate — the behaviour "save preset" implies.
    #[test]
    fn resaving_a_preset_name_overwrites_it() {
        let conn = open_memory().unwrap();
        let first = save_preset(&conn, "Look", &sample_stack(), None).unwrap();
        let second = save_preset(
            &conn,
            "Look",
            &EditStack {
                exposure: -1.0,
                ..Default::default()
            },
            None,
        )
        .unwrap();

        assert_eq!(first, second, "the row should be reused");
        let presets = list_presets(&conn).unwrap();
        assert_eq!(presets.len(), 1);
        assert_eq!(presets[0].stack.exposure, -1.0);
    }

    /// A preset can carry only some parts of an edit, and applies just
    /// those; one saved before presets had parts applies whole, as it did.
    #[test]
    fn a_preset_carries_the_parts_it_was_saved_with() {
        use crate::develop::stack::Group;
        let conn = open_memory().unwrap();
        let look = EditStack {
            exposure: 0.7,
            vibrance: 0.4,
            ..Default::default()
        };
        save_preset(&conn, "Colour only", &look, Some(&[Group::Color])).unwrap();
        save_preset(&conn, "Whole", &look, None).unwrap();
        let presets = list_presets(&conn).unwrap();
        let colour = presets.iter().find(|p| p.name == "Colour only").unwrap();
        let whole = presets.iter().find(|p| p.name == "Whole").unwrap();
        assert_eq!(colour.groups, Some(vec![Group::Color]));
        assert_eq!(whole.groups, None);

        let mine = EditStack {
            exposure: -0.3,
            ..Default::default()
        };
        let applied = colour.applied_to(&mine);
        assert_eq!((applied.exposure, applied.vibrance), (-0.3, 0.4));
        assert_eq!(whole.applied_to(&mine), look, "an old preset applies whole");
    }

    /// Erases fill one photo's own pixels: a preset is saved without them,
    /// and even one that applies whole leaves the photo's own in place.
    #[test]
    fn a_preset_never_carries_erases() {
        use crate::develop::erase::{Erase, Piece};
        let conn = open_memory().unwrap();
        let erase = |label: &str| Erase {
            label: label.into(),
            pieces: vec![Piece {
                fill: "c".repeat(64),
                x: 0,
                y: 0,
                size: 512,
            }],
        };
        let look = EditStack {
            exposure: 0.7,
            erases: vec![erase("theirs")],
            ..Default::default()
        };
        save_preset(&conn, "Whole", &look, None).unwrap();
        let whole = list_presets(&conn).unwrap().pop().unwrap();
        assert!(whole.stack.erases.is_empty());
        let mine = EditStack {
            erases: vec![erase("mine")],
            ..Default::default()
        };
        let applied = whole.applied_to(&mine);
        assert_eq!(applied.exposure, 0.7);
        assert_eq!(applied.erases, mine.erases);
    }

    /// The fills a catalogue's edits name, for the sweep of unused ones.
    #[test]
    fn the_fills_in_use_are_read_from_every_edit() {
        use crate::develop::erase::{Erase, Piece};
        let dir = crate::test_support::Scratch::new("db-fills-in-use");
        let path = dir.join("library.db");
        let conn = open(&path).unwrap();
        conn.execute_batch(
            "INSERT INTO photos (id, path, filename, file_hash, indexed_at)
             VALUES (1, 'a.arw', 'a.arw', 'h1', 'now'), (2, 'b.arw', 'b.arw', 'h2', 'now');",
        )
        .unwrap();
        let piece = |c: &str| Piece {
            fill: c.repeat(64),
            x: 0,
            y: 0,
            size: 512,
        };
        let erased = EditStack {
            erases: vec![Erase {
                label: "x".into(),
                pieces: vec![piece("a"), piece("b")],
            }],
            ..Default::default()
        };
        set_develop_settings(&conn, 1, &erased).unwrap();
        set_develop_settings(
            &conn,
            2,
            &EditStack {
                exposure: 0.3,
                ..Default::default()
            },
        )
        .unwrap();
        drop(conn);
        let names = erase_fills_in(&path).unwrap();
        assert_eq!(names, ["a".repeat(64), "b".repeat(64)].into());
        assert!(erase_fills_in(&dir.join("absent.db")).is_err());
    }

    /// Copying to a selection takes only the chosen parts, onto each photo's
    /// own edit — a photo's crop stays its own.
    #[test]
    fn copying_parts_to_photos_keeps_the_rest_of_their_edits() {
        use crate::develop::stack::Group;
        let mut conn = open_memory().unwrap();
        conn.execute_batch(
            "INSERT INTO photos (id, path, filename, file_hash, indexed_at)
             VALUES (1, 'a.arw', 'a.arw', 'h1', 'now'), (2, 'b.arw', 'b.arw', 'h2', 'now');",
        )
        .unwrap();
        let mut cropped = EditStack::default();
        cropped.geometry.straighten = 2.0;
        set_develop_settings(&conn, 2, &cropped).unwrap();

        let mut source = EditStack {
            exposure: 0.5,
            ..Default::default()
        };
        source.geometry.straighten = -4.0;
        let defaults: Vec<Group> = Group::ALL
            .into_iter()
            .filter(|g| g.copied_by_default())
            .collect();
        assert_eq!(
            apply_groups_to(&mut conn, &[1, 2], &source, &defaults).unwrap(),
            2
        );

        let one = get_develop_settings(&conn, 1).unwrap().unwrap();
        let two = get_develop_settings(&conn, 2).unwrap().unwrap();
        assert_eq!((one.exposure, one.geometry.straighten), (0.5, 0.0));
        assert_eq!(
            (two.exposure, two.geometry.straighten),
            (0.5, 2.0),
            "its own crop kept"
        );
    }

    #[test]
    fn an_unnamed_preset_is_refused() {
        let conn = open_memory().unwrap();
        assert!(save_preset(&conn, "   ", &sample_stack(), None).is_err());
    }

    /// One corrupt row must not take the whole preset list down with it.
    #[test]
    fn an_unparseable_preset_is_skipped_not_fatal() {
        let conn = open_memory().unwrap();
        save_preset(&conn, "Good", &sample_stack(), None).unwrap();
        conn.execute(
            "INSERT INTO presets(name, settings, created_at) VALUES('Broken', '{not json', '')",
            [],
        )
        .unwrap();

        let presets = list_presets(&conn).unwrap();
        assert_eq!(presets.len(), 1);
        assert_eq!(presets[0].name, "Good");
    }

    #[test]
    fn a_stack_can_be_applied_across_a_selection() {
        let mut conn = open_memory().unwrap();
        let ids: Vec<i32> = (0..3)
            .map(|i| {
                conn.execute(
                    "INSERT INTO photos (path, filename, file_hash, indexed_at)
                     VALUES (?1, 'a.arw', 'h', '2026-01-01')",
                    params![format!("/photo{i}.arw")],
                )
                .unwrap();
                conn.last_insert_rowid() as i32
            })
            .collect();

        let n = apply_stack_to(&mut conn, &ids, &sample_stack()).unwrap();
        assert_eq!(n, 3);
        for id in &ids {
            assert_eq!(
                get_develop_settings(&conn, *id).unwrap(),
                Some(sample_stack())
            );
        }

        // Applying a default stack clears them again, keeping "edited" and
        // "untouched" distinguishable in bulk too.
        apply_stack_to(&mut conn, &ids, &EditStack::default()).unwrap();
        for id in &ids {
            assert!(get_develop_settings(&conn, *id).unwrap().is_none());
        }
    }

    /// Copied settings put back as each photo had them: its own edit,
    /// or none at all.
    #[test]
    fn develop_settings_are_restored_as_they_were() {
        let mut conn = open_memory().unwrap();
        add_photos(&conn, 2);
        let own = EditStack {
            exposure: 0.7,
            ..Default::default()
        };
        set_develop_settings(&conn, 1, &own).unwrap();
        let before = vec![
            (1, get_develop_settings(&conn, 1).unwrap()),
            (2, get_develop_settings(&conn, 2).unwrap()),
        ];
        let copied = EditStack {
            exposure: -1.0,
            ..Default::default()
        };
        apply_groups_to(
            &mut conn,
            &[1, 2],
            &copied,
            &[crate::develop::stack::Group::Light],
        )
        .unwrap();
        assert_ne!(get_develop_settings(&conn, 1).unwrap(), Some(own.clone()));

        restore_develop_settings(&mut conn, &before).unwrap();
        assert_eq!(get_develop_settings(&conn, 1).unwrap(), Some(own));
        assert_eq!(get_develop_settings(&conn, 2).unwrap(), None);
    }
}
