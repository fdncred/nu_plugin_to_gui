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
    pub header_style: CellStyle,
    pub ls_colors: std::collections::HashMap<String, gpui::Rgba>,
}

use nu_plugin::{Plugin, PluginCommand, EvaluatedCall, EngineInterface};
use nu_protocol::{Value, LabeledError, PipelineData, Signature, SyntaxShape};
use gpui::Rgba;
use std::collections::HashMap;

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

    for (key, value) in map {
        let style = match value {
            Value::String { val, .. } => CellStyle {
                fg: parse_color(val),
                ..CellStyle::default()
            },
            Value::Record { val: rec, .. } => {
                let fg = rec
                    .as_ref()
                    .get("fg")
                    .and_then(|v| if let Value::String { val, .. } = v { Some(val) } else { None })
                    .and_then(|s| parse_color(s));
                let bg = rec
                    .as_ref()
                    .get("bg")
                    .and_then(|v| if let Value::String { val, .. } = v { Some(val) } else { None })
                    .and_then(|s| parse_color(s));
                let bold = rec
                    .as_ref()
                    .get("attr")
                    .and_then(|v| if let Value::String { val, .. } = v { Some(val) } else { None })
                    .map(|s| s.to_ascii_lowercase().contains("bold"))
                    .unwrap_or(false);
                CellStyle { fg, bg, bold }
            }
            _ => CellStyle::default(),
        };

        if key == "header" {
            header_style = style;
        } else {
            type_styles.insert(key.clone(), style);
        }
    }

    ColorConfig {
        type_styles,
        header_style,
        ls_colors: HashMap::new(),
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
            .named("no-transpose", SyntaxShape::Nothing, "do not auto-transpose a single record into key/value rows", None)
            .named("no-autosize", SyntaxShape::Nothing, "disable automatic column sizing (enabled by default)", None)
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
        let initial_filter: Option<String> = call.get_flag("filter")?;

        let transpose = !no_transpose;
        let autosize = !no_autosize;

        // Build color configuration from $env.config.color_config.
        // `Config::color_config` is a HashMap<String, Value> that maps nushell
        // value-type names (e.g. "int", "float", "string", "header") to either
        // a color name string or a record with fg/bg/attr fields.
        let color_config = _engine
            .get_config()
            .map(|cfg| color_config_from_map(&cfg.color_config))
            .unwrap_or_default();

        let mut color_config = color_config;
        if let Ok(Some(Value::String { val, .. })) = _engine.get_env_var("LS_COLORS") {
            color_config.ls_colors = parse_ls_colors(&val);
        }

        let save_dir = _engine
            .get_current_dir()
            .unwrap_or_else(|_| std::env::current_dir().map(|p| p.display().to_string()).unwrap_or_else(|_| ".".to_string()));

        let values: Vec<Value> = input.into_iter().collect();
        let closure_sources = crate::value_conv::collect_closure_sources_with_plugin_engine(&values, _engine);
        let table = crate::value_conv::values_to_table_with_plugin_engine(&values, transpose, _engine);

        #[cfg(test)]
        let _ = (&initial_filter, autosize, &save_dir, &table, &closure_sources);

        #[cfg(not(test))]
        if let Err(err) = crate::gui::run_table_gui(
            table,
            initial_filter,
            autosize,
            color_config,
            save_dir,
            closure_sources,
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
