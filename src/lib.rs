#![recursion_limit = "256"]

//! Core library for the `to-gui` nushell plugin.
//!
//! This crate contains the plugin implementation and helpers used by
//! `src/main.rs` when run as a plugin binary.  Keeping most logic in a
//! library makes it easier to test.

// The GUI module uses deeply-nested GPUI generic types that cause rustc to
// overflow its stack when compiling in test mode.  Gate it behind cfg(not(test))
// and expose a minimal stub for the types the rest of lib.rs depends on.
#[cfg(not(test))]
pub mod gui;
pub mod value_conv;

#[cfg(not(test))]
use gui::{CellStyle, ColorConfig};

// Stub ColorConfig used during unit tests so the gui module is not compiled.
#[cfg(test)]
#[derive(Clone, Default)]
pub struct CellStyle {
    pub fg: Option<gpui::Rgba>,
    pub bg: Option<gpui::Rgba>,
    pub bold: bool,
}

#[cfg(test)]
#[derive(Clone, Default)]
pub struct ColorConfig {
    pub type_styles: std::collections::HashMap<String, CellStyle>,
    pub value_styles: std::collections::HashMap<String, std::collections::HashMap<String, CellStyle>>,
    pub default_style: CellStyle,
    pub use_ls_colors: bool,
    pub header_style: CellStyle,
    pub ls_colors: std::collections::HashMap<String, gpui::Rgba>,
}

use nu_plugin::{Plugin, PluginCommand, EvaluatedCall, EngineInterface};
use nu_protocol::{Record, Value, LabeledError, PipelineData, Signature, SyntaxShape, Spanned};
use gpui::Rgba;
use std::collections::HashMap;
use nu_ansi_term::Color as AnsiColor;
use lscolors::{Indicator as LsIndicator, LsColors};

fn colors_debug_enabled() -> bool {
    std::env::var("TO_GUI_DEBUG_COLORS")
        .map(|v| {
            let low = v.to_ascii_lowercase();
            matches!(low.as_str(), "1" | "true" | "yes" | "on")
        })
        .unwrap_or(false)
}

fn debug_cell_style(style: &CellStyle) -> String {
    format!("fg={:?} bg={:?} bold={}", style.fg, style.bg, style.bold)
}

fn parse_ansi_color_code(code: &str) -> Option<Rgba> {
    match code {
        "30" => Some(gpui::rgb(0x000000)),
        "31" => Some(gpui::rgb(0x800000)),
        "32" => Some(gpui::rgb(0x008000)),
        "33" => Some(gpui::rgb(0x808000)),
        "34" => Some(gpui::rgb(0x000080)),
        "35" => Some(gpui::rgb(0x800080)),
        "36" => Some(gpui::rgb(0x008080)),
        "37" => Some(gpui::rgb(0xc0c0c0)),
        "90" => Some(gpui::rgb(0x808080)),
        "91" => Some(gpui::rgb(0xff0000)),
        "92" => Some(gpui::rgb(0x00ff00)),
        "93" => Some(gpui::rgb(0xffff00)),
        "94" => Some(gpui::rgb(0x0000ff)),
        "95" => Some(gpui::rgb(0xff00ff)),
        "96" => Some(gpui::rgb(0x00ffff)),
        "97" => Some(gpui::rgb(0xffffff)),
        _ => None,
    }
}

fn xterm_256_to_rgb(code: u8) -> Rgba {
    if code < 16 {
        let base = [
            0x000000, 0x800000, 0x008000, 0x808000, 0x000080, 0x800080, 0x008080, 0xc0c0c0,
            0x808080, 0xff0000, 0x00ff00, 0xffff00, 0x0000ff, 0xff00ff, 0x00ffff, 0xffffff,
        ];
        return gpui::rgb(base[code as usize]);
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
        return gpui::rgb((rr << 16) | (gg << 8) | bb);
    }

    let gray = 8 + (code - 232) * 10;
    let g = gray as u32;
    gpui::rgb((g << 16) | (g << 8) | g)
}

fn parse_ls_color_value(spec: &str) -> Option<Rgba> {
    let parts: Vec<&str> = spec.split(';').collect();

    for (i, part) in parts.iter().enumerate() {
        if let Some(c) = parse_ansi_color_code(part) {
            return Some(c);
        }

        if *part == "38" {
            if parts.get(i + 1) == Some(&"5") {
                if let Some(code) = parts.get(i + 2).and_then(|n| n.parse::<u8>().ok()) {
                    return Some(xterm_256_to_rgb(code));
                }
            }
            if parts.get(i + 1) == Some(&"2") {
                let r = parts.get(i + 2).and_then(|n| n.parse::<u8>().ok());
                let g = parts.get(i + 3).and_then(|n| n.parse::<u8>().ok());
                let b = parts.get(i + 4).and_then(|n| n.parse::<u8>().ok());
                if let (Some(r), Some(g), Some(b)) = (r, g, b) {
                    let rr = r as u32;
                    let gg = g as u32;
                    let bb = b as u32;
                    return Some(gpui::rgb((rr << 16) | (gg << 8) | bb));
                }
            }
        }
    }

    let style = nu_color_config::lookup_ansi_color_style(spec);
    if let Some(fg) = style.foreground {
        return Some(ansi_color_to_rgba(fg));
    }

    None
}

fn parse_ls_colors(ls_colors: &str) -> HashMap<String, Rgba> {
    let mut out = HashMap::new();
    for pair in ls_colors.split(':') {
        let mut it = pair.splitn(2, '=');
        let Some(key) = it.next() else { continue };
        let Some(val) = it.next() else { continue };
        if let Some(color) = parse_ls_color_value(val) {
            out.insert(key.to_string(), color);
        }
    }
    out
}

fn parse_ls_colors_record(record: &Record) -> HashMap<String, Rgba> {
    let mut out = HashMap::new();
    for (key, value) in record.iter() {
        if let Value::String { val, .. } = value {
            if let Some(color) = parse_ls_color_value(val) {
                out.insert(key.clone(), color);
            }
        }
    }
    out
}

fn ansi_color_to_rgba(color: AnsiColor) -> Rgba {
    match color {
        AnsiColor::Black => gpui::rgb(0x000000),
        AnsiColor::DarkGray => gpui::rgb(0x808080),
        AnsiColor::Red => gpui::rgb(0x800000),
        AnsiColor::LightRed => gpui::rgb(0xff0000),
        AnsiColor::Green => gpui::rgb(0x008000),
        AnsiColor::LightGreen => gpui::rgb(0x00ff00),
        AnsiColor::Yellow => gpui::rgb(0x808000),
        AnsiColor::LightYellow => gpui::rgb(0xffff00),
        AnsiColor::Blue => gpui::rgb(0x000080),
        AnsiColor::LightBlue => gpui::rgb(0x0000ff),
        AnsiColor::Purple => gpui::rgb(0x800080),
        AnsiColor::LightPurple => gpui::rgb(0xff00ff),
        AnsiColor::Magenta => gpui::rgb(0x800080),
        AnsiColor::LightMagenta => gpui::rgb(0xff00ff),
        AnsiColor::Cyan => gpui::rgb(0x008080),
        AnsiColor::LightCyan => gpui::rgb(0x00ffff),
        AnsiColor::White => gpui::rgb(0xc0c0c0),
        AnsiColor::LightGray => gpui::rgb(0xd3d3d3),
        AnsiColor::Default => gpui::rgb(0xffffff),
        AnsiColor::Fixed(code) => xterm_256_to_rgb(code),
        AnsiColor::Rgb(r, g, b) => {
            let rr = r as u32;
            let gg = g as u32;
            let bb = b as u32;
            gpui::rgb((rr << 16) | (gg << 8) | bb)
        }
    }
}

/// Parse a color string used in nushell's `color_config` into a `gpui::Rgba`.
///
/// Supported formats:
/// * `#rrggbb` or `rrggbb` — standard 6-digit hex
/// * Named ANSI colors: `black`, `dark_gray`, `red`, `light_red`, `green`,
///   `light_green`, `yellow`, `light_yellow`, `blue`, `light_blue`,
///   `purple`, `light_purple`, `magenta`, `light_magenta`, `cyan`,
///   `light_cyan`, `white`, `light_gray`
/// * Short names used by nushell: `r`, `g`, `b`, `u`, `y`, `p`, `c`, `w`, etc.
/// * Style suffixes like `_bold`, `_italic`, `_underline` are stripped before
///   the color name is looked up.
pub fn parse_color(s: &str) -> Option<Rgba> {
    let trimmed = s.trim();

    // Strip style suffixes: e.g. "green_bold" → "green"
    // Nushell uses `name_attr` notation; split on first `_`.
    // But some names themselves contain `_` (e.g. `light_red`) so we try the
    // full string first, then progressively trim the suffix.
    if let Some(c) = parse_color_name(trimmed) {
        return Some(c);
    }
    // Try stripping the last `_`-delimited segment
    let mut base = trimmed;
    while let Some(pos) = base.rfind('_') {
        base = &base[..pos];
        if let Some(c) = parse_color_name(base) {
            return Some(c);
        }
    }
    None
}

fn parse_color_name(s: &str) -> Option<Rgba> {
    let s = s.trim().trim_start_matches('#');

    // Hex: exactly 6 hex digits
    if s.len() == 6 && s.chars().all(|c| c.is_ascii_hexdigit()) {
        if let Ok(v) = u32::from_str_radix(s, 16) {
            return Some(gpui::rgb(v));
        }
    }

    // Named colors as used by nushell (nu_ansi_term names)
    match s.to_lowercase().as_str() {
        // short codes
        "b" | "black"         => Some(gpui::rgb(0x000000)),
        "dgr" | "dark_gray" | "dark_grey"   => Some(gpui::rgb(0x808080)),
        "r" | "red" | "dark_red"             => Some(gpui::rgb(0x800000)),
        "lr" | "light_red"                   => Some(gpui::rgb(0xff0000)),
        "g" | "green" | "dark_green"         => Some(gpui::rgb(0x008000)),
        "lg" | "light_green"                 => Some(gpui::rgb(0x00ff00)),
        "y" | "yellow" | "dark_yellow"       => Some(gpui::rgb(0x808000)),
        "ly" | "light_yellow"                => Some(gpui::rgb(0xffff00)),
        "u" | "blue" | "dark_blue"           => Some(gpui::rgb(0x000080)),
        "lu" | "light_blue"                  => Some(gpui::rgb(0x0000ff)),
        "p" | "purple" | "dark_purple"       => Some(gpui::rgb(0x800080)),
        "lp" | "light_purple"                => Some(gpui::rgb(0xff00ff)),
        "m" | "magenta" | "dark_magenta"     => Some(gpui::rgb(0x800080)),
        "lm" | "light_magenta"               => Some(gpui::rgb(0xff00ff)),
        "c" | "cyan" | "dark_cyan"           => Some(gpui::rgb(0x008080)),
        "lc" | "light_cyan"                  => Some(gpui::rgb(0x00ffff)),
        "w" | "white" | "dark_white"         => Some(gpui::rgb(0xc0c0c0)),
        "ligr" | "light_gray" | "light_grey" => Some(gpui::rgb(0xd3d3d3)),
        "default" | "reset" | "none"         => None,
        _                                    => None,
    }
}

/// Extract a `ColorConfig` from a nushell `color_config` `HashMap<String, Value>`.
///
/// Each value in the map may be:
/// * `Value::String` — a color string (hex or named) or short code
/// * `Value::Record { fg, bg, attr }` — only `fg` is used for cell color
///
/// Keys are the nushell value type names (`"int"`, `"float"`, `"string"`, …)
/// plus `"header"` for column headers.
pub fn color_config_from_map(map: &HashMap<String, Value>) -> ColorConfig {
    let mut type_styles: HashMap<String, CellStyle> = HashMap::new();
    let mut header_style: CellStyle = CellStyle::default();
    let mut default_style: CellStyle = CellStyle::default();

    let parsed = nu_color_config::get_color_map(map);
    for (key, style) in parsed {
        let parsed_style = CellStyle {
            fg: style.foreground.map(ansi_color_to_rgba),
            bg: style.background.map(ansi_color_to_rgba),
            bold: style.is_bold,
        };

        if key == "header" {
            header_style = parsed_style;
        } else if key == "foreground" {
            default_style = parsed_style;
        } else {
            type_styles.insert(key, parsed_style);
        }
    }

    if let Some(style) = type_styles.get("date").cloned() {
        type_styles.entry("datetime".to_string()).or_insert(style);
    }
    if let Some(style) = type_styles.get("datetime").cloned() {
        type_styles.entry("date".to_string()).or_insert(style);
    }
    if let Some(style) = type_styles.get("cellpath").cloned() {
        type_styles.entry("cell-path".to_string()).or_insert(style);
    }
    if let Some(style) = type_styles.get("cell-path").cloned() {
        type_styles.entry("cellpath".to_string()).or_insert(style);
    }

    ColorConfig {
        type_styles,
        value_styles: HashMap::new(),
        default_style,
        use_ls_colors: false,
        header_style,
        ls_colors: HashMap::new(),
    }
}

fn is_ls_like_table(table: &TableData) -> bool {
    let has = |name: &str| table.columns.iter().any(|c| c.eq_ignore_ascii_case(name));
    has("name") && has("type") && has("size") && has("modified")
}

fn value_type_key_for_color(v: &Value) -> &'static str {
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

fn find_sample_value_for_style_key(values: &[Value], style_key: &str) -> Option<Value> {
    let target = if style_key == "datetime" { "date" } else { style_key };

    fn walk(v: &Value, target: &str) -> Option<Value> {
        if value_type_key_for_color(v) == target {
            return Some(v.clone());
        }

        match v {
            Value::Record { val, .. } => {
                for (_, inner) in val.as_ref().iter() {
                    if let Some(found) = walk(inner, target) {
                        return Some(found);
                    }
                }
                None
            }
            Value::List { vals, .. } => {
                for inner in vals {
                    if let Some(found) = walk(inner, target) {
                        return Some(found);
                    }
                }
                None
            }
            Value::Custom { val, .. } => {
                if let Ok(base) = val.to_base_value(v.span()) {
                    walk(&base, target)
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    for value in values {
        if let Some(found) = walk(value, target) {
            return Some(found);
        }
    }

    None
}

fn collect_values_for_style_key(values: &[Value], style_key: &str) -> Vec<Value> {
    let target = if style_key == "datetime" { "date" } else { style_key };
    let mut out = Vec::new();

    fn walk(v: &Value, target: &str, out: &mut Vec<Value>) {
        if value_type_key_for_color(v) == target {
            out.push(v.clone());
        }

        match v {
            Value::Record { val, .. } => {
                for (_, inner) in val.as_ref().iter() {
                    walk(inner, target, out);
                }
            }
            Value::List { vals, .. } => {
                for inner in vals {
                    walk(inner, target, out);
                }
            }
            Value::Custom { val, .. } => {
                if let Ok(base) = val.to_base_value(v.span()) {
                    walk(&base, target, out);
                }
            }
            _ => {}
        }
    }

    for value in values {
        walk(value, target, &mut out);
    }

    out
}

fn lscolors_color_to_rgba(color: lscolors::style::Color) -> Rgba {
    match color {
        lscolors::style::Color::Black => gpui::rgb(0x000000),
        lscolors::style::Color::Red => gpui::rgb(0x800000),
        lscolors::style::Color::Green => gpui::rgb(0x008000),
        lscolors::style::Color::Yellow => gpui::rgb(0x808000),
        lscolors::style::Color::Blue => gpui::rgb(0x000080),
        lscolors::style::Color::Magenta => gpui::rgb(0x800080),
        lscolors::style::Color::Cyan => gpui::rgb(0x008080),
        lscolors::style::Color::White => gpui::rgb(0xc0c0c0),
        lscolors::style::Color::BrightBlack => gpui::rgb(0x808080),
        lscolors::style::Color::BrightRed => gpui::rgb(0xff0000),
        lscolors::style::Color::BrightGreen => gpui::rgb(0x00ff00),
        lscolors::style::Color::BrightYellow => gpui::rgb(0xffff00),
        lscolors::style::Color::BrightBlue => gpui::rgb(0x0000ff),
        lscolors::style::Color::BrightMagenta => gpui::rgb(0xff00ff),
        lscolors::style::Color::BrightCyan => gpui::rgb(0x00ffff),
        lscolors::style::Color::BrightWhite => gpui::rgb(0xffffff),
        lscolors::style::Color::Fixed(code) => xterm_256_to_rgb(code),
        lscolors::style::Color::RGB(r, g, b) => {
            let rr = r as u32;
            let gg = g as u32;
            let bb = b as u32;
            gpui::rgb((rr << 16) | (gg << 8) | bb)
        }
    }
}

fn collect_name_strings(v: &Value, out: &mut Vec<String>) {
    match v {
        Value::Record { val, .. } => {
            for (k, inner) in val.as_ref().iter() {
                if k.eq_ignore_ascii_case("name") {
                    if let Value::String { val, .. } = inner {
                        out.push(val.clone());
                    }
                }
                collect_name_strings(inner, out);
            }
        }
        Value::List { vals, .. } => {
            for inner in vals {
                collect_name_strings(inner, out);
            }
        }
        Value::Custom { val, .. } => {
            if let Ok(base) = val.to_base_value(v.span()) {
                collect_name_strings(&base, out);
            }
        }
        _ => {}
    }
}

fn default_ls_colors_from_nushell(values: &[Value]) -> HashMap<String, Rgba> {
    let mut out = HashMap::new();
    let ls: LsColors = nu_utils::get_ls_colors(None);

    let indicators = [
        ("di", LsIndicator::Directory),
        ("fi", LsIndicator::RegularFile),
        ("ln", LsIndicator::SymbolicLink),
        ("pi", LsIndicator::FIFO),
        ("so", LsIndicator::Socket),
        ("bd", LsIndicator::BlockDevice),
        ("cd", LsIndicator::CharacterDevice),
        ("or", LsIndicator::OrphanedSymbolicLink),
        ("ex", LsIndicator::ExecutableFile),
    ];

    for (key, ind) in indicators {
        if let Some(style) = ls.style_for_indicator(ind) {
            if let Some(fg) = style.foreground {
                out.insert(key.to_string(), lscolors_color_to_rgba(fg));
            }
        }
    }

    let mut names = Vec::new();
    for value in values {
        collect_name_strings(value, &mut names);
    }

    for name in names {
        if let Some(dot) = name.rfind('.') {
            if dot + 1 < name.len() {
                let ext = name[dot + 1..].to_ascii_lowercase();
                if let Some(style) = ls.style_for_str(&name) {
                    if let Some(fg) = style.foreground {
                        out.insert(format!("*.{}", ext), lscolors_color_to_rgba(fg));
                    }
                }
            }
        }
    }

    out
}

fn style_cache_key(v: &Value) -> String {
    serde_json::to_string(v).unwrap_or_else(|_| format!("{:?}", v.get_type()))
}

fn style_from_color_value(value: &Value) -> Option<CellStyle> {
    let mut one = HashMap::new();
    one.insert("_".to_string(), value.clone());
    let parsed = nu_color_config::get_color_map(&one);
    let style = parsed.get("_")?;
    Some(CellStyle {
        fg: style.foreground.map(ansi_color_to_rgba),
        bg: style.background.map(ansi_color_to_rgba),
        bold: style.is_bold,
    })
}

fn apply_closure_color_overrides(
    color_config: &mut ColorConfig,
    map: &HashMap<String, Value>,
    engine: &EngineInterface,
    values: &[Value],
) {
    let debug = colors_debug_enabled();
    for (key, entry) in map {
        let Value::Closure { val, .. } = entry else { continue };

        let Some(sample) = find_sample_value_for_style_key(values, key) else {
            if debug {
                eprintln!("to-gui[color]: closure key '{}' had no sample value in pipeline", key);
            }
            continue;
        };

        let closure = Spanned {
            item: *val.clone(),
            span: entry.span(),
        };

        let result = engine
            .eval_closure(&closure, vec![sample.clone()], None)
            .or_else(|_| engine.eval_closure(&closure, vec![], Some(sample)));

        let Ok(result) = result else {
            if debug {
                eprintln!("to-gui[color]: closure key '{}' failed to evaluate", key);
            }
            continue;
        };

        let Some(style) = style_from_color_value(&result) else {
            if debug {
                eprintln!(
                    "to-gui[color]: closure key '{}' produced non-style value {:?}",
                    key,
                    result.get_type()
                );
            }
            continue;
        };

        if key == "header" {
            color_config.header_style = style;
        } else {
            color_config.type_styles.insert(key.clone(), style);
        }

        if debug {
            if let Some(applied) = color_config.type_styles.get(key) {
                eprintln!(
                    "to-gui[color]: closure key '{}' resolved base style {}",
                    key,
                    debug_cell_style(applied)
                );
            }
        }

        let samples = collect_values_for_style_key(values, key);
        for sample in samples.into_iter().take(512) {
            let result = engine
                .eval_closure(&closure, vec![sample.clone()], None)
                .or_else(|_| engine.eval_closure(&closure, vec![], Some(sample.clone())));

            let Ok(result) = result else {
                continue;
            };

            let Some(per_value_style) = style_from_color_value(&result) else {
                continue;
            };

            color_config
                .value_styles
                .entry(key.clone())
                .or_default()
                .insert(style_cache_key(&sample), per_value_style);
        }

        if debug {
            let per_value_count = color_config
                .value_styles
                .get(key)
                .map(|m| m.len())
                .unwrap_or(0);
            eprintln!(
                "to-gui[color]: closure key '{}' cached {} per-value styles",
                key,
                per_value_count
            );
        }
    }
}

/// The plugin type returned to Nushell.
pub struct ToGuiPlugin;

/// Command implemented by this plugin.  Exported so that tests can
/// instantiate it directly.
pub struct ToGuiCommand;

impl Plugin for ToGuiPlugin {
    fn version(&self) -> String {
        env!("CARGO_PKG_VERSION").into()
    }

    fn commands(&self) -> Vec<Box<dyn PluginCommand<Plugin = Self>>> {
        vec![Box::new(ToGuiCommand)]
    }
}

impl PluginCommand for ToGuiCommand {
    type Plugin = ToGuiPlugin;

    fn name(&self) -> &str {
        "to-gui"
    }

    fn description(&self) -> &str {
        "Open a GUI window that displays incoming table or record data."
    }

    fn signature(&self) -> Signature {
        Signature::build("to-gui")
            .input_output_types(vec![(nu_protocol::Type::Any, nu_protocol::Type::Any)])
            .switch("no-transpose", "do not auto-transpose a single record into key/value rows", None)
            .switch("no-autosize", "disable automatic column sizing (enabled by default)", None)
            .switch("rfc3339", "format all datetime values in RFC3339", None)
            .named("filter", SyntaxShape::String, "initial filter string", None)
    }

    fn run(
        &self,
        _plugin: &ToGuiPlugin,
        _engine: &EngineInterface,
        call: &EvaluatedCall,
        input: PipelineData,
    ) -> Result<PipelineData, LabeledError> {
        // parse configuration flags
        let no_transpose = call.has_flag("no-transpose")?;
        let no_autosize = call.has_flag("no-autosize")?;
        let rfc3339 = call.has_flag("rfc3339")?;
        let initial_filter: Option<String> = call.get_flag("filter")?;

        let transpose = !no_transpose;
        let autosize = !no_autosize;

        let values: Vec<Value> = input.into_iter().collect();

        let table = crate::value_conv::values_to_table_with_plugin_engine(
            &values,
            transpose,
            _engine,
            rfc3339,
        );
        let ls_like = is_ls_like_table(&table);

        // Capture Nushell runtime config so formatting can match CLI output.
        let nu_config = _engine.get_config().unwrap_or_default();

        // Build color configuration from $env.config.color_config.
        // `Config::color_config` is a HashMap<String, Value> that maps nushell
        // value-type names (e.g. "int", "float", "string", "header") to either
        // a color name string or a record with fg/bg/attr fields.
        let mut color_config = color_config_from_map(&nu_config.color_config);
        color_config.use_ls_colors = ls_like;
        let debug = colors_debug_enabled();
        if debug {
            eprintln!(
                "to-gui[color]: color_config entries={}, pipeline values={}",
                nu_config.color_config.len(),
                values.len()
            );
            eprintln!("to-gui[color]: ls-like-table={}", ls_like);
        }
        apply_closure_color_overrides(&mut color_config, &nu_config.color_config, _engine, &values);
        if color_config.use_ls_colors {
            if let Ok(Some(ls_colors_val)) = _engine.get_env_var("LS_COLORS") {
                color_config.ls_colors = match ls_colors_val {
                    Value::String { val, .. } => parse_ls_colors(&val),
                    Value::Record { val, .. } => parse_ls_colors_record(val.as_ref()),
                    _ => HashMap::new(),
                };
            } else {
                color_config.ls_colors = default_ls_colors_from_nushell(&values);
            }
        } else {
            color_config.ls_colors.clear();
        }

        if debug {
            let keys = ["string", "int", "float", "filesize", "date", "datetime", "duration", "bool", "header"];
            for key in keys {
                if key == "header" {
                    eprintln!(
                        "to-gui[color]: header style {}",
                        debug_cell_style(&color_config.header_style)
                    );
                    continue;
                }
                if let Some(style) = color_config.type_styles.get(key) {
                    eprintln!("to-gui[color]: type '{}' => {}", key, debug_cell_style(style));
                } else {
                    eprintln!("to-gui[color]: type '{}' => <missing>", key);
                }
            }

            let ls_probe = ["di", "fi", "ln", "*.rs", "*.toml", "*.md"];
            for probe in ls_probe {
                if let Some(color) = color_config.ls_colors.get(probe) {
                    eprintln!("to-gui[color]: LS_COLORS '{}' => {:?}", probe, color);
                }
            }
            eprintln!(
                "to-gui[color]: total LS_COLORS keys={}, dynamic type caches={}",
                color_config.ls_colors.len(),
                color_config.value_styles.len()
            );
        }

        let save_dir = _engine
            .get_current_dir()
            .unwrap_or_else(|_| std::env::current_dir().map(|p| p.display().to_string()).unwrap_or_else(|_| ".".to_string()));

        let closure_sources = crate::value_conv::collect_closure_sources_with_plugin_engine(&values, _engine);

        #[cfg(test)]
        let _ = (&initial_filter, autosize, &save_dir, &table, &closure_sources, rfc3339);

        #[cfg(not(test))]
        if let Err(err) = crate::gui::run_table_gui(
            table,
            initial_filter,
            autosize,
            color_config,
            save_dir,
            closure_sources,
            (*nu_config).clone(),
            rfc3339,
        ) {
            eprintln!("to-gui: GUI error: {:#?}", err);
        }

        Ok(PipelineData::empty())
    }
}

/// A simple representation of tabular data used by the GUI layer.
///
/// Columns are stored as a list of keys; each row is a vector of strings
/// with the same length as `columns`.  Empty strings indicate missing
/// values.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct TableData {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<String>>,
    /// original cell values corresponding to `rows` (same shape)
    pub raw: Vec<Vec<Value>>,
}

impl TableData {
    pub fn new(columns: Vec<String>, rows: Vec<Vec<String>>, raw: Vec<Vec<Value>>) -> Self {
        TableData { columns, rows, raw }
    }
}

// tests for helper functions that don't require GPUI
#[cfg(test)]
mod tests {
    use super::*;
    use gpui;

    #[test]
    fn parse_color_hex() {
        assert_eq!(parse_color("#ff0000"), Some(gpui::rgb(0xff0000)));
        assert_eq!(parse_color("00ff00"), Some(gpui::rgb(0x00ff00)));
        assert_eq!(parse_color("abcdef"), Some(gpui::rgb(0xabcdef)));
        assert_eq!(parse_color("bad"), None);
        assert_eq!(parse_color("#123"), None);
    }

    #[test]
    fn parse_color_names() {
        // nushell ANSI color names: "red" is dark red (0x800000), not bright
        assert_eq!(parse_color("red"), Some(gpui::rgb(0x800000)));
        // "blue" is dark blue in ANSI
        assert_eq!(parse_color("Blue"), Some(gpui::rgb(0x000080)));
        // "green_bold" strips "_bold" suffix → "green" which is dark green
        assert_eq!(parse_color("green_bold"), Some(gpui::rgb(0x008000)));
        // "yellow_underlined" strips "_underlined" → "yellow" which is dark yellow
        assert_eq!(parse_color("yellow_underlined"), Some(gpui::rgb(0x808000)));
        assert_eq!(parse_color("unknowncolor"), None);
    }

    #[test]
    fn parse_ls_colors_basic_ansi() {
        let map = parse_ls_colors("di=01;34:fi=0:ln=01;36:*.rs=01;31");
        assert_eq!(map.get("di"), Some(&gpui::rgb(0x000080)));
        assert_eq!(map.get("ln"), Some(&gpui::rgb(0x008080)));
        assert_eq!(map.get("*.rs"), Some(&gpui::rgb(0x800000)));
    }

    #[test]
    fn parse_ls_colors_xterm_256_and_truecolor() {
        let map = parse_ls_colors("*.nu=38;5;196:*.md=38;2;1;2;3");
        assert!(map.get("*.nu").is_some());
        assert_eq!(map.get("*.md"), Some(&gpui::rgb(0x010203)));
    }
}
