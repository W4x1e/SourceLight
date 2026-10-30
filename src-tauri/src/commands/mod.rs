//! The commands the interface calls, by area.
//!
//! Each is listed in `specta_builder` in `lib.rs`, which also generates their
//! TypeScript bindings. Helpers more than one area uses are in [`common`].

pub(crate) mod ai;
pub(crate) mod app;
pub(crate) mod backups;
pub(crate) mod browse;
pub(crate) mod collections;
pub(crate) mod colours;
pub(crate) mod common;
pub(crate) mod cull;
pub(crate) mod develop;
pub(crate) mod export;
pub(crate) mod import;
pub(crate) mod library;
pub(crate) mod masks;
pub(crate) mod metadata;
pub(crate) mod people;
pub(crate) mod phone;
pub(crate) mod places;
pub(crate) mod presets;
pub(crate) mod protocols;
pub(crate) mod render;
pub(crate) mod sidecars;
pub(crate) mod tagging;

#[cfg(test)]
mod tests {
    use std::path::Path;

    /// A plain `#[tauri::command]` on a sync fn runs on the main thread, and
    /// the window waits for it. This lists every such command that
    /// takes the develop lock, decodes a file or backs the catalogue up, and
    /// fails on any not named here — each of which is either cheap, an edit
    /// that must apply in order with the others, or drawing to the window's
    /// own surface.
    #[test]
    fn no_new_heavy_command_runs_on_the_main_thread() {
        const ALLOWED: &[&str] = &[
            // Edits, applied in order: see ARCHITECTURE.md.
            "set_develop_stack",
            "reset_develop",
            "develop_remove_erase",
            "undo_develop",
            "redo_develop",
            "develop_go_to",
            "auto_tone_develop",
            "auto_wb_develop",
            "auto_all_develop",
            "pick_wb_develop",
            "apply_preset",
            "paste_settings",
            "paste_settings_from",
            "save_develop_as_copy",
            "save_preset",
            "close_develop",
            // The direct surface is the window's: AppKit wants its thread.
            "render_direct",
            "probe_direct",
        ];
        // Every command module, read as one.
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/commands");
        let src = std::fs::read_dir(dir)
            .unwrap()
            .map(|entry| std::fs::read_to_string(entry.unwrap().path()).unwrap())
            .collect::<Vec<_>>()
            .join("\n")
            .replace("pub(crate) fn ", "fn ");
        let mut heavy = Vec::new();
        let mut rest = src.as_str();
        while let Some(at) = rest.find("#[tauri::command]\n#[specta::specta]\nfn ") {
            let after = &rest[at + "#[tauri::command]\n#[specta::specta]\nfn ".len()..];
            let name: String = after
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            // The body, to the next command or item at column 0.
            let body = after.split("\n}\n").next().unwrap_or("");
            // A backup copies the whole catalogue.
            if (body.contains("with_develop(")
                || body.contains("LibRawDecoder")
                || body.contains("raster::load")
                || body.contains("backup::create"))
                && !ALLOWED.contains(&name.as_str())
            {
                heavy.push(name);
            }
            rest = after;
        }
        assert!(
            heavy.is_empty(),
            "heavy sync commands: {heavy:?} — make them #[tauri::command(async)]"
        );
    }
}
