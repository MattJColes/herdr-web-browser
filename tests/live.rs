//! Drives a real headless Chromium. Needs Chrome/Chromium installed:
//! `cargo test -- --ignored`
use std::process::Command;

fn hb(state: &std::path::Path, args: &[&str]) -> String {
    let out = Command::new(env!("CARGO_BIN_EXE_herdr-web-browser"))
        .args(args)
        .env("HERDR_PLUGIN_STATE_DIR", state)
        .env("HERDR_PLUGIN_ROOT", env!("CARGO_MANIFEST_DIR"))
        .env_remove("HERDR_PANE_ID")
        .output()
        .unwrap();
    assert!(out.status.success(), "{args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8(out.stdout).unwrap()
}

#[test]
#[ignore]
fn snapshot_type_click_round_trip() {
    let state = std::env::temp_dir().join(format!("hb-live-{}", std::process::id()));
    hb(&state, &["start", "--headless"]);
    let page = r#"data:text/html,<input placeholder=Name><button onclick="document.title='hi '+document.querySelector('input').value">Go</button>"#;
    hb(&state, &["open", page]);
    let snap = hb(&state, &["snapshot"]);
    assert!(snap.contains(r#"[1] textbox "Name""#) && snap.contains(r#"[2] button "Go""#), "{snap}");
    hb(&state, &["type", "1", "ada"]);
    hb(&state, &["click", "2"]);
    let title = hb(&state, &["eval", "document.title"]);
    hb(&state, &["stop"]);
    let _ = std::fs::remove_dir_all(&state);
    assert_eq!(title.trim(), "hi ada");
}
