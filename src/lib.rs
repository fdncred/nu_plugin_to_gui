#![recursion_limit = "256"]

//! Core library for the `to-gui` nushell plugin.
//!
//! This crate contains the plugin implementation and helpers used by
//! `src/main.rs` when run as a plugin binary.  Keeping most logic in a
//! library makes it easier to test.

pub mod gui;
pub mod value_conv;

use nu_plugin::{Plugin, PluginCommand, EvaluatedCall, EngineInterface};
use nu_protocol::{Value, LabeledError, PipelineData, Signature, SyntaxShape};
// colors will be represented as gpui::Fill so they can be applied to elements
use gpui::Rgba;

/// Try to parse a simple hex color string like `#rrggbb` or `rrggbb`.
///
/// Returns a `gpui::Fill` containing the parsed color.  We choose `Fill` since
/// most styling methods on elements accept a `Fill` and there is an
/// `impl From<Rgba> for Fill` to perform the conversion.
fn parse_color(s: &str) -> Option<Rgba> {
    let trimmed = s.trim();
    // strip off style hints like "_bold" or "_underlined" by taking prefix
    let mut key = trimmed;
    if let Some(idx) = trimmed.find('_') {
        key = &trimmed[..idx];
    }

    // first try hex format
    let hex = key.trim_start_matches('#');
    if hex.len() == 6 {
        if let Ok(v) = u32::from_str_radix(hex, 16) {
            let r = ((v >> 16) & 0xff) as u8;
            let g = ((v >> 8) & 0xff) as u8;
            let b = (v & 0xff) as u8;
            // gpui::rgb returns an Rgba color
            return Some(gpui::rgb((r as u32) << 16 | (g as u32) << 8 | (b as u32)));
        }
    }

    // simple named colors
    match key.to_lowercase().as_str() {
        "black" => Some(gpui::rgb(0x000000)),
        "white" => Some(gpui::rgb(0xffffff)),
        "red" => Some(gpui::rgb(0xff0000)),
        "green" => Some(gpui::rgb(0x00ff00)),
        "blue" => Some(gpui::rgb(0x0000ff)),
        "yellow" => Some(gpui::rgb(0xffff00)),
        "cyan" => Some(gpui::rgb(0x00ffff)),
        "magenta" => Some(gpui::rgb(0xff00ff)),
        _ => None,
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

        // color configuration from environment or shell config
        // first give precedence to explicit environment variables, then try
        // to read a couple of sensible entries out of `$env.config.color_config`.
        let mut fg: Option<Rgba> = std::env::var("NU_COLOR_FG").ok().and_then(|s| parse_color(&s));
        let mut bg: Option<Rgba> = std::env::var("NU_COLOR_BG").ok().and_then(|s| parse_color(&s));

        if fg.is_none() || bg.is_none() {
            if let Ok(cfg) = _engine.get_config() {
                // convert to json so we can look up arbitrary keys without
                // having to know the exact struct definition.
                if let Ok(json) = serde_json::to_value(&*cfg) {
                    if let Some(cc) = json.get("color_config") {
                        if let Some(map) = cc.as_object() {
                            if fg.is_none() {
                                if let Some(val) = map.get("header") {
                                    if let Some(s) = val.as_str() {
                                        fg = parse_color(s);
                                    }
                                }
                            }
                            if bg.is_none() {
                                if let Some(val) = map.get("empty") {
                                    if let Some(s) = val.as_str() {
                                        bg = parse_color(s);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        // supply harmless defaults so that the table isn't plain black/white
        // when no configuration is provided.
        let fg = fg.or(Some(gpui::rgb(0x000080))); // navy
        let bg = bg.or(Some(gpui::rgb(0xf0f8ff))); // alice blue

        let values: Vec<Value> = input.into_iter().collect();
        let table = crate::value_conv::values_to_table(&values, transpose);

        if let Err(err) = crate::gui::run_table_gui(table, initial_filter, autosize, fg, bg) {
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
        assert_eq!(parse_color("red"), Some(gpui::rgb(0xff0000)));
        assert_eq!(parse_color("Blue"), Some(gpui::rgb(0x0000ff)));
        assert_eq!(parse_color("green_bold"), Some(gpui::rgb(0x00ff00)));
        assert_eq!(parse_color("yellow_underlined"), Some(gpui::rgb(0xffff00)));
        assert_eq!(parse_color("unknowncolor"), None);
    }
}
