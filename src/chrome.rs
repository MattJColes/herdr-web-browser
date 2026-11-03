//! Find, launch, stop and mode-switch the shared Chromium.

use crate::cdp::Cdp;
use crate::state::{self, Config, Mode};
use crate::{R, page};
use serde_json::json;
use std::fs;
use std::net::TcpStream;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread::sleep;
use std::time::{Duration, Instant};

pub const NATIVE_HOST: &str = "dev.herdr.browser";
/// Pinned by the `key` in extension/manifest.json.
pub const EXTENSION_ID: &str = "lddhafndblopaoibknlffikbokipehfd";

#[cfg(target_os = "macos")]
const APP_PATHS: &[&str] = &[
    "Google Chrome.app/Contents/MacOS/Google Chrome",
    "Chromium.app/Contents/MacOS/Chromium",
    "Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
    "Brave Browser.app/Contents/MacOS/Brave Browser",
];
#[cfg(not(target_os = "macos"))]
const BIN_NAMES: &[&str] = &[
    "google-chrome",
    "google-chrome-stable",
    "chromium",
    "chromium-browser",
    "microsoft-edge",
    "brave-browser",
];

pub fn find_chrome(cfg: &Config) -> R<PathBuf> {
    if let Some(p) = &cfg.chrome_path {
        return if p.is_file() { Ok(p.clone()) } else { Err(format!("chrome_path {} does not exist", p.display()).into()) };
    }
    // Its own app, so it can't take over the user's Chrome (see browser_install).
    if let Some(p) = crate::browser_install::installed() {
        return Ok(p);
    }
    #[cfg(target_os = "macos")]
    {
        let home = PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join("Applications");
        for base in [PathBuf::from("/Applications"), home] {
            for app in APP_PATHS {
                let p = base.join(app);
                if p.is_file() {
                    return Ok(p);
                }
            }
        }
    }
    #[cfg(not(target_os = "macos"))]
    for dir in std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()) {
        for name in BIN_NAMES {
            let p = dir.join(name);
            if p.is_file() {
                return Ok(p);
            }
        }
    }
    Err(format!(
        "no Chrome, Chromium, Edge or Brave found; set chrome_path in {}/config.toml",
        state::config_dir().display()
    )
    .into())
}

/// `DevToolsActivePort` holds the port on line 1 and the browser ws path on line 2.
pub fn parse_port_file(s: &str) -> Option<(u16, String)> {
    let mut lines = s.lines();
    let port = lines.next()?.trim().parse().ok()?;
    let path = lines.next()?.trim();
    path.starts_with('/').then(|| (port, path.to_string()))
}

/// Browser websocket URL if a browser is running and accepting connections.
pub fn endpoint() -> Option<String> {
    let s = fs::read_to_string(state::profile_dir().join("DevToolsActivePort")).ok()?;
    let (port, path) = parse_port_file(&s)?;
    let addr = ([127, 0, 0, 1], port).into();
    TcpStream::connect_timeout(&addr, Duration::from_millis(300)).ok()?;
    Some(format!("ws://127.0.0.1:{port}{path}"))
}

pub fn port() -> Option<u16> {
    let s = fs::read_to_string(state::profile_dir().join("DevToolsActivePort")).ok()?;
    parse_port_file(&s).map(|(p, _)| p)
}

/// Chrome launches the host as `<path> <origin>`, so `path` must be a wrapper
/// that adds our subcommand.
fn install_native_host(profile: &Path) -> R<()> {
    let exe = fs::canonicalize(std::env::current_exe()?)?;
    let wrapper = state::state_dir().join("native-host.sh");
    let quoted = exe.display().to_string().replace('\'', r"'\''");
    fs::write(&wrapper, format!("#!/bin/sh\nexec '{quoted}' native-host \"$@\"\n"))?;
    fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o755))?;
    let dir = profile.join("NativeMessagingHosts");
    fs::create_dir_all(&dir)?;
    let manifest = json!({
        "name": NATIVE_HOST,
        "description": "herdr-web-browser bridge to herdr agents",
        "path": wrapper,
        "type": "stdio",
        "allowed_origins": [format!("chrome-extension://{EXTENSION_ID}/")],
    });
    fs::write(dir.join(format!("{NATIVE_HOST}.json")), serde_json::to_vec_pretty(&manifest)?)?;
    Ok(())
}

fn has_display() -> bool {
    cfg!(target_os = "macos") || ["DISPLAY", "WAYLAND_DISPLAY"].iter().any(|v| std::env::var_os(v).is_some_and(|x| !x.is_empty()))
}

pub fn launch(mode: Mode, cfg: &Config) -> R<String> {
    let mode = if mode == Mode::Headed && !has_display() { Mode::Headless } else { mode };
    let chrome = find_chrome(cfg)?;
    let profile = state::profile_dir();
    fs::create_dir_all(&profile)?;
    let _ = fs::remove_file(profile.join("DevToolsActivePort"));
    install_native_host(&profile)?;
    let log = fs::File::options().create(true).append(true).open(state::state_dir().join("chrome.log"))?;
    let mut cmd = Command::new(&chrome);
    cmd.arg(format!("--user-data-dir={}", profile.display()))
        // Port 0 = ephemeral; Chrome binds the DevTools listener to 127.0.0.1 only.
        .args(["--remote-debugging-port=0", "--remote-debugging-address=127.0.0.1"])
        // Lets us load our unpacked extension over CDP (branded Chrome ignores --load-extension).
        .arg("--enable-unsafe-extension-debugging")
        .args(["--no-first-run", "--no-default-browser-check", "--hide-crash-restore-bubble"])
        // With some profiles Chrome counts its headless windows as occluded after ~1s and
        // stops painting them, which freezes the live view on its first frame.
        .arg("--disable-backgrounding-occluded-windows");
    if mode == Mode::Headless {
        cmd.arg("--headless=new");
    }
    cmd.arg("about:blank")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(log)
        .process_group(0); // outlive the herdr action / pane that started us
    let child = cmd.spawn().map_err(|e| format!("launch {}: {e}", chrome.display()))?;
    let pid = child.id();

    let deadline = Instant::now() + Duration::from_secs(20);
    let url = loop {
        if let Some(u) = endpoint() {
            break u;
        }
        if Instant::now() > deadline {
            return Err(format!("Chromium did not start; see {}/chrome.log", state::state_dir().display()).into());
        }
        sleep(Duration::from_millis(100));
    };
    state::update(|s| s.mode = Some(mode))?;
    let ext = state::plugin_root().join("extension");
    let loaded = Cdp::connect(&url).and_then(|mut c| c.call("Extensions.loadUnpacked", json!({ "path": ext }), None));
    if let Err(e) = loaded {
        eprintln!("herdr-web-browser: extension not loaded ({e}); in-browser send menus unavailable");
    }
    if mode == Mode::Headed {
        present(&url, pid);
    }
    Ok(url)
}

/// Show a windowed browser properly: maximized and in front. Chrome started from a
/// background process (a herdr pane, an agent) otherwise opens small and behind.
fn present(url: &str, pid: u32) {
    if let Ok(mut c) = Cdp::connect(url) {
        // Chrome registers its first window just after the port opens.
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut windows = Vec::new();
        while windows.is_empty() && Instant::now() < deadline {
            for t in page::page_targets(&mut c).unwrap_or_default() {
                if let Ok(w) = c.call("Browser.getWindowForTarget", json!({ "targetId": t.id }), None)
                    && let Some(id) = w["windowId"].as_i64()
                    && !windows.contains(&id)
                {
                    windows.push(id);
                }
            }
            if windows.is_empty() {
                sleep(Duration::from_millis(100));
            }
        }
        for id in windows {
            let _ = c.call("Browser.setWindowBounds", json!({ "windowId": id, "bounds": { "windowState": "maximized" } }), None);
        }
    }
    bring_to_front(pid);
}

/// Bring this exact Chrome process forward (not the user's everyday Chrome, which shares
/// the app name). macOS may ask once to let herdr control System Events.
#[cfg(target_os = "macos")]
fn bring_to_front(pid: u32) {
    let script = format!("tell application \"System Events\" to set frontmost of (first process whose unix id is {pid}) to true");
    let _ = crate::host::run("osascript", &["-e", &script]);
}

#[cfg(not(target_os = "macos"))]
fn bring_to_front(_pid: u32) {}

/// Running browser's endpoint, starting a headless one if needed: a window only
/// appears when the user asks for one (`mode headed`, the Window button).
pub fn ensure(cfg: &Config) -> R<String> {
    match endpoint() {
        Some(u) => Ok(u),
        None if restarting() => Err("the browser is restarting".into()),
        None => launch(Mode::Headless, cfg),
    }
}

/// Held while `restart` has the browser stopped, so live views (which reconnect every
/// second) don't start another one mid-swap. Ignored after a few minutes, should a
/// restart die holding it.
fn restart_lock() -> PathBuf {
    state::state_dir().join("restarting")
}

fn restarting() -> bool {
    fs::metadata(restart_lock()).and_then(|m| m.modified()).is_ok_and(|t| t.elapsed().unwrap_or_default() < Duration::from_secs(600))
}

/// Chrome removes the profile's SingletonLock symlink on clean exit.
fn wait_exit() -> R<()> {
    let lock = state::profile_dir().join("SingletonLock");
    let deadline = Instant::now() + Duration::from_secs(15);
    while fs::symlink_metadata(&lock).is_ok() || endpoint().is_some() {
        if Instant::now() > deadline {
            return Err("Chromium did not exit within 15s".into());
        }
        sleep(Duration::from_millis(100));
    }
    Ok(())
}

pub fn stop() -> R<bool> {
    let Some(url) = endpoint() else { return Ok(false) };
    let mut c = Cdp::connect(&url)?;
    let _ = c.call("Browser.close", json!({}), None); // the connection drops as it closes
    wait_exit()?;
    Ok(true)
}

/// Relaunch in `mode`, reopening every tab and keeping pane ownership.
pub fn switch_mode(mode: Mode, cfg: &Config) -> R<()> {
    if endpoint().is_none() {
        launch(mode, cfg)?;
        return Ok(());
    }
    restart(mode, cfg, || Ok(()))
}

/// Stop the browser, run `between` (e.g. to change its profile), then relaunch in `mode`
/// with every tab reopened and pane ownership kept. Not running: just `between`.
pub fn restart(mode: Mode, cfg: &Config, between: impl FnOnce() -> R<()>) -> R<()> {
    fs::create_dir_all(state::state_dir())?;
    fs::write(restart_lock(), "")?;
    let result = restart_locked(mode, cfg, between);
    let _ = fs::remove_file(restart_lock());
    result
}

fn restart_locked(mode: Mode, cfg: &Config, between: impl FnOnce() -> R<()>) -> R<()> {
    let Some(url) = endpoint() else { return between() };
    let mut c = Cdp::connect(&url)?;
    let owners = state::load()?.tabs;
    let tabs: Vec<(Option<String>, String)> = page::page_targets(&mut c)?
        .into_iter()
        .map(|t| (owners.iter().find(|(_, id)| **id == t.id).map(|(p, _)| p.clone()), t.url))
        .collect();
    drop(c);
    stop()?;
    let changed = between();
    let url = launch(mode, cfg)?;
    let mut c = Cdp::connect(&url)?;
    // Chrome registers its startup tab shortly after the port opens; wait so it
    // can be closed instead of racing the restored tabs.
    let deadline = Instant::now() + Duration::from_secs(5);
    let initial = loop {
        let t = page::page_targets(&mut c)?;
        if !t.is_empty() || Instant::now() > deadline {
            break t;
        }
        sleep(Duration::from_millis(100));
    };
    let mut map = std::collections::BTreeMap::new();
    for (owner, u) in &tabs {
        let id = page::create_tab(&mut c, u)?;
        if let Some(p) = owner {
            map.insert(p.clone(), id);
        }
    }
    if !tabs.is_empty() {
        for t in initial {
            let _ = c.call("Target.closeTarget", json!({ "targetId": t.id }), None);
        }
    }
    state::update(|s| s.tabs = map)?;
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_devtools_active_port() {
        assert_eq!(
            parse_port_file("57047\n/devtools/browser/abc-123\n"),
            Some((57047, "/devtools/browser/abc-123".into()))
        );
    }

    #[test]
    fn rejects_truncated_port_file() {
        assert_eq!(parse_port_file("57047\n"), None);
        assert_eq!(parse_port_file("nope\n/devtools/browser/x"), None);
    }
}
