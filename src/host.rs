//! The two herdr front ends an agent can live in, behind one interface:
//! - `Tui`: the `herdr` terminal server (plugins, `herdr agent prompt`).
//! - `Desktop`: Herdr.app on herdr-core (`hc send`, `herdr-app` mods).

use crate::R;
use crate::agent::Agent;
use crate::state;
use serde_json::Value;
use std::io::Read;
use std::process::{Command, Stdio};
use std::thread::sleep;
use std::time::{Duration, Instant};

const CLI_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Host {
    Tui,
    Desktop,
}

fn env(var: &str) -> Option<String> {
    std::env::var(var).ok().filter(|v| !v.is_empty())
}

fn herdr_bin() -> String {
    env("HERDR_BIN_PATH").unwrap_or_else(|| "herdr".into())
}

/// The host the calling process runs in, from the env each host injects.
pub fn caller_host() -> Option<Host> {
    if env("HERDR_PANE_ID").is_some() {
        Some(Host::Tui)
    } else if env("HC_TERMINAL_ID").is_some() {
        Some(Host::Desktop)
    } else {
        None
    }
}

/// Caller's host, else the desktop when its window answers, else the TUI.
pub fn host() -> Host {
    caller_host().unwrap_or_else(|| if run("herdr-app", &["ls"]).is_ok() { Host::Desktop } else { Host::Tui })
}

/// Run a CLI with a deadline; parse stdout as JSON (Null when not JSON).
pub fn run(bin: &str, args: &[&str]) -> R<Value> {
    let mut child = Command::new(bin)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("cannot run {bin}: {e}"))?;
    let mut out = child.stdout.take().expect("piped");
    let mut err = child.stderr.take().expect("piped");
    let out_t = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = out.read_to_string(&mut s);
        s
    });
    let err_t = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = err.read_to_string(&mut s);
        s
    });
    let deadline = Instant::now() + CLI_TIMEOUT;
    let status = loop {
        if let Some(st) = child.try_wait()? {
            break st;
        }
        if Instant::now() > deadline {
            let _ = child.kill();
            return Err(format!("{bin} {} timed out", args.join(" ")).into());
        }
        sleep(Duration::from_millis(20));
    };
    let (stdout, stderr) = (out_t.join().unwrap_or_default(), err_t.join().unwrap_or_default());
    if !status.success() {
        let msg = if stderr.trim().is_empty() { stdout.trim() } else { stderr.trim() };
        return Err(format!("{bin} {}: {msg}", args.first().unwrap_or(&"")).into());
    }
    Ok(serde_json::from_str(&stdout).unwrap_or(Value::Null))
}

pub fn herdr(args: &[&str]) -> R<Value> {
    run(&herdr_bin(), args)
}

fn s(v: &Value, k: &str) -> String {
    v[k].as_str().unwrap_or_default().to_string()
}

/// `herdr agent list`.
pub fn parse_tui_agents(v: &Value) -> Vec<Agent> {
    v.pointer("/result/agents")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|a| {
            let pane_id = s(a, "pane_id");
            Agent {
                id: a["name"].as_str().filter(|n| !n.is_empty()).map(String::from).unwrap_or_else(|| pane_id.clone()),
                workspace_id: s(a, "workspace_id"),
                kind: s(a, "agent"),
                status: s(a, "agent_status"),
                title: s(a, "terminal_title_stripped"),
                cwd: s(a, "cwd"),
                pane_id,
            }
        })
        .collect()
}

/// `herdr-app ls`: terminals that report a status are agents; also returns the active space.
pub fn parse_desktop_agents(v: &Value) -> (Vec<Agent>, Option<String>) {
    let mut agents = Vec::new();
    let mut active = None;
    for sp in v["spaces"].as_array().into_iter().flatten() {
        if sp["active"] == true {
            active = Some(s(sp, "id"));
        }
        for tab in sp["tabs"].as_array().into_iter().flatten() {
            for t in tab["terminals"].as_array().into_iter().flatten().filter(|t| t["status"].is_string()) {
                let addr = s(t, "terminal");
                agents.push(Agent {
                    id: addr.clone(),
                    pane_id: addr,
                    workspace_id: s(sp, "id"),
                    kind: s(t, "app"),
                    status: s(t, "status"),
                    title: s(sp, "name"),
                    cwd: String::new(),
                });
            }
        }
    }
    (agents, active)
}

/// `hc ls --json` (window closed): agents only, no notion of focus.
pub fn parse_hc_agents(v: &Value) -> Vec<Agent> {
    v["terminals"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|t| t["state"] == "live" && t["status"].is_string())
        .map(|t| Agent {
            id: s(t, "address"),
            pane_id: s(t, "address"),
            workspace_id: String::new(),
            kind: s(t, "agent"),
            status: s(t, "status"),
            title: s(t, "where"),
            cwd: s(t, "cwd"),
        })
        .collect()
}

/// Live agents on `host` and the focused workspace/space id.
pub fn agents(host: Host) -> R<(Vec<Agent>, Option<String>)> {
    match host {
        Host::Tui => {
            let agents = parse_tui_agents(&herdr(&["agent", "list"])?);
            let ws = herdr(&["workspace", "list"])?;
            let focused = ws
                .pointer("/result/workspaces")
                .and_then(Value::as_array)
                .and_then(|w| w.iter().find(|w| w["focused"] == true))
                .map(|w| s(w, "workspace_id"));
            Ok((agents, focused))
        }
        Host::Desktop => match run("herdr-app", &["ls"]) {
            Ok(v) => Ok(parse_desktop_agents(&v)),
            Err(_) => Ok((parse_hc_agents(&run("hc", &["ls", "--json"])?), None)),
        },
    }
}

pub fn prompt(host: Host, target: &str, text: &str) -> R<()> {
    match host {
        Host::Tui => herdr(&["agent", "prompt", target, text]).map(drop),
        Host::Desktop => run("hc", &["send", target, text]).map(drop),
    }
}

/// Best effort: a failed notification never fails the send it reports.
pub fn notify(host: Host, title: &str, body: &str) {
    match host {
        Host::Tui => drop(herdr(&["notification", "show", title, "--body", body])),
        Host::Desktop => {
            // The CLI can't post desktop toasts; the toast mod picks this up and posts it.
            let stashed = state::update(|s| s.toast = Some([title.to_string(), body.to_string()]));
            if stashed.is_ok() {
                drop(run("herdr-app", &["do", "mod.open", "--name", "herdr-web-browser-toast"]));
            }
        }
    }
}

/// Ask the host to show the agent picker; it reports back through `herdr-web-browser picked`.
pub fn open_picker(host: Host) -> R<()> {
    match host {
        Host::Tui => herdr(&["plugin", "pane", "open", "--plugin", state::PLUGIN_ID, "--entrypoint", "picker"]).map(drop),
        Host::Desktop => run("herdr-app", &["do", "mod.open", "--name", "herdr-web-browser-picker"]).map(drop),
    }
}

/// Open the live view in a pane beside `owner` (or the focused pane).
pub fn open_view(host: Host, owner: Option<&str>) -> R<String> {
    let exe = std::fs::canonicalize(std::env::current_exe()?)?.display().to_string();
    let mut argv = vec![exe.clone()];
    if let Some(o) = owner {
        argv.extend(["--owner".to_string(), o.to_string()]);
    }
    argv.push("view".into());
    match host {
        Host::Desktop => {
            if let Some(o) = owner
                && let Some(t) = live_view_for(o)
            {
                return Ok(format!("live view already open in terminal {t}"));
            }
            let mut args = vec!["do".to_string(), "pane.split".into(), "--direction".into(), "right".into()];
            if let Some(o) = owner
                && let Some((_, p)) = desktop_place(&run("herdr-app", &["ls"])?, o)
            {
                args.extend(["--pane".into(), p]);
            }
            args.extend(["--argv".into(), serde_json::to_string(&argv)?]);
            let refs: Vec<&str> = args.iter().map(String::as_str).collect();
            let v = run("herdr-app", &refs)?;
            Ok(format!("live view in terminal {}", v["terminal"].as_str().unwrap_or("?")))
        }
        Host::Tui => {
            let mut args = vec!["pane", "split", "--direction", "right", "--no-focus"];
            if owner.is_some() && env("HERDR_PANE_ID").as_deref() == owner {
                args.push("--current");
            }
            let v = herdr(&args)?;
            let pane = v.pointer("/result/pane/pane_id").and_then(Value::as_str).ok_or("pane split returned no pane")?;
            let q = |a: &String| format!("'{}'", a.replace('\'', r"'\''"));
            let cmd = argv.iter().map(q).collect::<Vec<_>>().join(" ");
            herdr(&["pane", "run", pane, &cmd])?;
            Ok(format!("live view in pane {pane} (needs a terminal with Kitty graphics)"))
        }
    }
}

/// The herdr desktop tab and pane that hold `terminal`, from `herdr-app ls`.
fn desktop_place(ls: &Value, terminal: &str) -> Option<(String, String)> {
    ls["spaces"]
        .as_array()?
        .iter()
        .flat_map(|sp| sp["tabs"].as_array().into_iter().flatten())
        .find_map(|tab| {
            let t = tab["terminals"].as_array()?.iter().find(|t| t["terminal"] == terminal)?;
            Some((s(tab, "id"), s(t, "pane"))).filter(|(_, p)| !p.is_empty())
        })
}

/// herdr desktop's + menu always starts programs in a new tab. Move the browser into a
/// split right of the last focused agent instead and close that tab. Returns false
/// (stay put) when there's no agent to sit beside.
/// ponytail: "last focused agent" stands in for "the tab you were on"; herdr reports no tab history.
pub fn split_beside_agent() -> R<bool> {
    let me = env("HC_TERMINAL_ID").ok_or("not in a herdr desktop terminal")?;
    let status = run("herdr-app", &["status"])?;
    let Some(agent) = status["agent"].as_str().filter(|a| *a != me) else { return Ok(false) };
    let ls = run("herdr-app", &["ls"])?;
    let (Some((_, pane)), Some((my_tab, _))) = (desktop_place(&ls, agent), desktop_place(&ls, &me)) else {
        return Ok(false);
    };
    let exe = std::fs::canonicalize(std::env::current_exe()?)?.display().to_string();
    let argv = serde_json::to_string(&[exe.as_str(), "view", "--own-tab"])?;
    let made = run("herdr-app", &["do", "pane.split", "--direction", "right", "--pane", &pane, "--argv", &argv])?;
    if let Some(t) = made["terminal"].as_str() {
        drop(run("herdr-app", &["do", "pane.focus", "--terminal", t]));
    }
    // Last: closing this tab ends this process.
    run("herdr-app", &["do", "tab.close", "--id", &my_tab, "--confirmed", "true"])?;
    Ok(true)
}

/// A running live view for `owner`, so `show` doesn't open a second one.
fn live_view_for(owner: &str) -> Option<String> {
    let v = run("hc", &["ls", "--json"]).ok()?;
    // `hc ls` names the program; only `hc get` has the command line with the owner.
    v["terminals"].as_array()?.iter().filter(|t| t["state"] == "live" && t["agent"] == "herdr-web-browser").find_map(|t| {
        let addr = s(t, "address");
        let full = run("hc", &["get", &addr, "--json"]).ok()?;
        let argv: Vec<&str> = full["argv"].as_array()?.iter().filter_map(Value::as_str).collect();
        (argv.last() == Some(&"view") && argv.windows(2).any(|w| w == ["--owner", owner])).then_some(addr)
    })
}

/// Live herdr-core terminal addresses, or None when hc isn't reachable.
pub fn live_hc_terminals() -> Option<Vec<String>> {
    let v = run("hc", &["ls", "--json"]).ok()?;
    Some(
        v["terminals"]
            .as_array()?
            .iter()
            .filter(|t| t["state"] == "live")
            .map(|t| s(t, "address"))
            .collect(),
    )
}

/// TUI pane ids look like `w1:p2`; herdr-core addresses like `t5` or `machine/t5`.
pub fn is_hc_owner(owner: &str) -> bool {
    !owner.contains(':')
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn desktop_agents_are_terminals_with_status() {
        let v = json!({ "spaces": [
            { "id": "sp1", "name": "Fathom", "active": false, "tabs": [{ "terminals": [
                { "terminal": "t3", "status": "blocked", "app": "claude-code" },
                { "terminal": "t4", "status": null, "app": null } ] }] },
            { "id": "sp2", "name": "web", "active": true, "tabs": [{ "terminals": [
                { "terminal": "devdesk/t1", "status": "idle", "app": "codex" } ] }] } ] });
        let (agents, active) = parse_desktop_agents(&v);
        assert_eq!(agents.iter().map(|a| a.id.as_str()).collect::<Vec<_>>(), ["t3", "devdesk/t1"]);
        assert_eq!(agents[1].workspace_id, "sp2");
        assert_eq!(active.as_deref(), Some("sp2"));
    }

    #[test]
    fn hc_fallback_skips_shells_and_finished() {
        let v = json!({ "terminals": [
            { "address": "t3", "agent": "claude-code", "state": "live", "status": "working" },
            { "address": "t6", "agent": "zsh", "state": "live" },
            { "address": "t2", "agent": "codex", "state": "finished", "status": "done" } ] });
        let ids: Vec<_> = parse_hc_agents(&v).into_iter().map(|a| a.id).collect();
        assert_eq!(ids, ["t3"]);
    }

    #[test]
    fn owner_kinds_are_told_apart() {
        assert!(is_hc_owner("t5") && is_hc_owner("devdesk/t1"));
        assert!(!is_hc_owner("w1:p2"));
    }
}
