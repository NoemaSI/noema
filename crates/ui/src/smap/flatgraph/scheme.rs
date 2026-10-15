use gpui_kit::*;

/// Color scheme mirroring the HTML prototype tokens (dark + Tailwind-600 light).
#[derive(Clone)]
pub struct Scheme {
    pub bg: Hsla,
    pub surface: Hsla,
    pub surface_hover: Hsla,
    pub border: Hsla,
    pub text: Hsla,
    pub dim: Hsla,
    pub faint: Hsla,
    pub mono: Hsla,
    pub raw: Hsla,
    pub s1: Hsla,
    pub s2: Hsla,
    pub pwr: Hsla,
    pub amb: Hsla,
    pub ink: Hsla,
    pub light: bool,
}

pub fn hex(s: &str) -> Hsla {
    let s = s.trim_start_matches('#');
    let r = u8::from_str_radix(&s[0..2], 16).unwrap() as f32 / 255.0;
    let g = u8::from_str_radix(&s[2..4], 16).unwrap() as f32 / 255.0;
    let b = u8::from_str_radix(&s[4..6], 16).unwrap() as f32 / 255.0;
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let l = (max + min) / 2.0;
    let d = max - min;
    let (h, sa) = if d == 0.0 {
        (0.0, 0.0)
    } else {
        let sa = if l > 0.5 { d / (2.0 - max - min) } else { d / (max + min) };
        let h = if max == r {
            ((g - b) / d + if g < b { 6.0 } else { 0.0 }) / 6.0
        } else if max == g {
            ((b - r) / d + 2.0) / 6.0
        } else {
            ((r - g) / d + 4.0) / 6.0
        };
        (h, sa)
    };
    hsla(h, sa, l, 1.0)
}

impl Scheme {
    pub fn new(light: bool) -> Self {
        if light {
            Self {
                bg: hex("#EEF1F5"),
                surface: hex("#F8FAFC"),
                surface_hover: hex("#E9EEF4"),
                border: hex("#D7DEE7"),
                text: hex("#1E293B"),
                dim: hex("#64748B"),
                faint: hex("#94A3B8"),
                mono: hex("#334155"),
                raw: hex("#E8A33D"),
                s1: hex("#E05C4B"),
                s2: hex("#D062C7"),
                pwr: hex("#8A93A6"),
                amb: hex("#4FB3A9"),
                ink: hex("#1E293B"),
                light,
            }
        } else {
            Self {
                bg: hex("#0E1116"),
                surface: hex("#151A21"),
                surface_hover: hex("#1A212B"),
                border: hex("#2A313B"),
                text: hex("#E6EDF3"),
                dim: hex("#98A3B0"),
                faint: hex("#6B7684"),
                mono: hex("#C9D1D9"),
                raw: hex("#E8A33D"),
                s1: hex("#E05C4B"),
                s2: hex("#D062C7"),
                pwr: hex("#8A93A6"),
                amb: hex("#4FB3A9"),
                ink: hex("#FFFFFF"),
                light,
            }
        }
    }

    pub fn flow(&self, name: &str) -> Hsla {
        match name {
            "RawMaterialFeed" => self.raw,
            "Stage1Product" => self.s1,
            "Stage2Product" => self.s2,
            "ElectricalPower" => self.pwr,
            "AmbientInfluence" => self.amb,
            _ => self.dim,
        }
    }
}

pub fn mix(a: Hsla, b: Hsla, t: f32) -> Hsla {
    // mix in rgba space via hsla->rgba is overkill; blend h/s/l linearly
    let h = if (a.h - b.h).abs() > 0.5 {
        if a.h < b.h {
            a.h + (b.h - 1.0 - a.h) * t
        } else {
            a.h - (a.h - (b.h + 1.0)) * t
        }
    } else {
        a.h + (b.h - a.h) * t
    };
    hsla(
        (h + 1.0) % 1.0,
        a.s + (b.s - a.s) * t,
        a.l + (b.l - a.l) * t,
        a.a + (b.a - a.a) * t,
    )
}