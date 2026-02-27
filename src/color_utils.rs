use gpui::{rgb, Rgba};
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