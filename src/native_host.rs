//! Chrome native-messaging host: one request from the extension, one reply.
//! Wire format: 4-byte native-endian (little on our targets) length, then UTF-8 JSON.

use crate::agent::{self, deliver, save_shot_in, screenshot_prompt, wrap};
use crate::R;
use crate::state::Config;
use base64::Engine;
use serde::Deserialize;
use serde_json::{Value, json};
use std::io::{Read, Write};

/// Chrome caps extension→host messages at 64 MiB.
const MAX_MSG: usize = 64 * 1024 * 1024;

pub fn read_msg(r: &mut impl Read) -> R<Option<Value>> {
    let mut len = [0u8; 4];
    match r.read_exact(&mut len) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e.into()),
    }
    let n = u32::from_le_bytes(len) as usize;
    if n > MAX_MSG {
        return Err(format!("native message too large: {n} bytes").into());
    }
    let mut buf = vec![0; n];
    r.read_exact(&mut buf)?;
    Ok(Some(serde_json::from_slice(&buf)?))
}

pub fn write_msg(w: &mut impl Write, v: &Value) -> R<()> {
    let body = serde_json::to_vec(v)?;
    w.write_all(&(body.len() as u32).to_le_bytes())?;
    w.write_all(&body)?;
    w.flush()?;
    Ok(())
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Kind {
    Selection,
    Page,
    Link,
    Screenshot,
}

#[derive(Debug, Deserialize)]
struct Request {
    kind: Kind,
    url: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    data_url: Option<String>,
}

fn handle(msg: Value, cfg: &Config) -> R<String> {
    let req: Request = serde_json::from_value(msg).map_err(|e| format!("bad request from extension: {e}"))?;
    let (label, text, chars) = match req.kind {
        Kind::Screenshot => {
            let data = req.data_url.as_deref().and_then(|d| d.strip_prefix("data:image/png;base64,")).ok_or("screenshot without PNG data")?;
            let png = base64::engine::general_purpose::STANDARD.decode(data)?;
            let path = save_shot_in(&agent::shots_dir(), &png)?;
            let t = screenshot_prompt(&req.url, &path);
            ("screenshot", t.clone(), t.chars().count())
        }
        kind => {
            let body = req.text.unwrap_or_default();
            if body.trim().is_empty() {
                return Err("nothing to send".into());
            }
            let label = match kind {
                Kind::Selection => "selection",
                Kind::Link => "link",
                _ => "page",
            };
            (label, wrap(label, &req.url, &req.title, &body, cfg.max_chars()), body.chars().count())
        }
    };
    let (host, target) = agent::resolve_target(None, cfg)?;
    deliver(host, &target, label, &text, chars)?;
    Ok(target)
}

pub fn run() -> R<()> {
    let mut stdin = std::io::stdin().lock();
    let mut stdout = std::io::stdout().lock();
    while let Some(msg) = read_msg(&mut stdin)? {
        let reply = match Config::load().and_then(|cfg| handle(msg, &cfg)) {
            Ok(agent) => json!({ "ok": true, "agent": agent }),
            Err(e) => {
                crate::host::notify(crate::host::host(), "herdr-browser: send failed", &e.to_string());
                json!({ "ok": false, "error": e.to_string() })
            }
        };
        write_msg(&mut stdout, &reply)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn framing_round_trips() {
        let mut buf = Vec::new();
        write_msg(&mut buf, &json!({ "kind": "page", "url": "u" })).unwrap();
        assert_eq!(&buf[..4], &(buf.len() as u32 - 4).to_le_bytes());
        let mut r = buf.as_slice();
        assert_eq!(read_msg(&mut r).unwrap(), Some(json!({ "kind": "page", "url": "u" })));
        assert_eq!(read_msg(&mut r).unwrap(), None);
    }

    #[test]
    fn oversized_length_is_rejected() {
        let mut r: &[u8] = &u32::MAX.to_le_bytes();
        assert!(read_msg(&mut r).is_err());
    }

    #[test]
    fn unknown_kind_is_rejected() {
        let e = handle(json!({ "kind": "cookies", "url": "u" }), &Config::default()).unwrap_err();
        assert!(e.to_string().contains("bad request"));
    }
}
