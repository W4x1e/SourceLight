//! A photo's own title, caption and rights, and the templates that
//! fill them in.

use super::*;

/// Each field and the column it is kept in.
const IPTC_COLUMNS: [(IptcField, &str); 6] = [
    (IptcField::Title, "title"),
    (IptcField::Caption, "caption"),
    (IptcField::Creator, "creator"),
    (IptcField::Copyright, "copyright"),
    (IptcField::Credit, "credit"),
    (IptcField::UsageTerms, "usage_terms"),
];

fn iptc_column(field: IptcField) -> &'static str {
    IPTC_COLUMNS
        .iter()
        .find(|(f, _)| *f == field)
        .map(|(_, c)| *c)
        .unwrap_or("title")
}

impl IptcFields {
    pub fn get(&self, field: IptcField) -> Option<&String> {
        match field {
            IptcField::Title => self.title.as_ref(),
            IptcField::Caption => self.caption.as_ref(),
            IptcField::Creator => self.creator.as_ref(),
            IptcField::Copyright => self.copyright.as_ref(),
            IptcField::Credit => self.credit.as_ref(),
            IptcField::UsageTerms => self.usage_terms.as_ref(),
        }
    }

    fn slot(&mut self, field: IptcField) -> &mut Option<String> {
        match field {
            IptcField::Title => &mut self.title,
            IptcField::Caption => &mut self.caption,
            IptcField::Creator => &mut self.creator,
            IptcField::Copyright => &mut self.copyright,
            IptcField::Credit => &mut self.credit,
            IptcField::UsageTerms => &mut self.usage_terms,
        }
    }

    /// Every field trimmed, and blank ones taken as none.
    pub fn cleaned(mut self) -> Self {
        for (f, _) in IPTC_COLUMNS {
            let v = self.slot(f).take();
            *self.slot(f) = clean_text(v.as_deref());
        }
        self
    }

    pub fn is_empty(&self) -> bool {
        IPTC_COLUMNS.iter().all(|(f, _)| self.get(*f).is_none())
    }
}

fn clean_text(v: Option<&str>) -> Option<String> {
    v.map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

fn iptc_from_row(r: &rusqlite::Row, first: usize) -> rusqlite::Result<IptcFields> {
    Ok(IptcFields {
        title: r.get(first)?,
        caption: r.get(first + 1)?,
        creator: r.get(first + 2)?,
        copyright: r.get(first + 3)?,
        credit: r.get(first + 4)?,
        usage_terms: r.get(first + 5)?,
    })
}

/// One photo's own fields; all `None` when it has none.
pub fn photo_iptc(conn: &Connection, photo_id: i32) -> Result<IptcFields> {
    Ok(conn
        .query_row(
            "SELECT title, caption, creator, copyright, credit, usage_terms
               FROM photo_iptc WHERE photo_id = ?1",
            params![photo_id],
            |r| iptc_from_row(r, 0),
        )
        .optional()?
        .unwrap_or_default())
}

/// The fields across these photos: each one they share, or that they differ.
pub fn iptc_selection(conn: &Connection, ids: &[i32]) -> Result<IptcSelection> {
    let mut each = Vec::with_capacity(ids.len());
    for id in ids {
        each.push(photo_iptc(conn, *id)?);
    }
    let shared = |f: IptcField| -> SharedField {
        let mut values = each.iter().map(|p| p.get(f));
        let first = values.next().flatten().cloned();
        let mixed = each.iter().any(|p| p.get(f) != first.as_ref());
        SharedField {
            value: if mixed { None } else { first },
            mixed,
        }
    };
    Ok(IptcSelection {
        title: shared(IptcField::Title),
        caption: shared(IptcField::Caption),
        creator: shared(IptcField::Creator),
        copyright: shared(IptcField::Copyright),
        credit: shared(IptcField::Credit),
        usage_terms: shared(IptcField::UsageTerms),
    })
}

/// Sets one field on these photos — cleared with a blank — and says on how
/// many. A photo left with none is dropped from the table.
pub fn set_iptc_field(
    conn: &mut Connection,
    ids: &[i32],
    field: IptcField,
    value: Option<&str>,
) -> Result<usize> {
    let value = clean_text(value);
    let col = iptc_column(field);
    let tx = conn.transaction()?;
    {
        let mut set = tx.prepare(&format!(
            "INSERT INTO photo_iptc (photo_id, {col}) VALUES (?1, ?2)
             ON CONFLICT(photo_id) DO UPDATE SET {col} = excluded.{col}"
        ))?;
        for id in ids {
            set.execute(params![id, value])?;
        }
    }
    drop_empty_iptc(&tx, ids)?;
    tx.commit()?;
    Ok(ids.len())
}

/// Puts every field `fields` has on these photos, leaving the others as they
/// are — a template, or what a sidecar says. How many photos it touched.
pub fn apply_iptc(conn: &Connection, ids: &[i32], fields: &IptcFields) -> Result<usize> {
    let fields = fields.clone().cleaned();
    if fields.is_empty() {
        return Ok(0);
    }
    let set: Vec<(&str, &String)> = IPTC_COLUMNS
        .iter()
        .filter_map(|(f, c)| fields.get(*f).map(|v| (*c, v)))
        .collect();
    let cols: Vec<&str> = set.iter().map(|(c, _)| *c).collect();
    let sql = format!(
        "INSERT INTO photo_iptc (photo_id, {}) VALUES (?1, {})
         ON CONFLICT(photo_id) DO UPDATE SET {}",
        cols.join(", "),
        (2..cols.len() + 2)
            .map(|i| format!("?{i}"))
            .collect::<Vec<_>>()
            .join(", "),
        cols.iter()
            .map(|c| format!("{c} = excluded.{c}"))
            .collect::<Vec<_>>()
            .join(", "),
    );
    let mut stmt = conn.prepare(&sql)?;
    for id in ids {
        let mut values: Vec<&dyn rusqlite::ToSql> = vec![id];
        values.extend(set.iter().map(|(_, v)| *v as &dyn rusqlite::ToSql));
        stmt.execute(values.as_slice())?;
    }
    Ok(ids.len())
}

fn drop_empty_iptc(conn: &Connection, ids: &[i32]) -> Result<()> {
    let mut stmt = conn.prepare(
        "DELETE FROM photo_iptc WHERE photo_id = ?1
            AND COALESCE(title, caption, creator, copyright, credit, usage_terms) IS NULL",
    )?;
    for id in ids {
        stmt.execute(params![id])?;
    }
    Ok(())
}

/// The templates, by name.
pub fn list_iptc_templates(conn: &Connection) -> Result<Vec<IptcTemplate>> {
    let mut stmt = conn.prepare(
        "SELECT id, name, title, caption, creator, copyright, credit, usage_terms
           FROM iptc_templates ORDER BY name COLLATE NOCASE",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok(IptcTemplate {
            id: r.get(0)?,
            name: r.get(1)?,
            fields: iptc_from_row(r, 2)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

/// Saves a template — a new one without `id` — and returns its id. Names are
/// unique, whatever their case.
pub fn save_iptc_template(
    conn: &Connection,
    id: Option<i32>,
    name: &str,
    fields: &IptcFields,
) -> Result<i32> {
    let name = name.trim();
    anyhow::ensure!(!name.is_empty(), "a template needs a name");
    let f = fields.clone().cleaned();
    let clash: Option<i32> = conn
        .query_row(
            "SELECT id FROM iptc_templates WHERE name = ?1 COLLATE NOCASE",
            params![name],
            |r| r.get(0),
        )
        .optional()?;
    anyhow::ensure!(
        clash.is_none() || clash == id,
        "there's already a template called “{name}”"
    );
    let values = params![
        name,
        f.title,
        f.caption,
        f.creator,
        f.copyright,
        f.credit,
        f.usage_terms
    ];
    match id {
        Some(id) => {
            let mut all: Vec<&dyn rusqlite::ToSql> = values.to_vec();
            all.push(&id);
            conn.execute(
                "UPDATE iptc_templates SET name = ?1, title = ?2, caption = ?3, creator = ?4,
                        copyright = ?5, credit = ?6, usage_terms = ?7 WHERE id = ?8",
                all.as_slice(),
            )?;
            Ok(id)
        }
        None => {
            conn.execute(
                "INSERT INTO iptc_templates (name, title, caption, creator, copyright, credit, usage_terms)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                values,
            )?;
            Ok(conn.last_insert_rowid() as i32)
        }
    }
}

pub fn delete_iptc_template(conn: &Connection, id: i32) -> Result<()> {
    conn.execute("DELETE FROM iptc_templates WHERE id = ?1", params![id])?;
    Ok(())
}

/// Puts a template's fields on these photos; how many.
pub fn apply_iptc_template(conn: &mut Connection, ids: &[i32], template: i32) -> Result<usize> {
    let fields = list_iptc_templates(conn)?
        .into_iter()
        .find(|t| t.id == template)
        .ok_or_else(|| anyhow::anyhow!("that template has gone"))?
        .fields;
    let tx = conn.transaction()?;
    let n = apply_iptc(&tx, ids, &fields)?;
    tx.commit()?;
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::testing::*;

    fn iptc(title: Option<&str>, copyright: Option<&str>) -> IptcFields {
        IptcFields {
            title: title.map(String::from),
            copyright: copyright.map(String::from),
            ..Default::default()
        }
    }

    #[test]
    fn a_field_set_on_a_selection_reads_back_shared_or_mixed() {
        let mut conn = open_memory().unwrap();
        add_photos(&conn, 3);
        set_iptc_field(
            &mut conn,
            &[1, 2, 3],
            IptcField::Copyright,
            Some(" © 2025 Ada "),
        )
        .unwrap();
        set_iptc_field(&mut conn, &[1], IptcField::Title, Some("Dusk")).unwrap();
        let sel = iptc_selection(&conn, &[1, 2, 3]).unwrap();
        assert_eq!(
            sel.copyright,
            SharedField {
                value: Some("© 2025 Ada".into()),
                mixed: false
            }
        );
        assert_eq!(
            sel.title,
            SharedField {
                value: None,
                mixed: true
            }
        );
        assert_eq!(sel.credit, SharedField::default());

        // A blank clears it; a photo left with nothing leaves the table.
        set_iptc_field(&mut conn, &[2, 3], IptcField::Copyright, Some("  ")).unwrap();
        assert_eq!(photo_iptc(&conn, 2).unwrap(), IptcFields::default());
        let rows: i64 = conn
            .query_row("SELECT COUNT(*) FROM photo_iptc", [], |r| r.get(0))
            .unwrap();
        assert_eq!(rows, 1);
    }

    #[test]
    fn a_template_fills_only_the_fields_it_has() {
        let mut conn = open_memory().unwrap();
        add_photos(&conn, 2);
        set_iptc_field(&mut conn, &[1], IptcField::Title, Some("Mine")).unwrap();
        let t = save_iptc_template(
            &conn,
            None,
            "Agency",
            &IptcFields {
                creator: Some("Ada Lovelace".into()),
                copyright: Some("© 2025 Ada Lovelace".into()),
                credit: Some("Northern Light".into()),
                title: Some("   ".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(save_iptc_template(&conn, None, "agency", &IptcFields::default()).is_err());
        assert!(save_iptc_template(&conn, None, " ", &IptcFields::default()).is_err());

        assert_eq!(apply_iptc_template(&mut conn, &[1, 2], t).unwrap(), 2);
        let one = photo_iptc(&conn, 1).unwrap();
        assert_eq!(
            one.title.as_deref(),
            Some("Mine"),
            "a blank template field leaves it alone"
        );
        assert_eq!(one.creator.as_deref(), Some("Ada Lovelace"));
        assert_eq!(
            photo_iptc(&conn, 2).unwrap().credit.as_deref(),
            Some("Northern Light")
        );

        save_iptc_template(&conn, Some(t), "Agency 2026", &iptc(None, Some("© 2026"))).unwrap();
        let listed = list_iptc_templates(&conn).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].name, "Agency 2026");
        assert_eq!(listed[0].fields.creator, None);
        delete_iptc_template(&conn, t).unwrap();
        assert!(apply_iptc_template(&mut conn, &[1], t).is_err());
    }

    #[test]
    fn a_title_or_caption_someone_wrote_is_found_first() {
        let mut conn = open_memory().unwrap();
        add_photos(&conn, 3);
        set_caption(&conn, 1, Some("a lighthouse on a cliff")).unwrap();
        set_iptc_field(
            &mut conn,
            &[2],
            IptcField::Title,
            Some("Lighthouse at dusk"),
        )
        .unwrap();
        set_iptc_field(
            &mut conn,
            &[3],
            IptcField::Caption,
            Some("The old lighthouse, 1890"),
        )
        .unwrap();
        let found = caption_search(&conn, "lighthouse", 10).unwrap();
        assert_eq!(found.len(), 3);
        assert_eq!(
            found[2], 1,
            "the model's caption after the user's words: {found:?}"
        );

        // A changed or cleared field leaves the index with it.
        set_iptc_field(&mut conn, &[2], IptcField::Title, None).unwrap();
        set_iptc_field(&mut conn, &[3], IptcField::Caption, Some("The old lamp")).unwrap();
        assert_eq!(caption_search(&conn, "lighthouse", 10).unwrap(), vec![1]);
        assert_eq!(caption_search(&conn, "lamp", 10).unwrap(), vec![3]);
    }

    #[test]
    fn a_virtual_copy_keeps_the_rights_but_not_the_title() {
        let mut conn = open_memory().unwrap();
        add_photos(&conn, 1);
        set_iptc_field(&mut conn, &[1], IptcField::Title, Some("Dusk")).unwrap();
        set_iptc_field(&mut conn, &[1], IptcField::Copyright, Some("© Ada")).unwrap();
        let copy = create_variant(&mut conn, 1, None).unwrap();
        assert_eq!(photo_iptc(&conn, copy).unwrap(), iptc(None, Some("© Ada")));
    }

    #[test]
    fn title_caption_and_rights_round_trip_through_a_sidecar() {
        let mut conn = open_memory().unwrap();
        add_photos(&conn, 1);
        set_caption(&conn, 1, Some("the model's words")).unwrap();
        for (f, v) in [
            (IptcField::Title, "Dusk at the Point"),
            (IptcField::Caption, "Our first night at the Point"),
            (IptcField::Creator, "Ada Lovelace"),
            (IptcField::Copyright, "© 2025 Ada Lovelace"),
            (IptcField::Credit, "Northern Light"),
            (IptcField::UsageTerms, "Editorial use only"),
        ] {
            set_iptc_field(&mut conn, &[1], f, Some(v)).unwrap();
        }
        let meta = xmp_meta(&conn, &photo_of(&conn, 1)).unwrap();
        assert!(meta.description_is_own);
        assert_eq!(
            meta.description.as_deref(),
            Some("Our first night at the Point")
        );
        let packet = crate::xmp::write(&meta);

        // Read into a catalogue that has never seen the photo: every field is
        // back, as the photo's own — none of it counts as this app's to skip,
        // since that one wrote it.
        let mut fresh = open_memory().unwrap();
        add_photos(&fresh, 1);
        let mut sidecar = crate::xmp::read(&packet).unwrap();
        assert!(sidecar.description_is_ours && sidecar.text_from_elsewhere == Default::default());
        // As though another editor wrote it: a person's caption and rights.
        sidecar.description_is_ours = false;
        sidecar.text_from_elsewhere = sidecar.meta.text.clone();
        import_xmp_meta(&mut fresh, 1, &sidecar).unwrap();
        let back = photo_iptc(&fresh, 1).unwrap();
        assert_eq!(back.title.as_deref(), Some("Dusk at the Point"));
        assert_eq!(
            back.caption.as_deref(),
            Some("Our first night at the Point")
        );
        assert_eq!(back.copyright.as_deref(), Some("© 2025 Ada Lovelace"));
        assert_eq!(back.usage_terms.as_deref(), Some("Editorial use only"));
        assert_eq!(
            photo_of(&fresh, 1).caption,
            None,
            "a person's caption isn't the model's"
        );
    }

    #[test]
    fn an_export_takes_the_photos_own_rights_over_the_settings() {
        let mut conn = open_memory().unwrap();
        add_photos(&conn, 2);
        set_setting(&conn, crate::export::exif::CREATOR_KEY, "Studio").unwrap();
        set_setting(&conn, crate::export::exif::COPYRIGHT_KEY, "© Studio").unwrap();
        set_iptc_field(&mut conn, &[1], IptcField::Creator, Some("Ada Lovelace")).unwrap();
        set_iptc_field(&mut conn, &[1], IptcField::Title, Some("Dusk")).unwrap();
        let own = crate::export::exif::ExportMetadata::load(&conn, 1)
            .unwrap()
            .unwrap();
        assert_eq!(own.creator.as_deref(), Some("Ada Lovelace"));
        assert_eq!(
            own.copyright.as_deref(),
            Some("© Studio"),
            "none of its own: the default"
        );
        assert_eq!(own.title.as_deref(), Some("Dusk"));
        let plain = crate::export::exif::ExportMetadata::load(&conn, 2)
            .unwrap()
            .unwrap();
        assert_eq!(plain.creator.as_deref(), Some("Studio"));
    }
}
