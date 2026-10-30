//! The catalogue: one SQLite database holding every photo and everything
//! known about it.
//!
//! This file opens the database and keeps the helpers the other files share.
//! The queries live in one file per area. Each file's items are re-exported
//! here, so callers keep writing `db::name`.

use anyhow::{bail, Result};
use rusqlite::types::Value;
use rusqlite::{params, Connection, OptionalExtension, Row};
use std::path::Path;

use crate::develop::EditStack;
use crate::models::*;

mod capture_time;
mod collections;
mod colours;
mod cull;
mod develop;
mod embeddings;
mod faces;
mod facets;
mod folders;
mod insights;
mod iptc;
mod keywords;
mod migrations;
mod people;
mod people_edits;
mod phone;
mod photos;
mod places;
mod query;
mod search;
mod settings;
mod shoots;
mod sidecars;
mod tags;
#[cfg(test)]
mod testing;

use migrations::*;

pub use capture_time::*;
pub use collections::*;
pub use colours::*;
pub use cull::*;
pub use develop::*;
pub use embeddings::*;
pub use faces::*;
pub use facets::*;
pub use folders::*;
pub use insights::*;
pub use iptc::*;
pub use keywords::*;
pub use people::*;
pub use people_edits::*;
pub use phone::*;
pub use photos::*;
pub use places::*;
pub use query::*;
pub use search::*;
pub use settings::*;
pub use shoots::*;
pub use sidecars::*;
pub use tags::*;

/// Opens and migrates. Called once at startup.
///
/// Two things happen here that do not happen on a per-command `connect`, and
/// both are about the moments where the catalogue is at risk.
///
/// A restore staged by the user is applied *first*, before anything opens the
/// file — swapping a database out from under a live SQLite handle is how a
/// recoverable mistake becomes a corrupt one.
///
/// Then, if this build is about to migrate the schema forward, a snapshot is
/// taken and kept. A migration rewrites tables in place (11 rebuilds `photos`
/// wholesale), and the library database is the only copy of every rating, tag,
/// place, person and develop edit the user has ever made. Taking the copy is
/// cheap; not having it is unrecoverable.
pub fn open(path: &Path) -> Result<Connection> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let restored = crate::backup::take_pending(path)?;

    let conn = connect(path)?;

    let current: u32 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    let target = MIGRATIONS.len() as u32;
    if current > 0 && current < target {
        // `current > 0` skips a database being created right now — there is
        // nothing yet to lose, and an empty backup is just noise in the list.
        //
        // A failure here is reported and does not stop the migration. The
        // alternative is refusing to start because a *precaution* failed, which
        // would strand a user whose disk is merely full behind a locked door.
        match crate::backup::create(&conn, path, &format!("auto-v{current}")) {
            Ok(at) => {
                eprintln!(
                    "catalogue backed up before upgrading to v{target}: {}",
                    at.display()
                );
                let _ = crate::backup::prune(path);
            }
            Err(e) => eprintln!("could not back up the catalogue before migrating: {e:#}"),
        }
    }

    migrate(&conn)?;
    // Migrations 45 and 46 took the vectors out of the photo and face rows.
    // Until the file is rebuilt the pages they filled are only free space,
    // which about doubled it: rebuilt once, on the upgrade that moved them.
    // About 1.4 s at 50,000 photos. A failure leaves a larger file, nothing
    // worse.
    if current > 0 && EMBEDDINGS_MOVED.iter().any(|&m| current < m && target >= m) {
        if let Err(e) = conn.execute_batch("VACUUM") {
            eprintln!("could not compact the catalogue after moving its search vectors: {e:#}");
        }
    }
    forget_retired_settings(&conn)?;
    repair_drive_roots(&conn)?;
    // A restored catalogue can name previews deleted since it was backed up:
    // the next scan makes them again.
    if restored {
        if let Err(e) = rederive_missing(&conn) {
            eprintln!("could not check the restored catalogue's previews: {e:#}");
        }
    }
    // An optimisation, not a precaution: a catalogue that cannot be analysed
    // right now is still perfectly usable, just slower to filter.
    if let Err(e) = refresh_planner_stats(&conn) {
        eprintln!("could not refresh query-planner statistics: {e:#}");
    }
    Ok(conn)
}

/// Opens the catalogue like [`open`], and if it turns out to be damaged,
/// recovers: the damaged file is kept aside and the newest good backup takes
/// its place (see [`crate::backup::recover_damaged`]).
///
/// Before this, a damaged catalogue either stopped the app at startup ("Failed
/// to setup app: database disk image is malformed") or, when the damage was
/// past the first pages, let it open with every query failing and an empty
/// grid, while the good backup sat unused beside it.
pub fn open_or_recover(path: &Path) -> Result<(Connection, Option<crate::backup::Recovery>)> {
    let attempt = open(path).and_then(|conn| {
        schema_readable(&conn)?;
        Ok(conn)
    });
    // Failed backups from older builds were empty files; clear them out.
    let _ = crate::backup::prune(path);
    match attempt {
        Ok(conn) => Ok((conn, None)),
        Err(e) if is_damage(&e) => {
            eprintln!("catalogue is damaged ({e:#}); restoring the newest good backup");
            let recovery = crate::backup::recover_damaged(path)?;
            Ok((open(path)?, Some(recovery)))
        }
        Err(e) => Err(e),
    }
}

/// Reads the whole schema: where this catalogue's damage was, and cheap.
fn schema_readable(conn: &Connection) -> Result<()> {
    conn.query_row("SELECT count(*) FROM sqlite_master", [], |r| {
        r.get::<_, i64>(0)
    })?;
    conn.query_row("SELECT count(*) FROM photos", [], |r| r.get::<_, i64>(0))?;
    Ok(())
}

/// Whether an error says the file itself is damaged, as opposed to busy,
/// missing or full.
pub fn is_damage(e: &anyhow::Error) -> bool {
    e.chain().any(|cause| {
        matches!(
            cause.downcast_ref::<rusqlite::Error>(),
            Some(rusqlite::Error::SqliteFailure(f, _))
                if matches!(f.code, rusqlite::ErrorCode::DatabaseCorrupt | rusqlite::ErrorCode::NotADatabase)
        )
    })
}

/// A full consistency check, for the background check after startup. `Ok`
/// with SQLite's first complaint when the catalogue is damaged.
pub fn quick_check(path: &Path) -> Result<Option<String>> {
    let conn = Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let verdict: String = conn.query_row("PRAGMA quick_check(1)", [], |r| r.get(0))?;
    Ok((verdict != "ok").then_some(verdict))
}

/// Settings the app no longer reads, deleted on every open so a stale value
/// cannot sit in the catalogue looking as if it still mattered. Idempotent and
/// cheap, and not a migration: removing a key changes no schema, and a
/// migration would cost every catalogue an automatic backup for nothing.
///
/// - `ai_base_url` — the remote Ollama address, since removed: the app now
///   talks only to the runtime it manages, on a port of its own.
fn forget_retired_settings(conn: &Connection) -> Result<()> {
    conn.execute("DELETE FROM settings WHERE key IN ('ai_base_url')", [])?;
    Ok(())
}

/// Keeps the query planner's statistics current.
///
/// SQLite picks between indexes using row counts from `sqlite_stat1`, and until
/// this existed nothing ever wrote them — so every real catalogue was planned on
/// SQLite's fixed default guesses, while the benchmark, whose seeder ran a full
/// `ANALYZE`, measured plans users never got. At 50,000 photos that gap made a
/// single-genre click in the filter rail about four times slower in the app than
/// in the benchmark. The seeder now calls this instead, so the two agree.
///
/// `PRAGMA optimize` is SQLite's own remedy: it analyses only tables whose
/// statistics are missing or stale, on a bounded sample, so it is cheap to call
/// routinely — measured at ~160 ms the first time on a never-analysed
/// 50,000-photo catalogue and ~0.3 ms on every call after. `0x10002` asks it to
/// consider every table, not only those this connection has already queried,
/// which is what a connection that has just opened needs.
pub fn refresh_planner_stats(conn: &Connection) -> Result<()> {
    conn.execute_batch("PRAGMA optimize=0x10002;")?;
    Ok(())
}

/// In-memory database, for tests.
pub fn open_memory() -> Result<Connection> {
    let conn = Connection::open_in_memory()?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    migrate(&conn)?;
    Ok(conn)
}

/// Opens without migrating — used per command, since WAL lets readers run
/// concurrently with the writer.
pub fn connect(path: &Path) -> Result<Connection> {
    let mut conn = Connection::open(path)?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    // Wait rather than fail if the scan happens to be mid-write.
    conn.busy_timeout(std::time::Duration::from_secs(10))?;
    // Each command opens its own connection, so its page cache starts cold.
    // Mapping the file lets every connection read pages the last one
    // brought in, from the OS's cache instead of a read call per 4 KB page; a
    // larger cache holds a whole facet pass's pages; temp b-trees for a GROUP
    // BY stay in memory; and the WAL is cut back after a big write instead of
    // staying hundreds of MB on disk.
    conn.pragma_update(None, "mmap_size", 268_435_456i64)?;
    conn.pragma_update(None, "cache_size", -32_768i64)?;
    conn.pragma_update(None, "temp_store", "MEMORY")?;
    conn.pragma_update(None, "journal_size_limit", 67_108_864i64)?;
    // Transactions take the write lock when they begin, not at their first
    // write. A deferred one that reads and then writes cannot upgrade once
    // another connection has committed in between: SQLite fails it at once
    // with "database is locked", and the busy timeout above never applies.
    // Taken up front, it waits its turn like any other write.
    conn.set_transaction_behavior(rusqlite::TransactionBehavior::Immediate);
    Ok(conn)
}

fn placeholders(n: usize) -> String {
    std::iter::repeat_n("?", n).collect::<Vec<_>>().join(",")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::testing::*;

    /// Nothing wrote planner statistics before, so every real catalogue was
    /// planned on SQLite's defaults. `open` now refreshes them; this checks the
    /// refresh actually records some once there is data to describe.
    #[test]
    fn refreshing_planner_stats_records_them() {
        let mut conn = open_memory().unwrap();
        // Harmless on an empty catalogue — the state of every first launch.
        refresh_planner_stats(&conn).unwrap();

        let ids = photos(&conn, 50);
        add_manual_tag_many(&mut conn, &ids, "landscape", "genre").unwrap();
        refresh_planner_stats(&conn).unwrap();

        let rows: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_stat1 WHERE tbl = 'photo_tags'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(rows > 0, "no statistics recorded for photo_tags");
    }
}
