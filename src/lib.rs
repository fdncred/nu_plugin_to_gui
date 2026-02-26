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
use gui::ColorConfig;

// Stub ColorConfig used during unit tests so the gui module is not compiled.
#[cfg(test)]
#[derive(Clone, Default)]
pub struct ColorConfig {
    pub type_colors: std::collections::HashMap<String, gpui::Rgba>,
    pub header_color: Option<gpui::Rgba>,
}

use nu_plugin::{Plugin, PluginCommand, EvaluatedCall, EngineInterface};
use nu_protocol::{Value, LabeledError, PipelineData, Signature, SyntaxShape};
use gpui::Rgba;
use std::collections::HashMap;

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
    let mut type_colors: HashMap<String, Rgba> = HashMap::new();
    let mut header_color: Option<Rgba> = None;

    for (key, value) in map {
        let color = match value {
            Value::String { val, .. } => parse_color(val),
            Value::Record { val: rec, .. } => {
                // Look for the `fg` field
                rec.as_ref()
                    .get("fg")
                    .and_then(|v| if let Value::String { val, .. } = v { Some(val) } else { None })
                    .and_then(|s| parse_color(s))
            }
            _ => None,
        };

        if let Some(rgba) = color {
            if key == "header" {
                header_color = Some(rgba);
            } else {
                type_colors.insert(key.clone(), rgba);
            }
        }
    }

    ColorConfig { type_colors, header_color }
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

        let values: Vec<Value> = input.into_iter().collect();
        let table = crate::value_conv::values_to_table(&values, transpose);

        #[cfg(not(test))]
        if let Err(err) = crate::gui::run_table_gui(table, initial_filter, autosize, color_config) {
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
}
