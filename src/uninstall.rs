//! `uninstall`: undo what the install-* commands set up outside the plugin folder.

use crate::{R, agent_setup, chrome, state};
use serde_json::Value;
use std::path::PathBuf;

/// Remove agent instructions, desktop mods, their programs.json entry and menu shortcut, and the CLI link;
/// with `purge`, also the browser's state (profile, Chrome for Testing, screenshots).
/// Returns what was removed.
pub fn run(purge: bool) -> R<Vec<String>> {
    let mut done = Vec::new();
    if chrome::stop()? {
        done.push("stopped the browser".into());
    }
    done.extend(agent_setup::uninstall_agents()?);
    let app = crate::desktop_home();
    for name in crate::DESKTOP_MODS {
        let dir = app.join("mods").join(name);
        if dir.is_dir() {
            std::fs::remove_dir_all(&dir)?;
            done.push(format!("removed {}", dir.display()));
        }
    }
    let programs = app.join("programs.json");
    if let Ok(text) = std::fs::read_to_string(&programs) {
        let mut doc: Value = serde_json::from_str(&text).map_err(|e| format!("{}: {e}; not changing it", programs.display()))?;
        if doc["programs"].as_object_mut().and_then(|p| p.remove("herdr-web-browser")).is_some() {
            std::fs::write(&programs, serde_json::to_vec_pretty(&doc)?)?;
            done.push(format!("removed Browser from {}", programs.display()));
        }
    }
    let keys = app.join("keys.json");
    if let Ok(text) = std::fs::read_to_string(&keys) {
        let mut doc: Value = serde_json::from_str(&text).map_err(|e| format!("{}: {e}; not changing it", keys.display()))?;
        if let Some(g) = doc["global"].as_object_mut() {
            let before = g.len();
            g.retain(|_, v| v != crate::MENU_ACTION);
            if g.len() != before {
                std::fs::write(&keys, serde_json::to_vec_pretty(&doc)?)?;
                done.push(format!("removed the browser menu shortcut from {}", keys.display()));
            }
        }
    }
    let link = PathBuf::from(std::env::var_os("HOME").ok_or("HOME is not set")?).join(".local/bin/herdr-web-browser");
    // Only our own symlink, never a file someone put there.
    if std::fs::symlink_metadata(&link).is_ok_and(|m| m.file_type().is_symlink()) {
        std::fs::remove_file(&link)?;
        done.push(format!("removed {}", link.display()));
    }
    if purge && state::state_dir().is_dir() {
        std::fs::remove_dir_all(state::state_dir())?;
        done.push(format!("deleted {} (profile, Chrome for Testing, screenshots)", state::state_dir().display()));
    }
    Ok(done)
}
