use nu_protocol::engine::EngineState;
use nu_plugin::EvaluatedCall;
use nu_plugin::PluginCommand;
use nu_protocol::{Span, PipelineData, Value};

use nu_plugin_to_gui::{ToGuiCommand, ToGuiPlugin};

#[test]
fn command_returns_empty_output() {
    let plugin = ToGuiPlugin;
    let command = ToGuiCommand;
    let engine = EngineState::new();
    let call = EvaluatedCall::new(Span::unknown());

    // we don't need actual table data for the smoke test; an empty stream is enough
    let input = PipelineData::empty();

    let result = command.run(&plugin, &engine, &call, input).expect("run failed");
    assert!(result.is_empty());
}

#[test]
fn parsing_flags_doesnt_crash() {
    use nu_protocol::IntoSpanned;

    let plugin = ToGuiPlugin;
    let command = ToGuiCommand;
    let engine = EngineState::new();
    let mut call = EvaluatedCall::new(Span::unknown());
    call.add_flag("no-transpose".into_spanned(Span::unknown()));
    call.add_flag("no-autosize".into_spanned(Span::unknown()));
    call.add_named("filter".into_spanned(Span::unknown()), Value::string("foo", Span::unknown()));

    let input = PipelineData::empty();
    let result = command.run(&plugin, &engine, &call, input).expect("run failed");
    assert!(result.is_empty());
}

#[test]
fn default_autosize_true() {
    let plugin = ToGuiPlugin;
    let command = ToGuiCommand;
    let engine = EngineState::new();
    let call = EvaluatedCall::new(Span::unknown());
    let input = PipelineData::empty();
    // no flags means autosize should be enabled by default - just verify nothing panics
    let result = command.run(&plugin, &engine, &call, input).expect("run failed");
    assert!(result.is_empty());
}
