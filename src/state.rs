//! Paths, user config and the shared `state.json` (mode, sticky target, pane→tab map).

use crate::R;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs::{self, File};
use std::path::{Path, PathBuf};

pub const PLUGIN_ID: &str = "herdr-web-browser";

fn home() -> PathBuf {
    PathBuf::from(std::env::var_os("HOME").expect("HOME is not set"))
}

fn env_dir(var: &str) -> Option<PathBuf> {
    std::env::var_os(var).filter(|v| !v.is_empty()).map(PathBuf::from)
}

/// Herdr sets HERDR_PLUGIN_STATE_DIR for plugin commands; agents calling the CLI
/// from their shell don't have it, so fall back to the same path herdr uses.
pub fn state_dir() -> PathBuf {
    env_dir("HERDR_PLUGIN_STATE_DIR")
        .unwrap_or_else(|| env_dir("XDG_STATE_HOME").unwrap_or_else(|| home().join(".local/state")).join("herdr/plugins").join(PLUGIN_ID))
}

pub fn config_dir() -> PathBuf {
    env_dir("HERDR_PLUGIN_CONFIG_DIR")
        .unwrap_or_else(|| env_dir("XDG_CONFIG_HOME").unwrap_or_else(|| home().join(".config")).join("herdr/plugins/config").join(PLUGIN_ID))
}

/// Plugin checkout root: env from herdr, else derived from `<root>/target/release/herdr-web-browser`.
pub fn plugin_root() -> PathBuf {
    env_dir("HERDR_PLUGIN_ROOT").unwrap_or_else(|| {
        let exe = std::env::current_exe().and_then(fs::canonicalize).expect("cannot locate own executable");
        exe.ancestors().nth(3).map(Path::to_path_buf).unwrap_or_default()
    })
}

pub fn profile_dir() -> PathBuf {
    state_dir().join("profile")
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub default_agent: Option<String>,
    pub chrome_path: Option<PathBuf>,
    pub max_chars: Option<usize>,
    /// Live view: terminal cell height / width, if the detected one is off.
    pub cell_aspect: Option<f64>,
    /// Live view: "kitty" or "blocks"; detected when unset.
    pub view_renderer: Option<String>,
    /// What a new Browser (herdr desktop's + menu) opens on.
    pub home_page: Option<String>,
}

pub const DEFAULT_MAX_CHARS: usize = 20_000;
pub const DEFAULT_HOME_PAGE: &str = "https://www.google.com";

impl Config {
    pub fn load() -> R<Config> {
        let path = config_dir().join("config.toml");
        match fs::read_to_string(&path) {
            Ok(s) => toml::from_str(&s).map_err(|e| format!("{}: {e}", path.display()).into()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Config::default()),
            Err(e) => Err(format!("{}: {e}", path.display()).into()),
        }
    }
    pub fn max_chars(&self) -> usize {
        self.max_chars.unwrap_or(DEFAULT_MAX_CHARS)
    }
    pub fn home_page(&self) -> &str {
        self.home_page.as_deref().unwrap_or(DEFAULT_HOME_PAGE)
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, clap::ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    Headed,
    #[default]
    Headless,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Picker {
    Pending,
    Chosen,
    Cancelled,
}

#[derive(Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct State {
    /// Mode of the last launch; auto-start reuses it.
    pub mode: Option<Mode>,
    /// Sticky agent target (agent name or pane id).
    pub target: Option<String>,
    /// herdr pane id -> CDP target id of the tab that pane owns.
    #[serde(default)]
    pub tabs: BTreeMap<String, String>,
    pub picker: Option<Picker>,
    /// Desktop toast [title, body] waiting for the toast mod to post it.
    pub toast: Option<[String; 2]>,
}

fn state_path(dir: &Path) -> PathBuf {
    dir.join("state.json")
}

fn lock(dir: &Path) -> R<File> {
    fs::create_dir_all(dir)?;
    let f = File::options().create(true).truncate(false).write(true).open(dir.join("state.lock"))?;
    f.lock()?;
    Ok(f)
}

fn read(dir: &Path) -> R<State> {
    match fs::read_to_string(state_path(dir)) {
        Ok(s) => Ok(serde_json::from_str(&s)?),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(State::default()),
        Err(e) => Err(e.into()),
    }
}

/// Read-modify-write under an exclusive lock; the write is atomic (tmp + rename).
pub fn update_in<T>(dir: &Path, f: impl FnOnce(&mut State) -> T) -> R<T> {
    let _guard = lock(dir)?;
    let mut s = read(dir)?;
    let out = f(&mut s);
    let tmp = dir.join("state.json.tmp");
    fs::write(&tmp, serde_json::to_vec_pretty(&s)?)?;
    fs::rename(tmp, state_path(dir))?;
    Ok(out)
}

pub fn load_in(dir: &Path) -> R<State> {
    let _guard = lock(dir)?;
    read(dir)
}

pub fn update<T>(f: impl FnOnce(&mut State) -> T) -> R<T> {
    update_in(&state_dir(), f)
}

pub fn load() -> R<State> {
    load_in(&state_dir())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_round_trips_through_disk() {
        let dir = std::env::temp_dir().join(format!("hb-state-{}", std::process::id()));
        update_in(&dir, |s| {
            s.mode = Some(Mode::Headless);
            s.target = Some("reviewer".into());
            s.tabs.insert("w1:p1".into(), "TARGET1".into());
            s.picker = Some(Picker::Chosen);
        })
        .unwrap();
        let s = load_in(&dir).unwrap();
        assert_eq!(s.mode, Some(Mode::Headless));
        assert_eq!(s.target.as_deref(), Some("reviewer"));
        assert_eq!(s.tabs.get("w1:p1").map(String::as_str), Some("TARGET1"));
        assert_eq!(s.picker, Some(Picker::Chosen));
        fs::remove_dir_all(dir).unwrap();
    }
}
