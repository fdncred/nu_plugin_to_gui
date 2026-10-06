use gpui::{Hsla, Rgba, rgb};
use nu_protocol::Value;

pub fn style_cache_key(v: &Value) -> String {
    serde_json::to_string(v).unwrap_or_else(|_| format!("{:?}", v.get_type()))
}

pub fn value_type_key(v: &Value) -> &'static str {
    match v {
        Value::Bool { .. } => "bool",
        Value::Int { .. } => "int",
        Value::Float { .. } => "float",
        Value::String { .. } => "string",
        Value::Filesize { .. } => "filesize",
        Value::Duration { .. } => "duration",
        Value::Date { .. } => "date",
        Value::Range { .. } => "range",
        Value::Record { .. } => "record",
        Value::List { .. } => "list",
        Value::Closure { .. } => "closure",
        Value::Nothing { .. } => "nothing",
        Value::Binary { .. } => "binary",
        Value::CellPath { .. } => "cellpath",
        _ => "string",
    }
}

pub fn ansi_16_fg(code: u8) -> Option<Rgba> {
    match code {
        30 => Some(rgb(0x000000)),
        31 => Some(rgb(0x800000)),
        32 => Some(rgb(0x008000)),
        33 => Some(rgb(0x808000)),
        34 => Some(rgb(0x000080)),
        35 => Some(rgb(0x800080)),
        36 => Some(rgb(0x008080)),
        37 => Some(rgb(0xc0c0c0)),
        90 => Some(rgb(0x808080)),
        91 => Some(rgb(0xff0000)),
        92 => Some(rgb(0x00ff00)),
        93 => Some(rgb(0xffff00)),
        94 => Some(rgb(0x0000ff)),
        95 => Some(rgb(0xff00ff)),
        96 => Some(rgb(0x00ffff)),
        97 => Some(rgb(0xffffff)),
        _ => None,
    }
}

pub fn xterm_256_to_rgb(code: u8) -> Rgba {
    if code < 16 {
        let base = [
            0x000000, 0x800000, 0x008000, 0x808000, 0x000080, 0x800080, 0x008080, 0xc0c0c0,
            0x808080, 0xff0000, 0x00ff00, 0xffff00, 0x0000ff, 0xff00ff, 0x00ffff, 0xffffff,
        ];
        return rgb(base[code as usize]);
    }

    if (16..=231).contains(&code) {
        let idx = code - 16;
        let r = idx / 36;
        let g = (idx % 36) / 6;
        let b = idx % 6;
        let level = |v: u8| if v == 0 { 0 } else { 55 + 40 * v };
        let rr = level(r) as u32;
        let gg = level(g) as u32;
        let bb = level(b) as u32;
        return rgb((rr << 16) | (gg << 8) | bb);
    }

    let gray = 8 + (code - 232) * 10;
    let g = gray as u32;
    rgb((g << 16) | (g << 8) | g)
}

/// Minimum contrast between text and its background (WCAG AA for body text).
pub const MIN_CONTRAST: f32 = 4.5;

fn linear_channel(c: f32) -> f32 {
    if c <= 0.039_28 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// WCAG relative luminance, 0.0 (black) to 1.0 (white).
pub fn relative_luminance(c: Rgba) -> f32 {
    0.2126 * linear_channel(c.r) + 0.7152 * linear_channel(c.g) + 0.0722 * linear_channel(c.b)
}

/// WCAG contrast ratio, 1.0 (none) to 21.0 (black on white).
pub fn contrast_ratio(a: Rgba, b: Rgba) -> f32 {
    let (la, lb) = (relative_luminance(a), relative_luminance(b));
    let (hi, lo) = if la > lb { (la, lb) } else { (lb, la) };
    (hi + 0.05) / (lo + 0.05)
}

/// Return `fg`, lightened or darkened just enough to reach [`MIN_CONTRAST`]
/// against `bg`. Hue and saturation are kept, so navy becomes a readable blue
/// and black text on a black background becomes gray.
pub fn ensure_contrast(fg: Rgba, bg: Rgba) -> Rgba {
    if contrast_ratio(fg, bg) >= MIN_CONTRAST {
        return fg;
    }
    let hsla = Hsla::from(fg);
    let with_lightness = |l: f32| Rgba::from(Hsla { l, ..hsla });
    // Below this luminance, white contrasts better than black.
    let lighten = relative_luminance(bg) < 0.18;
    // Luminance grows with lightness, so binary-search the closest passing value.
    let (mut lo, mut hi) = if lighten {
        (hsla.l, 1.0)
    } else {
        (0.0, hsla.l)
    };
    for _ in 0..16 {
        let mid = (lo + hi) / 2.0;
        let passes = contrast_ratio(with_lightness(mid), bg) >= MIN_CONTRAST;
        match (lighten, passes) {
            (true, true) | (false, false) => hi = mid,
            (true, false) | (false, true) => lo = mid,
        }
    }
    with_lightness(if lighten { hi } else { lo })
}

#[cfg(test)]
mod tests {
    use super::*;

    const BLACK_BG: u32 = 0x0a0a0a;

    #[test]
    fn readable_colors_are_unchanged() {
        let fg = rgb(0x61afef);
        assert_eq!(ensure_contrast(fg, rgb(BLACK_BG)), fg);
    }

    #[test]
    fn dark_colors_are_lightened_on_dark_background() {
        let bg = rgb(BLACK_BG);
        for dark in [0x000000, 0x000080, 0x008080, 0x800000, 0x1a1a1a] {
            let fixed = ensure_contrast(rgb(dark), bg);
            assert!(
                contrast_ratio(fixed, bg) >= MIN_CONTRAST,
                "{dark:06x} -> {fixed:?}"
            );
        }
    }

    #[test]
    fn hue_is_kept_when_lightening() {
        let fixed = Hsla::from(ensure_contrast(rgb(0x000080), rgb(BLACK_BG)));
        let navy = Hsla::from(rgb(0x000080));
        assert!((fixed.h - navy.h).abs() < 0.01);
    }

    #[test]
    fn light_colors_are_darkened_on_light_background() {
        let bg = rgb(0xd7d787); // xterm 186, the README highlight in LS_COLORS
        let fixed = ensure_contrast(rgb(0xffffff), bg);
        assert!(contrast_ratio(fixed, bg) >= MIN_CONTRAST);
        // Black on that highlight is already readable.
        assert_eq!(ensure_contrast(rgb(0x000000), bg), rgb(0x000000));
    }
}
