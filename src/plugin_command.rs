use crate::color_config::build_runtime_color_config;
use nu_plugin::{EngineInterface, EvaluatedCall, Plugin, PluginCommand};
use nu_protocol::{LabeledError, PipelineData, Signature, SyntaxShape, Value};

/// The plugin type returned to Nushell.
pub struct ToGuiPlugin;

/// Command implemented by this plugin. Exported so tests can instantiate it.
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
        "to gui"
    }

    fn description(&self) -> &str {
        "Open a GUI window that displays incoming table or record data."
    }

    fn signature(&self) -> Signature {
        Signature::build("to gui")
            .input_output_types(vec![(nu_protocol::Type::Any, nu_protocol::Type::Any)])
            .switch("no-transpose", "do not auto-transpose a single record into key/value rows", None)
            .switch("no-autosize", "disable automatic column sizing (enabled by default)", None)
            .switch("rfc3339", "format all datetime values in RFC3339", None)
            .named("filter", SyntaxShape::String, "initial filter string", None)
    }

    fn run(
        &self,
        _plugin: &ToGuiPlugin,
        engine: &EngineInterface,
        call: &EvaluatedCall,
        input: PipelineData,
    ) -> Result<PipelineData, LabeledError> {
        let no_transpose = call.has_flag("no-transpose")?;
        let no_autosize = call.has_flag("no-autosize")?;
        let rfc3339 = call.has_flag("rfc3339")?;
        let initial_filter: Option<String> = call.get_flag("filter")?;

        let transpose = !no_transpose;
        let autosize = !no_autosize;
        let values: Vec<Value> = input.into_iter().collect();

        let table = crate::value_conv::values_to_table_with_plugin_engine(&values, transpose, engine, rfc3339);
        let color_config = build_runtime_color_config(&table, &values, engine);

        let save_dir = engine.get_current_dir().unwrap_or_else(|_| {
            std::env::current_dir()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|_| ".".to_string())
        });

        let closure_sources = crate::value_conv::collect_closure_sources_with_plugin_engine(&values, engine);
        #[cfg(not(test))]
        let nu_config = engine.get_config().unwrap_or_default();

        #[cfg(test)]
        let _ = (
            &initial_filter,
            autosize,
            &save_dir,
            &table,
            &closure_sources,
            rfc3339,
            &color_config,
        );

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
            return Err(LabeledError::new(format!(
                "to gui: failed to launch GUI: {err:#}"
            )));
        }

        Ok(PipelineData::empty())
    }
}
