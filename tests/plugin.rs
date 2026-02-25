use nu_protocol::engine::EngineState;
use nu_protocol::command::EvaluatedCall;
use nu_protocol::IntoPipelineData;
use nu_protocol::PipelineDataExt;
use nu_protocol::{Span, PipelineData};

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
