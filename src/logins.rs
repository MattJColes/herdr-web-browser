//! `import-logins`: copy the user's Google Chrome sign-ins (cookies) into this browser's
//! profile. Both run the same Chrome app, whose cookie encryption key belongs to the app
//! (macOS Keychain "Chrome Safe Storage"), not the profile, so the encrypted rows work
//! as they are: nothing is decrypted. Uses the system `sqlite3` tool.

use crate::R;
use crate::chrome;
use crate::state::{self, Config};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Google Chrome's cookie database for `profile`: a profile folder name ("Default",
/// "Profile 1", ...) in Chrome's usual place, or a path to a profile folder.
fn chrome_cookies(profile: &str) -> R<PathBuf> {
    let home = PathBuf::from(std::env::var_os("HOME").ok_or("HOME is not set")?);
    let base = if cfg!(target_os = "macos") { home.join("Library/Application Support/Google/Chrome") } else { home.join(".config/google-chrome") };
    let db = base.join(profile).join("Cookies"); // an absolute `profile` replaces `base`
    if !db.is_file() {
        return Err(format!("no Google Chrome cookies at {}", db.display()).into());
    }
    Ok(db)
}

/// A copy, so Chrome can keep its own database open while we read.
fn snapshot(profile: &str) -> R<PathBuf> {
    let src = chrome_cookies(profile)?;
    std::fs::create_dir_all(state::state_dir())?;
    let copy = state::state_dir().join("chrome-cookies.sqlite");
    std::fs::copy(&src, &copy).map_err(|e| format!("can't read {}: {e}", src.display()))?;
    Ok(copy)
}

fn sqlite(db: &Path, sql: &str) -> R<String> {
    let out = Command::new("sqlite3").arg("-batch").arg(db).arg(sql).output().map_err(|e| format!("sqlite3: {e}"))?;
    if !out.status.success() {
        return Err(format!("sqlite3: {}", String::from_utf8_lossy(&out.stderr).trim()).into());
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// The site a cookie host belongs to: `.mail.google.com` -> `google.com`.
/// ponytail: last two labels (three for `co.uk`-style suffixes), not the public suffix list.
pub fn site_of(host: &str) -> String {
    let host = host.trim_start_matches('.');
    let labels: Vec<&str> = host.split('.').collect();
    if labels.len() < 3 || host.parse::<std::net::IpAddr>().is_ok() {
        return host.to_string();
    }
    let n = labels.len();
    let short_suffix = labels[n - 1].len() == 2 && labels[n - 2].len() <= 3;
    labels[n - if short_suffix { 3 } else { 2 }..].join(".")
}

/// Sites as typed by a person or the settings menu; anything else could break the SQL.
fn valid_site(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
}

/// SQL matching the cookies of `sites` and their subdomains; every cookie when empty.
fn filter(sites: &[String]) -> String {
    if sites.is_empty() {
        return "1".into();
    }
    sites
        .iter()
        .map(|s| format!("(host_key = '{s}' OR host_key = '.{s}' OR host_key LIKE '%.{s}')"))
        .collect::<Vec<_>>()
        .join(" OR ")
}

/// Sites signed in to (or at least holding cookies) in Chrome, with their cookie counts.
pub fn sites(profile: &str) -> R<Vec<(String, u64)>> {
    let db = snapshot(profile)?;
    let rows = sqlite(&db, "SELECT host_key, COUNT(*) FROM cookies GROUP BY host_key")?;
    let _ = std::fs::remove_file(&db);
    let mut by_site: BTreeMap<String, u64> = BTreeMap::new();
    for line in rows.lines() {
        if let Some((host, n)) = line.rsplit_once('|') {
            *by_site.entry(site_of(host)).or_default() += n.parse::<u64>().unwrap_or(0);
        }
    }
    Ok(by_site.into_iter().collect())
}

/// Copy the cookies of `sites` (all when empty) into this browser; returns how many.
pub fn import(cfg: &Config, profile: &str, sites: &[String]) -> R<u64> {
    if let Some(bad) = sites.iter().find(|s| !valid_site(s)) {
        return Err(format!("not a site name: {bad:?}").into());
    }
    let exe = chrome::find_chrome(cfg)?;
    if !exe.to_string_lossy().to_lowercase().contains("google") {
        return Err(format!("logins can only be copied when herdr-web-browser runs Google Chrome (it runs {})", exe.display()).into());
    }
    let src = snapshot(profile)?;
    let src_sql = src.display().to_string().replace('\'', "''");
    let filter = filter(sites);
    let count: u64 = sqlite(&src, &format!("SELECT COUNT(*) FROM cookies WHERE {filter}"))?.trim().parse().unwrap_or(0);
    let dst = state::profile_dir().join("Default").join("Cookies");
    let mode = state::load()?.mode.unwrap_or_default();
    // The browser keeps its cookie database open, so swap rows in while it's stopped.
    let result = chrome::restart(mode, cfg, || {
        let _ = std::fs::remove_file(dst.with_file_name("Cookies-journal"));
        if dst.is_file() {
            sqlite(&dst, &format!("ATTACH '{src_sql}' AS src; INSERT OR REPLACE INTO main.cookies SELECT * FROM src.cookies WHERE {filter};"))?;
        } else {
            std::fs::create_dir_all(dst.parent().ok_or("bad profile path")?)?;
            std::fs::copy(&src, &dst)?;
            sqlite(&dst, &format!("DELETE FROM cookies WHERE NOT ({filter});"))?;
        }
        Ok(())
    });
    let _ = std::fs::remove_file(&src);
    result.map(|()| count)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hosts_group_by_site() {
        assert_eq!(site_of(".mail.google.com"), "google.com");
        assert_eq!(site_of("github.com"), "github.com");
        assert_eq!(site_of(".www.bbc.co.uk"), "bbc.co.uk");
        assert_eq!(site_of("localhost"), "localhost");
        assert_eq!(site_of("192.168.1.10"), "192.168.1.10");
    }

    #[test]
    fn site_names_with_quotes_are_refused() {
        assert!(valid_site("github.com"));
        assert!(!valid_site("x' OR 1=1 --"));
    }

    #[test]
    fn filter_covers_subdomains_and_all() {
        assert_eq!(filter(&[]), "1");
        assert!(filter(&["github.com".into()]).contains("LIKE '%.github.com'"));
    }
}
