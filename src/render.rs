//! Terminal graphics for the live view: Kitty images where the terminal supports
//! them (herdr desktop; the herdr TUI inside Ghostty, kitty or WezTerm), else
//! truecolor half blocks, which work in any terminal.

use crate::R;
use crate::state::Config;
use base64::Engine;
use std::io::{Read, Write, stdout};
use std::time::Duration;

pub const IMAGE_ID: u32 = 4711;
/// Synchronized output (DEC 2026): the terminal shows an update all at once.
pub const SYNC_ON: &str = "\x1b[?2026h";
pub const SYNC_OFF: &str = "\x1b[?2026l";

/// Cell size in device px and the device pixel ratio guessed from it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Cells {
    pub w: f64,
    pub h: f64,
    pub dpr: f64,
    /// Width of one image column. herdr desktop places Kitty images on a whole-pixel
    /// grid (8 px) while its text cells are fractional (8.4 px), so images come out narrow.
    pub img_w: f64,
}

impl Cells {
    /// Used when the terminal doesn't answer the size query.
    pub const FALLBACK: Cells = Cells { w: 9.0, h: 18.0, dpr: 1.0, img_w: 9.0 };

    pub fn from_px(w: f64, h: f64) -> Cells {
        // Logical cells are ~14-22 px tall; twice that means a 2x (Retina) display.
        Cells { w, h, dpr: if h >= 28.0 { 2.0 } else { 1.0 }, img_w: w }
    }

    /// Image columns that cover `cols` text columns.
    pub fn img_cols(&self, cols: u16) -> u16 {
        (cols as f64 * self.w / self.img_w).round() as u16
    }

    pub fn aspect(&self) -> f64 {
        self.h / self.w
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Renderer {
    Kitty,
    Blocks,
}

/// Parse `CSI 6 ; height ; width t` (reply to `CSI 16 t`) out of raw terminal input.
pub fn parse_cell_reply(buf: &[u8]) -> Option<(f64, f64)> {
    let s = String::from_utf8_lossy(buf);
    let start = s.find("\x1b[6;")? + 4;
    let rest = &s[start..];
    let body = &rest[..rest.find('t')?];
    let mut it = body.split(';').map(|n| n.parse::<f64>().ok());
    let (h, w) = (it.next()??, it.next()??);
    (w > 0.0 && h > 0.0).then_some((w, h))
}

/// A Kitty terminal answers the probe image query with `ESC _ G i=31;OK`.
pub fn parse_kitty_reply(buf: &[u8]) -> bool {
    String::from_utf8_lossy(buf).contains("\x1b_Gi=31;OK")
}

/// herdr desktop draws terminals itself while herdr-core answers `CSI 16 t` with a fixed
/// 10x20, so derive cells from the app's font size (cells measured 8.4x22 px at 14 pt).
fn desktop_cells() -> Option<Cells> {
    let st = crate::host::run("herdr-app", &["status"]).ok()?;
    let fs = st["font_size"].as_f64()? * st["zoom"].as_f64().unwrap_or(1.0);
    let c = Cells::from_px(fs * 0.6, fs * 22.0 / 14.0);
    Some(Cells { img_w: c.w.floor().max(1.0), ..c })
}

/// Read terminal replies until DA1 arrives or `timeout` passes, without leaving a reader
/// behind: a thread blocked on stdin would later swallow the user's keys and clicks.
fn read_replies(timeout: Duration) -> Vec<u8> {
    let deadline = std::time::Instant::now() + timeout;
    let mut buf = Vec::new();
    loop {
        let left = deadline.saturating_duration_since(std::time::Instant::now());
        if left.is_zero() {
            return buf;
        }
        let mut pfd = libc::pollfd { fd: libc::STDIN_FILENO, events: libc::POLLIN, revents: 0 };
        // SAFETY: one valid pollfd for the duration of the call.
        let ready = unsafe { libc::poll(&mut pfd, 1, left.as_millis().min(i32::MAX as u128) as i32) };
        if ready <= 0 {
            return buf;
        }
        let mut chunk = [0u8; 256];
        let n = match std::io::stdin().lock().read(&mut chunk) {
            Ok(n) if n > 0 => n,
            _ => return buf,
        };
        buf.extend_from_slice(&chunk[..n]);
        let s = String::from_utf8_lossy(&buf);
        if let Some(i) = s.find("\x1b[?")
            && s[i..].contains('c')
        {
            return buf; // DA1 reply: ESC [ ? ... c
        }
    }
}

/// Ask for cell size and Kitty support in one round trip. DA1 (`CSI c`) goes last because
/// every terminal answers it, so the read ends as soon as the others would have arrived.
fn query_terminal() -> (Cells, bool) {
    let mut out = stdout();
    let probe = "\x1b[16t\x1b_Gi=31,s=1,v=1,a=q,t=d,f=24;AAAA\x1b\\\x1b[c";
    if write!(out, "{probe}").and_then(|_| out.flush()).is_err() {
        return (Cells::FALLBACK, false);
    }
    let buf = read_replies(Duration::from_millis(500));
    (parse_cell_reply(&buf).map(|(w, h)| Cells::from_px(w, h)).unwrap_or(Cells::FALLBACK), parse_kitty_reply(&buf))
}

/// Cell metrics and the renderer to use; `view_renderer` / `cell_aspect` override.
pub fn detect(cfg: &Config) -> (Cells, Renderer) {
    let desktop = std::env::var_os("HC_TERMINAL_ID").is_some();
    let (mut cells, kitty) = match desktop.then(desktop_cells).flatten() {
        Some(c) => (c, true),
        None => query_terminal(),
    };
    if let Some(k) = cfg.cell_aspect.filter(|k| *k > 0.5 && *k < 5.0) {
        cells.h = cells.w * k;
    }
    let renderer = match cfg.view_renderer.as_deref() {
        Some("kitty") => Renderer::Kitty,
        Some("blocks") => Renderer::Blocks,
        _ if kitty => Renderer::Kitty,
        _ => Renderer::Blocks,
    };
    (cells, renderer)
}

/// Cells (cols, rows) that fit a `w`×`h` frame into `cols`×`rows`, keeping aspect;
/// `k` is cell height / cell width.
pub fn fit(w: f64, h: f64, cols: u16, rows: u16, k: f64) -> (u16, u16) {
    let want_rows = (cols as f64 * h / (w * k)).round().max(1.0);
    if want_rows <= rows as f64 {
        (cols, want_rows as u16)
    } else {
        (((rows as f64 * k * w / h).round().max(1.0) as u16).min(cols), rows)
    }
}

/// Kitty graphics, double-buffered: upload the PNG (already base64) as image `id`, place it
/// at the cursor, then delete the previous image `old`. Replacing one id in place blanks
/// the picture while the new one uploads and decodes, which flickers on every frame.
pub fn kitty_swap(out: &mut Vec<u8>, b64: &str, id: u32, old: u32, cols: u16, rows: u16) -> R<()> {
    let chunks: Vec<&str> = b64.as_bytes().chunks(4096).map(|c| std::str::from_utf8(c).unwrap_or("")).collect();
    for (i, chunk) in chunks.iter().enumerate() {
        let more = u8::from(i + 1 < chunks.len());
        if i == 0 {
            write!(out, "\x1b_Ga=t,f=100,i={id},q=2,m={more};{chunk}\x1b\\")?;
        } else {
            write!(out, "\x1b_Gm={more};{chunk}\x1b\\")?;
        }
    }
    write!(out, "\x1b_Ga=p,i={id},p=1,q=2,C=1,c={cols},r={rows}\x1b\\")?;
    write!(out, "\x1b_Ga=d,d=I,i={old},q=2\x1b\\")?;
    Ok(())
}

/// Upload a PNG (base64) as image `id` without showing it.
pub fn kitty_upload(out: &mut Vec<u8>, b64: &str, id: u32) -> R<()> {
    let chunks: Vec<&str> = b64.as_bytes().chunks(4096).map(|c| std::str::from_utf8(c).unwrap_or("")).collect();
    for (i, chunk) in chunks.iter().enumerate() {
        let more = u8::from(i + 1 < chunks.len());
        if i == 0 {
            write!(out, "\x1b_Ga=t,f=100,i={id},q=2,m={more};{chunk}\x1b\\")?;
        } else {
            write!(out, "\x1b_Gm={more};{chunk}\x1b\\")?;
        }
    }
    Ok(())
}

pub fn kitty_clear(out: &mut impl Write) {
    let _ = write!(out, "\x1b_Ga=d,d=I,i={IMAGE_ID},q=2\x1b\\\x1b_Ga=d,d=I,i={},q=2\x1b\\", IMAGE_ID + 1);
}

/// Decode a base64 PNG into (width, height, RGB bytes).
fn decode_rgb(b64: &str) -> R<(usize, usize, Vec<u8>)> {
    let bytes = base64::engine::general_purpose::STANDARD.decode(b64)?;
    let mut dec = png::Decoder::new(std::io::Cursor::new(bytes));
    dec.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = dec.read_info()?;
    let mut buf = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf)?;
    let ch = info.color_type.samples();
    let (w, h) = (info.width as usize, info.height as usize);
    let mut rgb = Vec::with_capacity(w * h * 3);
    for px in buf[..info.buffer_size()].chunks(ch) {
        match ch {
            1 | 2 => rgb.extend_from_slice(&[px[0]; 3]),
            _ => rgb.extend_from_slice(&px[..3]),
        }
    }
    Ok((w, h, rgb))
}

/// Box-average an RGB image down to `gw`×`gh`.
pub fn downsample(w: usize, h: usize, rgb: &[u8], gw: usize, gh: usize) -> Vec<[u8; 3]> {
    let mut out = Vec::with_capacity(gw * gh);
    for gy in 0..gh {
        let (y0, y1) = (gy * h / gh, ((gy + 1) * h / gh).max(gy * h / gh + 1).min(h));
        for gx in 0..gw {
            let (x0, x1) = (gx * w / gw, ((gx + 1) * w / gw).max(gx * w / gw + 1).min(w));
            let mut sum = [0u32; 3];
            let mut n = 0u32;
            for y in y0..y1 {
                for x in x0..x1 {
                    let i = (y * w + x) * 3;
                    for c in 0..3 {
                        sum[c] += rgb[i + c] as u32;
                    }
                    n += 1;
                }
            }
            let n = n.max(1);
            out.push([(sum[0] / n) as u8, (sum[1] / n) as u8, (sum[2] / n) as u8]);
        }
    }
    out
}

/// Half-block rendering: each cell shows two pixels (`▀` top in fg, bottom in bg).
pub fn blocks(out: &mut Vec<u8>, b64: &str, top_row: u16, cols: u16, rows: u16) -> R<()> {
    let (w, h, rgb) = decode_rgb(b64)?;
    let (gw, gh) = (cols as usize, rows as usize * 2);
    let px = downsample(w, h, &rgb, gw, gh);
    for y in 0..rows as usize {
        write!(out, "\x1b[{};1H", top_row as usize + y + 1)?;
        let (mut fg, mut bg) = ([0u8; 3], [0u8; 3]);
        for x in 0..gw {
            let (t, b) = (px[2 * y * gw + x], px[(2 * y + 1) * gw + x]);
            if x == 0 || t != fg {
                write!(out, "\x1b[38;2;{};{};{}m", t[0], t[1], t[2])?;
                fg = t;
            }
            if x == 0 || b != bg {
                write!(out, "\x1b[48;2;{};{};{}m", b[0], b[1], b[2])?;
                bg = b;
            }
            out.extend_from_slice("▀".as_bytes());
        }
        out.extend_from_slice(b"\x1b[0m");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wide_frames_fill_the_width() {
        // 900x450 CSS px at 9x18 px cells is 100x25 cells.
        assert_eq!(fit(900.0, 450.0, 100, 40, 2.0), (100, 25));
    }

    #[test]
    fn tall_frames_are_bound_by_rows() {
        assert_eq!(fit(900.0, 1800.0, 100, 20, 2.0), (20, 20));
    }

    #[test]
    fn desktop_images_get_extra_columns_for_whole_pixel_cells() {
        let c = Cells { w: 8.4, h: 22.0, dpr: 1.0, img_w: 8.0 };
        assert_eq!(c.img_cols(80), 84); // 80 x 8.4 px = 672 px = 84 x 8 px
    }

    #[test]
    fn cell_reply_is_parsed_from_mixed_input() {
        assert_eq!(parse_cell_reply(b"\x1b[6;34;16t\x1b[?62;22c"), Some((16.0, 34.0)));
        assert_eq!(parse_cell_reply(b"\x1b[?62;22c"), None);
    }

    #[test]
    fn kitty_support_is_read_from_the_probe_reply() {
        assert!(parse_kitty_reply(b"\x1b_Gi=31;OK\x1b\\\x1b[?62c"));
        assert!(!parse_kitty_reply(b"\x1b[?62;22c"));
    }

    #[test]
    fn downsample_averages_each_box() {
        // 2x1 image: black and white -> one grey pixel.
        let rgb = [0, 0, 0, 255, 255, 255];
        assert_eq!(downsample(2, 1, &rgb, 1, 1), vec![[127, 127, 127]]);
    }
}
