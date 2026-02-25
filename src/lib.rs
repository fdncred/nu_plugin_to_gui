//! Core library for the `to-gui` nushell plugin.
//!
//! This crate contains the plugin implementation and helpers used by
//! `src/main.rs` when run as a plugin binary.  Keeping most logic in a
//! library makes it easier to test.

pub mod gui;
pub mod value_conv;

use nu_plugin::{Plugin, PluginCommand, EvaluatedCall, EngineInterface};
use nu_protocol::{Value, LabeledError, PipelineData, Signature};

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
        Signature::build("to-gui").input_output_types(vec![(nu_protocol::Type::Any, nu_protocol::Type::Any)])
    }

    fn run(
        &self,
        _plugin: &ToGuiPlugin,
        _engine: &EngineInterface,
        _call: &EvaluatedCall,
        input: PipelineData,
    ) -> Result<PipelineData, LabeledError> {
        let values: Vec<Value> = input.into_iter().collect();
        let table = crate::value_conv::values_to_table(&values);

        if let Err(err) = crate::gui::run_table_gui(table) {
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
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableData {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<String>>,
}

impl TableData {
    pub fn new(columns: Vec<String>, rows: Vec<Vec<String>>) -> Self {
        TableData { columns, rows }
    }
}
