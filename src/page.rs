//! Tab resolution and page operations over CDP.

use crate::cdp::{Cdp, DEFAULT_TIMEOUT};
use crate::state::{self, Config};
use crate::{R, chrome, host};
use serde_json::{Value, json};
use std::thread::sleep;
use std::time::{Duration, Instant};

const COMMON_JS: &str = include_str!("js/common.js");
const SNAPSHOT_JS: &str = include_str!("js/snapshot.js");
const PICK_JS: &str = include_str!("js/pick.js");

/// Wrap a script so the shared helpers are scoped to this one evaluation.
fn script(prelude: &str, body: &str) -> String {
    format!("(() => {{ {prelude}\n{COMMON_JS}\nreturn (\n{body}\n); }})()")
}

#[derive(Debug, Clone)]
pub struct Tab {
    pub id: String,
    pub url: String,
    pub title: String,
}

pub fn page_targets(c: &mut Cdp) -> R<Vec<Tab>> {
    let r = c.call("Target.getTargets", json!({}), None)?;
    let infos = r["targetInfos"].as_array().cloned().unwrap_or_default();
    Ok(infos
        .iter()
        .filter(|t| t["type"] == "page")
        .filter(|t| !t["url"].as_str().unwrap_or("").starts_with("devtools://"))
        .map(|t| Tab {
            id: t["targetId"].as_str().unwrap_or_default().into(),
            url: t["url"].as_str().unwrap_or_default().into(),
            title: t["title"].as_str().unwrap_or_default().into(),
        })
        .collect())
}

pub fn create_tab(c: &mut Cdp, url: &str) -> R<String> {
    // Headless tabs sharing a window are hidden unless active, and hidden pages don't
    // paint, so live views would stay blank: give each its own window instead.
    let headless = state::load()?.mode == Some(state::Mode::Headless);
    let r = c.call("Target.createTarget", json!({ "url": url, "newWindow": headless }), None)?;
    r["targetId"].as_str().map(String::from).ok_or_else(|| "createTarget returned no id".into())
}

/// Set by `--owner`: act as if called from that pane/terminal (the live view, desktop mods).
pub static OWNER: std::sync::OnceLock<String> = std::sync::OnceLock::new();

/// Who owns the caller's tab: `--owner`, a herdr TUI pane, or a herdr-core terminal.
pub fn caller_pane() -> Option<String> {
    let env = |v| std::env::var(v).ok().filter(|p: &String| !p.is_empty());
    OWNER.get().cloned().or_else(|| env("HERDR_PANE_ID")).or_else(|| env("HC_TERMINAL_ID"))
}

/// Close tabs owned by herdr-core terminals that no longer exist. TUI panes are
/// cleaned by the plugin's pane.closed hook; herdr desktop has no such hook.
pub fn gc(c: &mut Cdp) -> R<()> {
    let Some(live) = host::live_hc_terminals() else { return Ok(()) };
    let dead = state::update(|s| {
        let dead: Vec<(String, String)> =
            s.tabs.iter().filter(|(o, _)| host::is_hc_owner(o) && !live.contains(o)).map(|(o, t)| (o.clone(), t.clone())).collect();
        for (o, _) in &dead {
            s.tabs.remove(o);
        }
        dead
    })?;
    for (_, tab) in dead {
        let _ = c.call("Target.closeTarget", json!({ "targetId": tab }), None);
    }
    Ok(())
}

/// The tab a caller should act on; see `resolve`.
pub struct Page {
    pub cdp: Cdp,
    pub tab: Tab,
    pub session: String,
}

/// Which tab a command acts on:
/// `--tab n` (1-based, as listed by `tabs`) > the calling pane's own tab
/// (created on first use when `create`) > the tab the user is looking at.
pub fn resolve(cfg: &Config, tab: Option<usize>, create: bool) -> R<Page> {
    let url = chrome::ensure(cfg)?;
    let mut c = Cdp::connect(&url)?;
    let tabs = page_targets(&mut c)?;
    let chosen = if let Some(n) = tab {
        tabs.get(n.wrapping_sub(1)).cloned().ok_or_else(|| format!("no tab {n}; run `herdr-browser tabs`"))?
    } else if let Some(pane) = caller_pane() {
        let owned = state::load()?.tabs.get(&pane).cloned();
        match owned.and_then(|id| tabs.iter().find(|t| t.id == id).cloned()) {
            Some(t) => t,
            None if create => {
                gc(&mut c)?;
                let id = create_tab(&mut c, "about:blank")?;
                state::update(|s| s.tabs.insert(pane.clone(), id.clone()))?;
                Tab { id, url: "about:blank".into(), title: String::new() }
            }
            None => focused_tab(&mut c, &tabs)?,
        }
    } else {
        focused_tab(&mut c, &tabs)?
    };
    let r = c.call("Target.attachToTarget", json!({ "targetId": chosen.id, "flatten": true }), None)?;
    let session = r["sessionId"].as_str().ok_or("attachToTarget returned no session")?.to_string();
    Ok(Page { cdp: c, tab: chosen, session })
}

/// The tab with focus, else a visible one, else the first.
fn focused_tab(c: &mut Cdp, tabs: &[Tab]) -> R<Tab> {
    if tabs.is_empty() {
        let id = create_tab(c, "about:blank")?;
        return Ok(Tab { id, url: "about:blank".into(), title: String::new() });
    }
    let mut visible = None;
    for t in tabs {
        let Ok(r) = c.call("Target.attachToTarget", json!({ "targetId": t.id, "flatten": true }), None) else {
            continue;
        };
        let sid = r["sessionId"].as_str().unwrap_or_default().to_string();
        let v = c.eval(&sid, "[document.visibilityState, document.hasFocus()]", Duration::from_secs(2));
        let _ = c.call("Target.detachFromTarget", json!({ "sessionId": sid }), None);
        match v {
            Ok(v) if v[1] == true => return Ok(t.clone()),
            Ok(v) if v[0] == "visible" && visible.is_none() => visible = Some(t.clone()),
            _ => {}
        }
    }
    Ok(visible.unwrap_or_else(|| tabs[0].clone()))
}

pub fn normalize_url(url: &str) -> String {
    let has_scheme = url.contains("://") || ["about:", "data:", "file:", "chrome:", "javascript:"].iter().any(|p| url.starts_with(p));
    if has_scheme {
        return url.to_string();
    }
    // Local dev servers rarely have TLS: localhost, loopback, LAN and *.local get http.
    let host = url.split(['/', '?', '#']).next().unwrap_or("");
    let name = host.rsplit_once(':').filter(|(_, port)| port.chars().all(|c| c.is_ascii_digit())).map_or(host, |(h, _)| h);
    let local = name == "localhost"
        || name.ends_with(".localhost")
        || name.ends_with(".local")
        || name == "[::1]"
        || name.parse::<std::net::Ipv4Addr>().is_ok_and(|ip| ip.is_loopback() || ip.is_private() || ip.is_unspecified());
    format!("{}://{url}", if local { "http" } else { "https" })
}

impl Page {
    pub fn eval(&mut self, expr: &str) -> R<Value> {
        self.cdp.eval(&self.session, expr, DEFAULT_TIMEOUT)
    }

    fn call(&mut self, method: &str, params: Value) -> R<Value> {
        let sid = self.session.clone();
        self.cdp.call(method, params, Some(&sid))
    }

    /// Wait for `document.readyState == "complete"`, tolerating context swaps mid-navigation.
    pub fn wait_ready(&mut self) {
        let deadline = Instant::now() + Duration::from_secs(30);
        while Instant::now() < deadline {
            if let Ok(v) = self.cdp.eval(&self.session, "document.readyState", Duration::from_secs(2))
                && v == "complete"
            {
                return;
            }
            sleep(Duration::from_millis(100));
        }
    }

    pub fn info(&mut self) -> R<Value> {
        self.eval("({ url: location.href, title: document.title })")
    }

    pub fn navigate(&mut self, url: &str) -> R<Value> {
        let r = self.call("Page.navigate", json!({ "url": normalize_url(url) }))?;
        if let Some(e) = r["errorText"].as_str().filter(|e| !e.is_empty()) {
            return Err(format!("navigation failed: {e}").into());
        }
        self.wait_ready();
        self.info()
    }

    pub fn back(&mut self) -> R<Value> {
        self.eval("history.back()")?;
        sleep(Duration::from_millis(300));
        self.wait_ready();
        self.info()
    }

    pub fn forward(&mut self) -> R<Value> {
        self.eval("history.forward()")?;
        sleep(Duration::from_millis(300));
        self.wait_ready();
        self.info()
    }

    pub fn reload(&mut self) -> R<Value> {
        self.call("Page.reload", json!({}))?;
        sleep(Duration::from_millis(300));
        self.wait_ready();
        self.info()
    }

    pub fn activate(&mut self) -> R<()> {
        let id = self.tab.id.clone();
        self.cdp.call("Target.activateTarget", json!({ "targetId": id }), None)?;
        self.call("Page.bringToFront", json!({}))?;
        Ok(())
    }

    pub fn text(&mut self) -> R<String> {
        Ok(self.eval("document.body ? document.body.innerText : ''")?.as_str().unwrap_or_default().to_string())
    }

    pub fn selection(&mut self) -> R<String> {
        Ok(self.eval("String(getSelection())")?.as_str().unwrap_or_default().to_string())
    }

    pub fn snapshot(&mut self) -> R<Value> {
        self.eval(&script("", SNAPSHOT_JS))
    }

    /// Scroll the element into view and return its centre in viewport coordinates.
    fn locate(&mut self, target: &str) -> R<(f64, f64)> {
        let t = serde_json::to_string(target)?;
        let v = self.eval(&script(
            "",
            &format!(
                "(() => {{ const el = __hb.resolve({t}); el.scrollIntoView({{ block: 'center', inline: 'center' }});\
                 const r = el.getBoundingClientRect(); return [r.left + r.width / 2, r.top + r.height / 2]; }})()"
            ),
        ))?;
        Ok((v[0].as_f64().unwrap_or(0.0), v[1].as_f64().unwrap_or(0.0)))
    }

    /// Real (trusted) mouse click at the element's centre.
    pub fn click(&mut self, target: &str) -> R<()> {
        let (x, y) = self.locate(target)?;
        for kind in ["mouseMoved", "mousePressed", "mouseReleased"] {
            self.call(
                "Input.dispatchMouseEvent",
                json!({ "type": kind, "x": x, "y": y, "button": "left", "buttons": 1, "clickCount": 1 }),
            )?;
        }
        sleep(Duration::from_millis(200));
        self.wait_ready();
        Ok(())
    }

    /// Replace the field's contents with `text` (fill semantics), optionally pressing Enter.
    pub fn type_text(&mut self, target: &str, text: &str, submit: bool) -> R<()> {
        let t = serde_json::to_string(target)?;
        self.eval(&script(
            "",
            &format!(
                "(() => {{ const el = __hb.resolve({t}); el.scrollIntoView({{ block: 'center' }}); el.focus();\
                 if ('value' in el) el.select ? el.select() : (el.value = '');\
                 else if (el.isContentEditable) document.execCommand('selectAll'); return true; }})()"
            ),
        ))?;
        self.call("Input.insertText", json!({ "text": text }))?;
        if submit {
            for kind in ["keyDown", "keyUp"] {
                self.call(
                    "Input.dispatchKeyEvent",
                    json!({ "type": kind, "key": "Enter", "code": "Enter", "windowsVirtualKeyCode": 13, "text": "\r" }),
                )?;
            }
            sleep(Duration::from_millis(300));
            self.wait_ready();
        }
        Ok(())
    }

    pub fn screenshot(&mut self) -> R<Vec<u8>> {
        use base64::Engine;
        // Background tabs in a headed browser don't paint, so bring it forward first.
        if state::load()?.mode != Some(state::Mode::Headless) {
            self.activate()?;
        }
        let r = self.call("Page.captureScreenshot", json!({ "format": "png" }))?;
        let data = r["data"].as_str().ok_or("captureScreenshot returned no data")?;
        Ok(base64::engine::general_purpose::STANDARD.decode(data)?)
    }

    pub fn wait_for(&mut self, css: &str, timeout: Duration) -> R<()> {
        let sel = serde_json::to_string(css)?;
        let ms = timeout.as_millis();
        let found = self.cdp.eval(
            &self.session,
            &script(
                "",
                &format!(
                    "new Promise((ok) => {{ const end = Date.now() + {ms}; (function poll() {{\
                     const el = document.querySelector({sel}); if (el && __hb.visible(el)) return ok(true);\
                     if (Date.now() > end) return ok(false); setTimeout(poll, 100); }})(); }})"
                ),
            ),
            timeout + Duration::from_secs(5),
        )?;
        if found == true { Ok(()) } else { Err(format!("timed out waiting for {css}").into()) }
    }

    pub fn pick(&mut self, prompt: &str, timeout: Duration) -> R<Value> {
        if state::load()?.mode == Some(state::Mode::Headless) {
            return Err("pick needs a visible browser: run `herdr-browser mode headed`".into());
        }
        self.activate()?;
        let prelude = format!(
            "const PROMPT = {}; const TIMEOUT_MS = {};",
            serde_json::to_string(prompt)?,
            timeout.as_millis()
        );
        let v = self.cdp.eval(&self.session, &script(&prelude, PICK_JS), timeout + Duration::from_secs(5))?;
        if v["cancelled"] == true {
            return Err("pick cancelled by the user".into());
        }
        if v["timeout"] == true {
            return Err(format!("pick timed out after {}s", timeout.as_secs()).into());
        }
        Ok(v)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bare_hosts_get_https() {
        assert_eq!(normalize_url("example.com/x"), "https://example.com/x");
    }

    #[test]
    fn local_dev_servers_get_http() {
        for u in ["localhost:3000", "127.0.0.1:8080/app", "192.168.1.20:5173", "myapp.local", "[::1]:4000"] {
            assert_eq!(normalize_url(u), format!("http://{u}"));
        }
        assert_eq!(normalize_url("example.com:8443/x"), "https://example.com:8443/x");
    }

    #[test]
    fn explicit_schemes_are_kept() {
        for u in ["http://a.b", "about:blank", "data:text/html,hi", "file:///tmp/x"] {
            assert_eq!(normalize_url(u), u);
        }
    }
}
