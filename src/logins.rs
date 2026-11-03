//! `import-logins`: copy the user's Google Chrome sign-ins (cookies) into this browser.
//! Chrome encrypts cookie values with a key kept in the macOS Keychain ("Chrome Safe
//! Storage"; on Linux the Secret Service, or a built-in password), and this browser has its
//! own key, so we decrypt with Chrome's (macOS asks the user once) and hand the cookies over
//! CDP, which stores them this browser's way. Reads Chrome's database with `sqlite3`.

use crate::R;
use crate::cdp::Cdp;
use crate::chrome;
use crate::state::{self, Config};
use aes::cipher::{BlockModeDecrypt, KeyIvInit, block_padding::Pkcs7};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

type Aes128CbcDec = cbc::Decryptor<aes::Aes128>;

/// Seconds between 1601-01-01 (Chrome's epoch) and 1970-01-01.
const WINDOWS_TO_UNIX_SECS: i64 = 11_644_473_600;

/// Google Chrome's own data folder (not Chrome for Testing's).
fn chrome_dir() -> R<PathBuf> {
    let home = PathBuf::from(std::env::var_os("HOME").ok_or("HOME is not set")?);
    Ok(if cfg!(target_os = "macos") { home.join("Library/Application Support/Google/Chrome") } else { home.join(".config/google-chrome") })
}

/// Chrome's profiles as (folder, name) from its `Local State`, the last used first.
pub fn profiles() -> R<Vec<(String, String)>> {
    let path = chrome_dir()?.join("Local State");
    let raw = std::fs::read(&path).map_err(|e| format!("can't read {}: {e}", path.display()))?;
    Ok(parse_profiles(&serde_json::from_slice(&raw)?))
}

fn parse_profiles(st: &Value) -> Vec<(String, String)> {
    let last = st["profile"]["last_used"].as_str().unwrap_or("Default");
    let mut out: Vec<(String, String)> = st["profile"]["info_cache"]
        .as_object()
        .map(|m| m.iter().map(|(dir, v)| (dir.clone(), v["name"].as_str().unwrap_or(dir).to_string())).collect())
        .unwrap_or_default();
    out.sort_by_key(|(dir, _)| dir != last);
    if out.is_empty() {
        out.push(("Default".into(), "Default".into()));
    }
    out
}

/// The profile to read when none is named: the one Chrome used last.
pub fn default_profile() -> String {
    profiles().ok().and_then(|p| p.into_iter().next()).map_or_else(|| "Default".into(), |(dir, _)| dir)
}

/// Google Chrome's cookie database for `profile`: a profile folder name ("Default",
/// "Profile 1", ...) or a path to a profile folder. Newer Chrome keeps it under
/// `Network/`; with both present, the one written last is the live one.
fn chrome_cookies(profile: &str) -> R<PathBuf> {
    let dir = chrome_dir()?.join(profile); // an absolute `profile` replaces Chrome's folder
    let modified = |p: &PathBuf| std::fs::metadata(p).and_then(|m| m.modified()).ok();
    [dir.join("Network/Cookies"), dir.join("Cookies")]
        .into_iter()
        .filter(|p| p.is_file())
        .max_by_key(modified)
        .ok_or_else(|| format!("no Google Chrome cookies in {}", dir.display()).into())
}

/// A copy, so Chrome can keep its own database open while we read.
fn snapshot(profile: &str) -> R<PathBuf> {
    let src = chrome_cookies(profile)?;
    std::fs::create_dir_all(state::state_dir())?;
    let copy = state::state_dir().join("chrome-cookies.sqlite");
    std::fs::copy(&src, &copy).map_err(|e| format!("can't read {}: {e}", src.display()))?;
    Ok(copy)
}

/// Rows of `sql` as JSON objects.
fn query(db: &Path, sql: &str) -> R<Vec<Value>> {
    let out = Command::new("sqlite3").args(["-batch", "-json"]).arg(db).arg(sql).output().map_err(|e| format!("sqlite3: {e}"))?;
    if !out.status.success() {
        return Err(format!("sqlite3: {}", String::from_utf8_lossy(&out.stderr).trim()).into());
    }
    if out.stdout.iter().all(u8::is_ascii_whitespace) {
        return Ok(Vec::new()); // -json prints nothing for no rows
    }
    Ok(serde_json::from_slice(&out.stdout)?)
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
    sites.iter().map(|s| format!("(host_key = '{s}' OR host_key = '.{s}' OR host_key LIKE '%.{s}')")).collect::<Vec<_>>().join(" OR ")
}

/// Sites holding cookies in Chrome (signed in, mostly), with their cookie counts.
pub fn sites(profile: &str) -> R<Vec<(String, u64)>> {
    let db = snapshot(profile)?;
    let rows = query(&db, "SELECT host_key AS h, COUNT(*) AS n FROM cookies GROUP BY host_key");
    let _ = std::fs::remove_file(&db);
    let mut by_site: BTreeMap<String, u64> = BTreeMap::new();
    for r in rows? {
        *by_site.entry(site_of(r["h"].as_str().unwrap_or_default())).or_default() += r["n"].as_u64().unwrap_or(0);
    }
    Ok(by_site.into_iter().collect())
}

/// Chrome's AES key for values prefixed `prefix` ("v10"/"v11").
fn chrome_key(prefix: &str) -> R<[u8; 16]> {
    let (password, iterations) = if cfg!(target_os = "macos") {
        let out = Command::new("security").args(["find-generic-password", "-w", "-s", "Chrome Safe Storage", "-a", "Chrome"]).output()?;
        if !out.status.success() {
            return Err("couldn't read Chrome's cookie key from the Keychain; click Allow when macOS asks".into());
        }
        (String::from_utf8_lossy(&out.stdout).trim().to_string(), 1003)
    } else if prefix == "v11" {
        let out = Command::new("secret-tool").args(["lookup", "application", "chrome"]).output()?;
        (String::from_utf8_lossy(&out.stdout).trim().to_string(), 1)
    } else {
        ("peanuts".to_string(), 1) // Linux without a keyring: Chrome's fixed password
    };
    let mut key = [0u8; 16];
    pbkdf2::pbkdf2_hmac::<sha1::Sha1>(password.as_bytes(), b"saltysalt", iterations, &mut key);
    Ok(key)
}

/// Decrypt one `encrypted_value`. Databases from version 24 prefix the value with a
/// 32-byte SHA-256 of the cookie's domain.
fn decrypt(enc: &[u8], key: &[u8; 16], domain_hash: bool) -> R<String> {
    let mut buf = enc.get(3..).ok_or("encrypted value too short")?.to_vec();
    let plain =
        Aes128CbcDec::new(key.into(), &[b' '; 16].into()).decrypt_padded::<Pkcs7>(&mut buf).map_err(|_| "couldn't decrypt a cookie (wrong key?)")?;
    let value = if domain_hash { plain.get(32..).unwrap_or_default() } else { plain };
    Ok(String::from_utf8_lossy(value).into_owned())
}

fn unhex(s: &str) -> Vec<u8> {
    (0..s.len() / 2).filter_map(|i| u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).ok()).collect()
}

/// A Chrome cookie row as CDP's `CookieParam`.
fn cookie_param(r: &Value, value: String) -> Value {
    let host = r["host_key"].as_str().unwrap_or_default();
    let path = r["path"].as_str().unwrap_or("/");
    let secure = r["is_secure"].as_i64() == Some(1);
    let mut c = json!({ "name": r["name"], "value": value, "path": path, "secure": secure, "httpOnly": r["is_httponly"].as_i64() == Some(1) });
    if host.starts_with('.') {
        c["domain"] = json!(host);
    } else {
        // Host-only: set through a URL so it doesn't spread to subdomains.
        c["url"] = json!(format!("{}://{host}{path}", if secure { "https" } else { "http" }));
    }
    if let Some(s) = match r["samesite"].as_i64() {
        Some(0) => Some("None"),
        Some(1) => Some("Lax"),
        Some(2) => Some("Strict"),
        _ => None,
    } {
        c["sameSite"] = json!(s);
    }
    if r["has_expires"].as_i64() == Some(1) {
        c["expires"] = json!(r["expires_utc"].as_i64().unwrap_or(0) / 1_000_000 - WINDOWS_TO_UNIX_SECS);
    }
    c
}

/// Copy the cookies of `sites` (all when empty) into this browser; returns how many.
pub fn import(cfg: &Config, profile: &str, sites: &[String]) -> R<usize> {
    if let Some(bad) = sites.iter().find(|s| !valid_site(s)) {
        return Err(format!("not a site name: {bad:?}").into());
    }
    let src = snapshot(profile)?;
    let rows = query(&src, "SELECT value FROM meta WHERE key = 'version'").and_then(|v| {
        let version: i64 = v.first().and_then(|r| r["value"].as_str()).and_then(|s| s.parse().ok()).unwrap_or(0);
        let sql = format!(
            "SELECT host_key, name, path, hex(encrypted_value) AS enc, value, is_secure, is_httponly, samesite, expires_utc, has_expires FROM cookies WHERE {}",
            filter(sites)
        );
        Ok((version, query(&src, &sql)?))
    });
    let _ = std::fs::remove_file(&src);
    let (version, rows) = rows?;
    let mut keys: BTreeMap<String, [u8; 16]> = BTreeMap::new();
    let mut cookies = Vec::with_capacity(rows.len());
    for r in &rows {
        let enc = unhex(r["enc"].as_str().unwrap_or_default());
        let value = if enc.is_empty() {
            r["value"].as_str().unwrap_or_default().to_string()
        } else {
            let prefix = String::from_utf8_lossy(&enc[..enc.len().min(3)]).into_owned();
            if !keys.contains_key(&prefix) {
                keys.insert(prefix.clone(), chrome_key(&prefix)?);
            }
            decrypt(&enc, &keys[&prefix], version >= 24)?
        };
        cookies.push(cookie_param(r, value));
    }
    let mut c = Cdp::connect(&chrome::ensure(cfg)?)?;
    for chunk in cookies.chunks(200) {
        c.call("Storage.setCookies", json!({ "cookies": chunk }), None)?;
    }
    Ok(cookies.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use aes::cipher::BlockModeEncrypt;

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

    #[test]
    fn decrypts_a_chrome_v10_value_and_drops_the_domain_hash() {
        let mut key = [0u8; 16];
        pbkdf2::pbkdf2_hmac::<sha1::Sha1>(b"peanuts", b"saltysalt", 1, &mut key);
        let plain = [&[7u8; 32][..], b"secret123"].concat();
        let mut buf = vec![0u8; plain.len() + 16];
        let n = cbc::Encryptor::<aes::Aes128>::new(&key.into(), &[b' '; 16].into()).encrypt_padded_b2b::<Pkcs7>(&plain, &mut buf).unwrap().len();
        let enc = [b"v10", &buf[..n]].concat();
        assert_eq!(decrypt(&enc, &key, true).unwrap(), "secret123");
        assert!(decrypt(&enc, &[0u8; 16], true).is_err());
    }

    #[test]
    fn profiles_list_the_last_used_first() {
        let st = json!({ "profile": { "last_used": "Profile 2", "info_cache": {
            "Default": { "name": "Personal" }, "Profile 2": { "name": "Work" } } } });
        assert_eq!(parse_profiles(&st), [("Profile 2".to_string(), "Work".to_string()), ("Default".into(), "Personal".into())]);
        assert_eq!(parse_profiles(&json!({})), [("Default".to_string(), "Default".to_string())]);
    }

    #[test]
    fn expiry_moves_from_chrome_to_unix_time() {
        let row = json!({ "host_key": ".github.com", "name": "n", "path": "/", "is_secure": 1, "is_httponly": 0, "samesite": 1, "has_expires": 1, "expires_utc": 13_400_000_000_000_000i64 });
        let c = cookie_param(&row, "v".into());
        assert_eq!((c["domain"].as_str(), c["sameSite"].as_str(), c["expires"].as_i64()), (Some(".github.com"), Some("Lax"), Some(1_755_526_400)));
    }
}
