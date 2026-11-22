//! Windows locks a running exe against overwrite but not rename. herdr's status pane and live
//! views run target/release/herdr-web-browser.exe, so move a running build (and its hardlinked
//! copy in deps/) aside before cargo links the new one, or rebuilds and plugin updates fail.

use std::path::Path;

fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    // OUT_DIR is <target>/<profile>/build/<pkg>-<hash>/out (cargo 1.100+: build/<pkg>/<hash>/out).
    let Some(out) = std::env::var_os("OUT_DIR") else { return };
    let Some(build) = Path::new(&out).ancestors().find(|p| p.file_name().is_some_and(|n| n == "build")) else { return };
    let Some(profile) = build.parent().map(Path::to_path_buf) else { return };
    let deps = profile.join("deps");
    for dir in [&profile, &deps] {
        let Ok(entries) = std::fs::read_dir(dir) else { continue };
        for path in entries.flatten().map(|e| e.path()) {
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or_default();
            let ours = name.starts_with("herdr-web-browser") || name.starts_with("herdr_web_browser");
            if ours && name.ends_with(".old") {
                let _ = std::fs::remove_file(&path); // fails while that old build still runs; next time
            } else if ours && name.ends_with(".exe") && std::fs::OpenOptions::new().write(true).open(&path).is_err() {
                // Only a locked (running) build: moving a free one would leave none if this build fails.
                let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or_default();
                let _ = std::fs::rename(&path, path.with_extension(format!("exe.{nanos}.old")));
            }
        }
    }
}
