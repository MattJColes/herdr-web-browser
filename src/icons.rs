//! Live-view button icons in herdr desktop's style (24-unit stroke icons, round caps),
//! rasterized for Kitty graphics; glyphs stand in where images aren't available.

use crate::R;
use resvg::{tiny_skia, usvg};

pub type Rgb = (u8, u8, u8);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Icon {
    Back,
    Forward,
    Reload,
    Camera,
    Window,
    Settings,
}

pub const ALL: [Icon; 6] = [Icon::Back, Icon::Forward, Icon::Reload, Icon::Camera, Icon::Window, Icon::Settings];

impl Icon {
    fn body(self) -> &'static str {
        match self {
            Icon::Back => r#"<path d="M19 12H5"/><path d="m12 19-7-7 7-7"/>"#,
            Icon::Forward => r#"<path d="M5 12h14"/><path d="m12 5 7 7-7 7"/>"#,
            Icon::Reload => r#"<path d="M20 12a8 8 0 1 1-2.35-5.65"/><path d="M20 4v5h-5"/>"#,
            Icon::Camera => r#"<path d="M4 8.5h3l1.8-2.5h6.4L17 8.5h3V19H4z"/><circle cx="12" cy="13.5" r="3.2"/>"#,
            Icon::Window => r#"<rect x="3" y="4.5" width="18" height="15" rx="2.5"/><path d="M3 9h18"/>"#,
            Icon::Settings => r#"<path d="M12.22 2h-.44a2 2 0 0 0-2 2v.18a2 2 0 0 1-1 1.73l-.43.25a2 2 0 0 1-2 0l-.15-.08a2 2 0 0 0-2.73.73l-.22.38a2 2 0 0 0 .73 2.73l.15.1a2 2 0 0 1 1 1.72v.51a2 2 0 0 1-1 1.74l-.15.09a2 2 0 0 0-.73 2.73l.22.38a2 2 0 0 0 2.73.73l.15-.08a2 2 0 0 1 2 0l.43.25a2 2 0 0 1 1 1.73V20a2 2 0 0 0 2 2h.44a2 2 0 0 0 2-2v-.18a2 2 0 0 1 1-1.73l.43-.25a2 2 0 0 1 2 0l.15.08a2 2 0 0 0 2.73-.73l.22-.39a2 2 0 0 0-.73-2.73l-.15-.08a2 2 0 0 1-1-1.74v-.5a2 2 0 0 1 1-1.74l.15-.09a2 2 0 0 0 .73-2.73l-.22-.38a2 2 0 0 0-2.73-.73l-.15.08a2 2 0 0 1-2 0l-.43-.25a2 2 0 0 1-1-1.73V4a2 2 0 0 0-2-2z"/><circle cx="12" cy="12" r="3"/>"#,
        }
    }

    /// Fallback when the terminal can't show images.
    pub fn glyph(self) -> &'static str {
        match self {
            Icon::Back => "←",
            Icon::Forward => "→",
            Icon::Reload => "↻",
            Icon::Camera => "◉",
            Icon::Window => "◐",
            Icon::Settings => "⚙",
        }
    }

    pub fn svg(self, color: Rgb) -> String {
        format!(
            r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#{:02x}{:02x}{:02x}" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round">{}</svg>"##,
            color.0,
            color.1,
            color.2,
            self.body()
        )
    }

    /// PNG of the icon centered in a `w`×`h` px transparent box, filling `scale` of its height.
    pub fn png(self, color: Rgb, w: u32, h: u32, scale: f32) -> R<Vec<u8>> {
        let tree = usvg::Tree::from_str(&self.svg(color), &usvg::Options::default())?;
        let mut pixmap = tiny_skia::Pixmap::new(w.max(1), h.max(1)).ok_or("bad icon size")?;
        let side = h as f32 * scale;
        let k = side / tree.size().height();
        let (dx, dy) = ((w as f32 - side) / 2.0, (h as f32 - side) / 2.0);
        resvg::render(&tree, tiny_skia::Transform::from_scale(k, k).post_translate(dx, dy), &mut pixmap.as_mut());
        Ok(pixmap.encode_png()?)
    }
}

fn hex(c: Rgb) -> String {
    format!("#{:02x}{:02x}{:02x}", c.0, c.1, c.2)
}

fn render_svg(svg: &str, w: u32, h: u32) -> R<Vec<u8>> {
    let tree = usvg::Tree::from_str(svg, &usvg::Options::default())?;
    let mut pixmap = tiny_skia::Pixmap::new(w.max(1), h.max(1)).ok_or("bad image size")?;
    resvg::render(&tree, tiny_skia::Transform::identity(), &mut pixmap.as_mut());
    Ok(pixmap.encode_png()?)
}

/// A hovered button: the icon on a rounded pill, like herdr desktop's hover state.
pub fn pill_png(icon: Icon, fg: Rgb, bg: Rgb, w: u32, h: u32) -> R<Vec<u8>> {
    let (inset, side) = (h as f32 * 0.1, h as f32 * 0.62);
    let r = (h as f32 - 2.0 * inset) * 0.32;
    let k = side / 24.0;
    let (dx, dy) = ((w as f32 - side) / 2.0, (h as f32 - side) / 2.0);
    let svg = format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{w}" height="{h}" viewBox="0 0 {w} {h}"><rect x="{inset}" y="{inset}" width="{rw}" height="{rh}" rx="{r}" fill="{bg}"/><g transform="translate({dx} {dy}) scale({k})" fill="none" stroke="{fg}" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round">{body}</g></svg>"#,
        rw = w as f32 - 2.0 * inset,
        rh = h as f32 - 2.0 * inset,
        bg = hex(bg),
        fg = hex(fg),
        body = icon.body()
    );
    render_svg(&svg, w, h)
}

/// One end of the rounded address field (`left` or right), one cell wide.
pub fn cap_png(left: bool, color: Rgb, w: u32, h: u32) -> R<Vec<u8>> {
    let r = h as f32 * 0.28;
    // A rect twice the cell wide, shifted so only its rounded end shows in this cell.
    let x = if left { 0.0 } else { -(w as f32) };
    let svg = format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{w}" height="{h}" viewBox="0 0 {w} {h}"><rect x="{x}" y="0" width="{ww}" height="{h}" rx="{r}" fill="{c}"/></svg>"#,
        ww = 2 * w,
        c = hex(color)
    );
    render_svg(&svg, w, h)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_icon_rasterizes_with_visible_pixels() {
        for icon in ALL {
            let png = icon.png((255, 255, 255), 34, 44, 0.7).unwrap();
            let px = tiny_skia::Pixmap::decode_png(&png).unwrap();
            assert!(px.pixels().iter().any(|p| p.alpha() > 0), "{icon:?} is blank");
        }
    }

    #[test]
    fn field_caps_round_only_their_outer_side() {
        let left = tiny_skia::Pixmap::decode_png(&cap_png(true, (255, 0, 0), 16, 40).unwrap()).unwrap();
        let alpha = |p: &tiny_skia::Pixmap, x: u32, y: u32| p.pixel(x, y).unwrap().alpha();
        assert_eq!(alpha(&left, 0, 0), 0); // rounded outer corner is transparent
        assert_eq!(alpha(&left, 15, 0), 255); // inner edge is solid
    }
}
