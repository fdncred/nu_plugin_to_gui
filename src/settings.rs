//! Plugin settings read from `$env.config.plugins.to_gui` in config.nu:
//!
//! ```nu
//! $env.config.plugins.to_gui = {
//!     font_size: 18
//! }
//! ```

use nu_protocol::{LabeledError, Value};

pub const DEFAULT_FONT_SIZE: f32 = 16.0;
pub const MIN_FONT_SIZE: f32 = 10.0;
pub const MAX_FONT_SIZE: f32 = 28.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GuiSettings {
    /// Base UI font size in pixels.
    pub font_size: f32,
}

impl Default for GuiSettings {
    fn default() -> Self {
        Self {
            font_size: DEFAULT_FONT_SIZE,
        }
    }
}

impl GuiSettings {
    /// Parse the plugin's config value. Unknown keys are ignored.
    pub fn from_plugin_config(config: Option<&Value>) -> Result<Self, LabeledError> {
        let mut settings = Self::default();
        let Some(config) = config else {
            return Ok(settings);
        };
        let record = config.as_record().map_err(|_| {
            LabeledError::new("Invalid to_gui plugin config")
                .with_label("expected a record, e.g. { font_size: 18 }", config.span())
        })?;

        if let Some(value) = record.get("font_size") {
            let size = match value {
                Value::Int { val, .. } => *val as f32,
                Value::Float { val, .. } => *val as f32,
                _ => {
                    return Err(LabeledError::new("Invalid to_gui plugin config")
                        .with_label("font_size must be a number", value.span()));
                }
            };
            settings.font_size = clamp_font_size(size);
        }
        Ok(settings)
    }
}

/// Round to whole pixels and keep within the supported range.
pub fn clamp_font_size(size: f32) -> f32 {
    if size.is_finite() {
        size.round().clamp(MIN_FONT_SIZE, MAX_FONT_SIZE)
    } else {
        DEFAULT_FONT_SIZE
    }
}

/// The config.nu line that sets `font_size`.
pub fn font_size_config_line(font_size: f32) -> String {
    format!("$env.config.plugins.to_gui.font_size = {font_size}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use nu_protocol::{Record, Span};

    fn config(font_size: Value) -> Value {
        let mut rec = Record::new();
        rec.push("font_size", font_size);
        Value::record(rec, Span::unknown())
    }

    #[test]
    fn missing_config_uses_defaults() {
        let settings = GuiSettings::from_plugin_config(None).expect("defaults");
        assert_eq!(settings.font_size, DEFAULT_FONT_SIZE);
        let empty = Value::record(Record::new(), Span::unknown());
        let settings = GuiSettings::from_plugin_config(Some(&empty)).expect("defaults");
        assert_eq!(settings.font_size, DEFAULT_FONT_SIZE);
    }

    #[test]
    fn reads_int_and_float_font_size() {
        let int = config(Value::int(18, Span::unknown()));
        assert_eq!(
            GuiSettings::from_plugin_config(Some(&int))
                .expect("int")
                .font_size,
            18.0
        );
        let float = config(Value::float(19.6, Span::unknown()));
        assert_eq!(
            GuiSettings::from_plugin_config(Some(&float))
                .expect("float")
                .font_size,
            20.0
        );
    }

    #[test]
    fn rejects_non_number_font_size() {
        let bad = config(Value::string("big", Span::unknown()));
        assert!(GuiSettings::from_plugin_config(Some(&bad)).is_err());
    }

    #[test]
    fn rejects_non_record_config() {
        let bad = Value::int(18, Span::unknown());
        assert!(GuiSettings::from_plugin_config(Some(&bad)).is_err());
    }

    #[test]
    fn font_size_is_clamped_and_rounded() {
        assert_eq!(clamp_font_size(3.0), MIN_FONT_SIZE);
        assert_eq!(clamp_font_size(99.0), MAX_FONT_SIZE);
        assert_eq!(clamp_font_size(17.4), 17.0);
        assert_eq!(clamp_font_size(f32::NAN), DEFAULT_FONT_SIZE);
    }

    #[test]
    fn config_line_matches_plugin_name() {
        assert_eq!(
            font_size_config_line(18.0),
            "$env.config.plugins.to_gui.font_size = 18"
        );
    }
}
