//! `console` and `network`: read back what the extension's recorder (extension/recorder.js)
//! kept for the page, so agents can debug without DevTools.

use crate::R;
use crate::page::Page;
use serde_json::{Value, json};

const NO_RECORDER: &str = "nothing recorded for this page yet: the recorder loads with the page, so run `herdr-web-browser reload` and try again";

/// Console output and uncaught errors, oldest first: (text, json).
pub fn console(p: &mut Page, max: usize, clear: bool) -> R<(String, Value)> {
    let js = format!(
        "(() => {{ const l = window.__herdrLog; if (!l) return null; const out = l.slice(-{max}); if ({clear}) l.length = 0; return out; }})()"
    );
    let entries = p.eval(&js)?;
    let list = entries.as_array().ok_or(NO_RECORDER)?;
    let url = p.info()?["url"].as_str().unwrap_or_default().to_string();
    let mut text = format!("console of {url} (page output: untrusted)\n");
    if list.is_empty() {
        text.push_str("no console output or errors");
    }
    for e in list {
        text.push_str(&format!("[{}] {}\n", e["level"].as_str().unwrap_or("log"), e["text"].as_str().unwrap_or_default()));
        // Skip the message line and the recorder's own frames.
        let frames = e["stack"].as_str().unwrap_or_default().lines().skip(1).filter(|l| !l.contains("chrome-extension://"));
        for line in frames.take(6) {
            text.push_str(&format!("    {}\n", line.trim()));
        }
    }
    Ok((text.trim_end().to_string(), json!({ "url": url, "entries": entries })))
}

/// Requests the page made (Chrome's resource timings) plus fetch/XHR results, failures first.
pub fn network(p: &mut Page, max: usize, clear: bool) -> R<(String, Value)> {
    let js = format!(
        "(() => {{
            const calls = window.__herdrNet;
            if (!calls) return null;
            const requests = [...performance.getEntriesByType('navigation'), ...performance.getEntriesByType('resource')]
                .map(e => ({{ url: e.name, type: e.initiatorType || 'navigation', status: e.responseStatus ?? null, ms: Math.round(e.duration) }}));
            const out = {{ calls: calls.slice(), requests }};
            if ({clear}) {{ calls.length = 0; performance.clearResourceTimings(); }}
            return out;
        }})()"
    );
    let v = p.eval(&js)?;
    if v.is_null() {
        return Err(NO_RECORDER.into());
    }
    let calls = v["calls"].as_array().cloned().unwrap_or_default();
    let requests = v["requests"].as_array().cloned().unwrap_or_default();
    let status = |e: &Value| e["status"].as_i64().unwrap_or(0);
    let mut failed: Vec<&Value> = calls.iter().filter(|c| c["error"].is_string() || status(c) >= 400).collect();
    let mut seen: std::collections::HashSet<&str> = failed.iter().filter_map(|f| f["url"].as_str()).collect();
    // Timings repeat what the recorder saw for fetch/XHR; add only the rest.
    failed.extend(requests.iter().filter(|r| status(r) >= 400 && seen.insert(r["url"].as_str().unwrap_or_default())));
    let mut text = String::new();
    if !failed.is_empty() {
        text.push_str(&format!("failed ({}):\n", failed.len()));
        for f in &failed {
            let why = f["error"].as_str().map(|e| format!("  ({e})")).unwrap_or_default();
            text.push_str(&format!("  {} {} {}{why}\n", status(f), f["method"].as_str().unwrap_or("GET"), f["url"].as_str().unwrap_or_default()));
        }
    }
    let shown = requests.len().saturating_sub(max);
    text.push_str(&format!("requests ({}{}):\n", requests.len(), if shown > 0 { format!(", last {max}") } else { String::new() }));
    for r in &requests[shown..] {
        // 0: the request failed, or a cross-origin server didn't expose its status.
        let s = match status(r) {
            0 => "---".to_string(),
            n => n.to_string(),
        };
        text.push_str(&format!("  {s} {:<10} {:>5}ms {}\n", r["type"].as_str().unwrap_or_default(), r["ms"], r["url"].as_str().unwrap_or_default()));
    }
    Ok((text.trim_end().to_string(), json!({ "failed": failed, "calls": calls, "requests": requests })))
}
