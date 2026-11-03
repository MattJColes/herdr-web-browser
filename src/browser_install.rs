//! `install-browser`: download Chrome for Testing (Google's Chrome build for automation) and use
//! it instead of the user's own Chrome. Same engine, but its own app: on macOS a hidden Google
//! Chrome takes over the Chrome app, so clicking Chrome in the Dock opens nothing.

use crate::R;
use crate::state;
use serde_json::Value;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

const VERSIONS: &str = "https://googlechromelabs.github.io/chrome-for-testing/last-known-good-versions-with-downloads.json";

fn platform() -> R<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => Ok("mac-arm64"),
        ("macos", "x86_64") => Ok("mac-x64"),
        ("linux", "x86_64") => Ok("linux64"),
        (os, arch) => Err(format!("Chrome for Testing has no build for {os}/{arch}; set chrome_path instead").into()),
    }
}

fn dir() -> PathBuf {
    state::state_dir().join("chrome-for-testing")
}

/// The installed Chrome for Testing executable, if there is one.
pub fn installed() -> Option<PathBuf> {
    let p = platform().ok()?;
    let base = dir().join(format!("chrome-{p}"));
    let exe =
        if p.starts_with("mac") { base.join("Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing") } else { base.join("chrome") };
    exe.is_file().then_some(exe)
}

fn run(cmd: &mut Command) -> R<Vec<u8>> {
    let name = cmd.get_program().to_string_lossy().into_owned();
    let out = cmd.output().map_err(|e| format!("{name}: {e}"))?;
    if !out.status.success() {
        return Err(format!("{name} failed: {}", String::from_utf8_lossy(&out.stderr).trim()).into());
    }
    Ok(out.stdout)
}

/// Download the current stable build unless it's already there; returns its version.
/// The browser must not be running from the old copy while this replaces it.
pub fn install() -> R<String> {
    let p = platform()?;
    let index: Value = serde_json::from_slice(&run(Command::new("curl").args(["-fsSL", "--max-time", "30", VERSIONS]))?)?;
    let stable = &index["channels"]["Stable"];
    let version = stable["version"].as_str().ok_or("Chrome for Testing lists no stable version")?;
    let url = stable["downloads"]["chrome"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|d| d["platform"] == p)
        .and_then(|d| d["url"].as_str())
        .ok_or_else(|| format!("no Chrome for Testing download for {p}"))?;
    if installed().is_some() && fs::read_to_string(dir().join("version")).ok().as_deref() == Some(version) {
        return Ok(version.into());
    }
    eprintln!("downloading Chrome for Testing {version} (about 200 MB)…");
    let tmp = state::state_dir().join("chrome-for-testing.download");
    let _ = fs::remove_dir_all(&tmp);
    fs::create_dir_all(&tmp)?;
    let zip = tmp.join("chrome.zip");
    run(Command::new("curl").args(["-fsSL", "--max-time", "900", "--retry", "2", "-o"]).arg(&zip).arg(url))?;
    run(Command::new("unzip").args(["-q", "-o"]).arg(&zip).arg("-d").arg(&tmp))?;
    fs::remove_file(&zip)?;
    fs::write(tmp.join("version"), version)?;
    let _ = fs::remove_dir_all(dir());
    fs::rename(&tmp, dir())?;
    Ok(version.into())
}
