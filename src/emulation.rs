//! Viewport and device emulation for one tab, kept in `state.json` so the live view and every
//! CLI call (each its own CDP session, and Chrome drops a session's overrides when it detaches)
//! apply the same one.

use crate::R;
use crate::cdp::Cdp;
use serde::{Deserialize, Serialize};
use serde_json::json;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Emulation {
    pub name: Option<String>,
    pub width: u32,
    pub height: u32,
    pub dpr: f64,
    pub mobile: bool,
    pub user_agent: Option<String>,
}

struct Device {
    name: &'static str,
    width: u32,
    height: u32,
    dpr: f64,
    mobile: bool,
    user_agent: &'static str,
}

const IPHONE_UA: &str =
    "Mozilla/5.0 (iPhone; CPU iPhone OS 18_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.0 Mobile/15E148 Safari/604.1";
const IPAD_UA: &str =
    "Mozilla/5.0 (iPad; CPU OS 18_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.0 Mobile/15E148 Safari/604.1";
const ANDROID_UA: &str = "Mozilla/5.0 (Linux; Android 15; Pixel 9) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Mobile Safari/537.36";

const DEVICES: &[Device] = &[
    Device { name: "iphone", width: 393, height: 852, dpr: 3.0, mobile: true, user_agent: IPHONE_UA },
    Device { name: "iphone-se", width: 375, height: 667, dpr: 2.0, mobile: true, user_agent: IPHONE_UA },
    Device { name: "pixel", width: 412, height: 915, dpr: 2.625, mobile: true, user_agent: ANDROID_UA },
    Device { name: "ipad", width: 820, height: 1180, dpr: 2.0, mobile: true, user_agent: IPAD_UA },
    Device { name: "laptop", width: 1366, height: 768, dpr: 1.0, mobile: false, user_agent: "" },
    Device { name: "desktop", width: 1920, height: 1080, dpr: 1.0, mobile: false, user_agent: "" },
];

pub fn device_names() -> String {
    DEVICES.iter().map(|d| d.name).collect::<Vec<_>>().join(", ")
}

/// `WIDTHxHEIGHT` (CSS px) or a device name; `dpr` and `mobile` override either.
pub fn parse(spec: &str, dpr: Option<f64>, mobile: bool) -> R<Emulation> {
    let spec = spec.trim().to_lowercase();
    if let Some(d) = DEVICES.iter().find(|d| d.name == spec) {
        return Ok(Emulation {
            name: Some(d.name.to_string()),
            width: d.width,
            height: d.height,
            dpr: dpr.unwrap_or(d.dpr),
            mobile: d.mobile || mobile,
            user_agent: (!d.user_agent.is_empty()).then(|| d.user_agent.to_string()),
        });
    }
    let bad = || format!("viewport: expected WIDTHxHEIGHT or one of {}, got {spec:?}", device_names());
    let (w, h) = spec.split_once('x').ok_or_else(bad)?;
    let (width, height): (u32, u32) = (w.parse().map_err(|_| bad())?, h.parse().map_err(|_| bad())?);
    if !(100..=8000).contains(&width) || !(100..=8000).contains(&height) {
        return Err(format!("viewport: {width}x{height} is outside 100..8000 px").into());
    }
    let dpr = dpr.unwrap_or(1.0);
    if !(0.5..=4.0).contains(&dpr) {
        return Err(format!("viewport: --dpr {dpr} is outside 0.5..4").into());
    }
    Ok(Emulation { name: None, width, height, dpr, mobile, user_agent: None })
}

impl Emulation {
    pub fn label(&self) -> String {
        let size = format!("{}x{} @{}x", self.width, self.height, self.dpr);
        match &self.name {
            Some(n) => format!("{n} ({size})"),
            None if self.mobile => format!("{size}, mobile"),
            None => size,
        }
    }

    pub fn apply(&self, cdp: &mut Cdp, session: &str) -> R<()> {
        let sid = Some(session);
        cdp.call(
            "Emulation.setDeviceMetricsOverride",
            json!({ "width": self.width, "height": self.height, "deviceScaleFactor": self.dpr, "mobile": self.mobile }),
            sid,
        )?;
        cdp.call("Emulation.setTouchEmulationEnabled", json!({ "enabled": self.mobile }), sid)?;
        if let Some(ua) = &self.user_agent {
            cdp.call("Emulation.setUserAgentOverride", json!({ "userAgent": ua }), sid)?;
        }
        Ok(())
    }
}

/// Chrome's own zoom steps.
const ZOOM_LEVELS: &[f64] = &[0.25, 0.33, 0.5, 0.67, 0.75, 0.8, 0.9, 1.0, 1.1, 1.25, 1.5, 1.75, 2.0, 2.5, 3.0];

/// `in`, `out`, `reset`, or a level as `125`, `125%` or `1.25`.
pub fn zoom(current: f64, arg: &str) -> R<f64> {
    let arg = arg.trim().to_lowercase();
    let next = match arg.as_str() {
        "in" | "+" => ZOOM_LEVELS.iter().copied().find(|z| *z > current + 1e-6).unwrap_or(current),
        "out" | "-" => ZOOM_LEVELS.iter().rev().copied().find(|z| *z < current - 1e-6).unwrap_or(current),
        "reset" | "0" => 1.0,
        n => {
            let v: f64 = n.trim_end_matches('%').parse().map_err(|_| format!("zoom: expected in, out, reset or a level like 125%, got {n:?}"))?;
            if n.ends_with('%') || v > 5.0 { v / 100.0 } else { v }
        }
    };
    if !(0.25..=3.0).contains(&next) {
        return Err(format!("zoom: {}% is outside 25%..300%", (next * 100.0).round()).into());
    }
    Ok(next)
}

pub fn clear(cdp: &mut Cdp, session: &str) -> R<()> {
    let sid = Some(session);
    cdp.call("Emulation.clearDeviceMetricsOverride", json!({}), sid)?;
    cdp.call("Emulation.setTouchEmulationEnabled", json!({ "enabled": false }), sid)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_and_devices_parse() {
        let e = parse("390x844", None, false).unwrap();
        assert_eq!((e.width, e.height, e.dpr, e.mobile, e.name), (390, 844, 1.0, false, None));
        let e = parse("1280x800", Some(2.0), true).unwrap();
        assert_eq!((e.dpr, e.mobile), (2.0, true));
        let e = parse("iPhone", None, false).unwrap();
        assert_eq!((e.width, e.height, e.dpr, e.mobile), (393, 852, 3.0, true));
        assert!(e.user_agent.unwrap().contains("iPhone"));
        assert_eq!(parse("laptop", None, false).unwrap().user_agent, None);
    }

    #[test]
    fn bad_specs_are_refused_with_the_choices() {
        let e = parse("phone", None, false).unwrap_err().to_string();
        assert!(e.contains("iphone, iphone-se, pixel"), "{e}");
        assert!(parse("10x10", None, false).is_err());
        assert!(parse("800x600", Some(9.0), false).is_err());
        assert!(parse("800by600", None, false).is_err());
    }

    #[test]
    fn zoom_steps_like_chrome_and_takes_levels() {
        assert_eq!(zoom(1.0, "in").unwrap(), 1.1);
        assert_eq!(zoom(1.1, "in").unwrap(), 1.25);
        assert_eq!(zoom(1.0, "out").unwrap(), 0.9);
        assert_eq!(zoom(1.2, "out").unwrap(), 1.1);
        assert_eq!(zoom(3.0, "in").unwrap(), 3.0);
        assert_eq!(zoom(1.5, "reset").unwrap(), 1.0);
        assert_eq!(zoom(1.0, "125%").unwrap(), 1.25);
        assert_eq!(zoom(1.0, "150").unwrap(), 1.5);
        assert_eq!(zoom(1.0, "0.8").unwrap(), 0.8);
        assert!(zoom(1.0, "400%").is_err());
        assert!(zoom(1.0, "big").is_err());
    }

    #[test]
    fn labels() {
        assert_eq!(parse("ipad", None, false).unwrap().label(), "ipad (820x1180 @2x)");
        assert_eq!(parse("390x844", None, true).unwrap().label(), "390x844 @1x, mobile");
    }
}
