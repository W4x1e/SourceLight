//! Settings, kept as key-value pairs, and the names of tags in each
//! language.

use super::*;

pub fn get_setting(conn: &Connection, key: &str) -> Result<Option<String>> {
    let mut stmt = conn.prepare("SELECT value FROM settings WHERE key = ?1")?;
    let mut rows = stmt.query(params![key])?;
    Ok(match rows.next()? {
        Some(r) => Some(r.get(0)?),
        None => None,
    })
}

/// Every tag name known in `language`: English name → the name shown.
pub fn tag_translations(
    conn: &Connection,
    language: &str,
) -> Result<std::collections::HashMap<String, String>> {
    let mut stmt =
        conn.prepare("SELECT name, translation FROM tag_translations WHERE language = ?1")?;
    let rows = stmt.query_map(params![language], |r| Ok((r.get(0)?, r.get(1)?)))?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// Keeps tag names in `language`, replacing any already kept.
pub fn set_tag_translations(
    conn: &mut Connection,
    language: &str,
    names: &[(String, String)],
) -> Result<()> {
    let tx = conn.transaction()?;
    {
        let mut stmt = tx.prepare(
            "INSERT INTO tag_translations(name, language, translation) VALUES(?1, ?2, ?3)
             ON CONFLICT(name, language) DO UPDATE SET translation = excluded.translation",
        )?;
        for (name, translation) in names {
            stmt.execute(params![name, language, translation])?;
        }
    }
    tx.commit()?;
    Ok(())
}

pub fn set_setting(conn: &Connection, key: &str, value: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO settings(key, value) VALUES(?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![key, value],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tag_names_are_kept_per_language_and_replaced() {
        let mut conn = open_memory().unwrap();
        set_tag_translations(
            &mut conn,
            "de",
            &[("misty glow".into(), "Nebelschein".into())],
        )
        .unwrap();
        set_tag_translations(
            &mut conn,
            "fr",
            &[("misty glow".into(), "lueur brumeuse".into())],
        )
        .unwrap();
        set_tag_translations(
            &mut conn,
            "de",
            &[("misty glow".into(), "Nebelglanz".into())],
        )
        .unwrap();
        let de = tag_translations(&conn, "de").unwrap();
        assert_eq!(de.len(), 1);
        assert_eq!(de["misty glow"], "Nebelglanz");
        assert_eq!(
            tag_translations(&conn, "fr").unwrap()["misty glow"],
            "lueur brumeuse"
        );
        assert!(tag_translations(&conn, "ja").unwrap().is_empty());
    }
}
