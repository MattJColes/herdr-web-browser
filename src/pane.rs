//! Terminal UIs: the status/control pane and the agent picker popup.

use crate::agent::{self, Agent, SendKind};
use crate::cdp::Cdp;
use crate::host;
use crate::state::{self, Config, Mode, Picker};
use crate::{R, chrome, page};
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use crossterm::{cursor, execute, terminal};
use std::io::{Write, stdout};
use std::time::Duration;

/// Restores the terminal even if a draw or action fails.
struct Screen;

impl Screen {
    fn enter() -> R<Screen> {
        terminal::enable_raw_mode()?;
        execute!(stdout(), terminal::EnterAlternateScreen, cursor::Hide)?;
        Ok(Screen)
    }
    fn draw(&self, lines: &[String]) -> R<()> {
        let mut out = stdout();
        execute!(out, cursor::MoveTo(0, 0), terminal::Clear(terminal::ClearType::All))?;
        let width = terminal::size().map(|(w, _)| w as usize).unwrap_or(80);
        for l in lines {
            let l: String = l.chars().take(width).collect();
            write!(out, "{l}\r\n")?;
        }
        out.flush()?;
        Ok(())
    }
}

impl Drop for Screen {
    fn drop(&mut self) {
        let _ = execute!(stdout(), cursor::Show, terminal::LeaveAlternateScreen);
        let _ = terminal::disable_raw_mode();
    }
}

fn key(timeout: Duration) -> R<Option<KeyCode>> {
    if event::poll(timeout)?
        && let Event::Key(k) = event::read()?
        && k.kind == KeyEventKind::Press
    {
        return Ok(Some(k.code));
    }
    Ok(None)
}

fn status_lines(msg: &str) -> Vec<String> {
    let st = state::load().unwrap_or_default();
    let mode = st.mode.unwrap_or_default();
    let mode_s = match mode {
        Mode::Headed => "headed",
        Mode::Headless => "headless",
    };
    let target = st.target.clone().unwrap_or_else(|| "auto".into());
    let mut lines = Vec::new();
    match chrome::endpoint().and_then(|u| Cdp::connect(&u).ok()) {
        Some(mut c) => {
            let port = chrome::port().map(|p| p.to_string()).unwrap_or_default();
            lines.push(format!("herdr-web-browser  ● running {mode_s}  127.0.0.1:{port}   target: {target}"));
            lines.push(String::new());
            for (i, t) in page::page_targets(&mut c).unwrap_or_default().iter().enumerate() {
                let owner = st.tabs.iter().find(|(_, id)| **id == t.id).map(|(p, _)| p.as_str()).unwrap_or("-");
                lines.push(format!(" {:>2}  {owner:<8} {} — {}", i + 1, t.title, t.url));
            }
        }
        None => {
            lines.push(format!("herdr-web-browser  ○ stopped (next start: {mode_s})   target: {target}"));
            lines.push(String::new());
            lines.push("  [o] start the browser".into());
        }
    }
    lines.push(String::new());
    lines.push("[o] start  [h] headed/headless  [p] send page  [s] send selection  [c] clear target  [x] stop  [q] quit".into());
    if !msg.is_empty() {
        lines.push(msg.to_string());
    }
    lines
}

fn result(r: R<String>) -> String {
    r.unwrap_or_else(|e| format!("error: {e}"))
}

pub fn run_status() -> R<()> {
    let cfg = Config::load()?;
    let screen = Screen::enter()?;
    let mut msg = String::new();
    loop {
        screen.draw(&status_lines(&msg))?;
        let Some(k) = key(Duration::from_secs(1))? else { continue };
        msg = match k {
            KeyCode::Char('q') | KeyCode::Esc => return Ok(()),
            KeyCode::Char('o') => result(chrome::ensure(&cfg).map(|_| "browser running".into())),
            KeyCode::Char('x') => result(chrome::stop().map(|_| "browser stopped".into())),
            KeyCode::Char('h') => {
                let next = match state::load()?.mode.unwrap_or_default() {
                    Mode::Headed => Mode::Headless,
                    Mode::Headless => Mode::Headed,
                };
                screen.draw(&["switching mode…".into()])?;
                result(chrome::switch_mode(next, &cfg).map(|_| format!("now {next:?}").to_lowercase()))
            }
            KeyCode::Char(c @ ('p' | 's')) => {
                let kind = if c == 'p' { SendKind::Page } else { SendKind::Selection };
                result(agent::send_from_browser(kind, None, None, &cfg).map(|t| format!("sent to {t}")))
            }
            KeyCode::Char('c') => result(state::update(|s| s.target = None).map(|_| "target cleared".into())),
            _ => continue,
        };
    }
}

fn picker_lines(agents: &[Agent], sel: usize) -> Vec<String> {
    let mut lines = vec!["Send to which agent?  (↑/↓ or 1-9, Enter to choose, Esc to cancel)".into(), String::new()];
    for (i, a) in agents.iter().enumerate() {
        let mark = if i == sel { ">" } else { " " };
        lines.push(format!("{mark} {}. {:<10} {:<7} {:<8} {}  ({})", i + 1, a.id, a.kind, a.status, a.title, a.cwd));
    }
    lines
}

pub fn run_picker() -> R<()> {
    let (agents, _) = host::agents(host::Host::Tui)?;
    let choice = {
        let screen = Screen::enter()?;
        let mut sel = 0usize;
        loop {
            screen.draw(&picker_lines(&agents, sel))?;
            match key(Duration::from_secs(60))? {
                Some(KeyCode::Up | KeyCode::Char('k')) => sel = sel.saturating_sub(1),
                Some(KeyCode::Down | KeyCode::Char('j')) => sel = (sel + 1).min(agents.len().saturating_sub(1)),
                Some(KeyCode::Char(c @ '1'..='9')) if (c as usize - '1' as usize) < agents.len() => {
                    break Some(c as usize - '1' as usize);
                }
                Some(KeyCode::Enter) if !agents.is_empty() => break Some(sel),
                Some(KeyCode::Esc | KeyCode::Char('q')) => break None,
                _ => {}
            }
        }
    };
    state::update(|s| match choice {
        Some(i) => {
            s.target = Some(agents[i].id.clone());
            s.picker = Some(Picker::Chosen);
        }
        None => s.picker = Some(Picker::Cancelled),
    })?;
    Ok(())
}
