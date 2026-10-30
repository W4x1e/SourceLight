//! Updates for the direct-download build.
//!
//! A direct download has no store behind it to deliver a fix, so this is the
//! only way a bug fix reaches someone who installed it, and the beta exists to
//! find bugs in real libraries — so it has to exist before the beta does. A
//! build from the Microsoft Store would get updates from the Store and need
//! none of it.
//!
//! **Switched on by configuration, not by code.** `tauri-plugin-updater` refuses
//! to start without a public key, and a key is a decision with no undo: every
//! installed copy trusts it for good, so losing the private half means no
//! installed copy can ever be updated again. That key, and the public address the
//! manifest is served from, are the maintainer's to create. Until
//! both are set under `plugins.updater` in `tauri.conf.json`, the plugin is not
//! registered and Settings says updates are not set up for this build. Nothing
//! else changes. See `docs/UPDATES.md`.
//!
//! **Offline is a promise, not a mode.** A check is one request to the
//! configured endpoint and nothing else; when it fails, the automatic check at
//! launch says nothing at all, and only a check the user asked for reports it.

use serde::Serialize;
use tauri::Runtime;
use tauri_plugin_updater::UpdaterExt;

/// Whether this build knows where updates come from and how to trust them: a
/// non-empty `pubkey` and at least one endpoint under `plugins.updater`.
///
/// Checked before the plugin is registered — registering it without a key fails
/// at start-up, which would turn a missing optional feature into an app that
/// does not open.
pub fn configured(config: &tauri::Config) -> bool {
    let Some(updater) = config.plugins.0.get("updater") else {
        return false;
    };
    let has_key = updater
        .get("pubkey")
        .and_then(|k| k.as_str())
        .is_some_and(|k| !k.trim().is_empty());
    let has_endpoint = updater
        .get("endpoints")
        .and_then(|e| e.as_array())
        .is_some_and(|e| !e.is_empty());
    has_key && has_endpoint
}

/// What a check for updates found.
#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct UpdateCheck {
    /// False when this build has no update source — see [`configured`].
    pub configured: bool,
    /// The version running now.
    pub current_version: String,
    /// A newer release, if there is one.
    pub available: Option<AvailableUpdate>,
}

#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct AvailableUpdate {
    pub version: String,
    /// The release notes, as written in the manifest.
    pub notes: Option<String>,
    /// When it was published, if the manifest says.
    pub date: Option<String>,
}

/// Asks the configured endpoint whether there is a newer release.
///
/// Errors — no network, a manifest that will not parse, a signature problem —
/// come back as `Err` for the caller to decide about: shown when the user asked,
/// swallowed when the app asked on its own.
pub async fn check<R: Runtime>(app: &tauri::AppHandle<R>) -> Result<UpdateCheck, String> {
    let current_version = app.package_info().version.to_string();
    if !configured(app.config()) {
        return Ok(UpdateCheck {
            configured: false,
            current_version,
            available: None,
        });
    }
    let update = app
        .updater()
        .map_err(|e| e.to_string())?
        .check()
        .await
        .map_err(|e| e.to_string())?;
    Ok(UpdateCheck {
        configured: true,
        current_version,
        available: update.map(|u| AvailableUpdate {
            version: u.version,
            notes: u.body,
            date: u.date.map(|d| d.to_string()),
        }),
    })
}

/// Downloads the newer release, verifies its signature, and runs it.
///
/// The signature is checked against the public key in the configuration before
/// anything is executed; a download that does not verify is refused by the
/// plugin, not by this code. On Windows the installer takes over and the app
/// exits while it runs; elsewhere the app restarts into the new version.
pub async fn install<R: Runtime>(app: &tauri::AppHandle<R>) -> Result<(), String> {
    if !configured(app.config()) {
        return Err("Updates are not set up for this build.".into());
    }
    let update = app
        .updater()
        .map_err(|e| e.to_string())?
        .check()
        .await
        .map_err(|e| e.to_string())?
        .ok_or("You already have the newest version.")?;
    // How far the download is, for the page: the button sat on
    // "Installing…" through a slow download with nothing moving.
    let mut received: u64 = 0;
    update
        .download_and_install(
            |chunk, total| {
                use tauri::Emitter;
                received += chunk as u64;
                let _ = app.emit(
                    "update-progress",
                    serde_json::json!({ "received": received, "total": total.unwrap_or(0) }),
                );
            },
            || {},
        )
        .await
        .map_err(|e| e.to_string())?;
    app.restart();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config_with(plugins: serde_json::Value) -> tauri::Config {
        let mut base: serde_json::Value = serde_json::from_str(include_str!("../tauri.conf.json"))
            .expect("tauri.conf.json parses");
        base["plugins"] = plugins;
        serde_json::from_value(base).expect("a valid tauri config")
    }

    /// Registering the plugin without a key stops the app opening, so a partial
    /// configuration must read as no configuration at all.
    #[test]
    fn updates_need_both_a_key_and_an_endpoint() {
        let none = config_with(serde_json::json!({}));
        assert!(!configured(&none));

        let key_only = config_with(serde_json::json!({ "updater": { "pubkey": "abc" } }));
        assert!(!configured(&key_only));

        let blank_key = config_with(serde_json::json!({
            "updater": { "pubkey": "  ", "endpoints": ["https://example.com/latest.json"] }
        }));
        assert!(!configured(&blank_key));

        let both = config_with(serde_json::json!({
            "updater": { "pubkey": "abc", "endpoints": ["https://example.com/latest.json"] }
        }));
        assert!(configured(&both));
    }
}
