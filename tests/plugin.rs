use nu_plugin::PluginCommand;

use nu_plugin_to_gui::{ToGuiCommand, ToGuiPlugin};

// Note: `PluginCommand::run` requires a real `EngineInterface` which cannot be
// constructed in a unit-test context.  These tests therefore verify the
// command's metadata (name, description, signature) without invoking `run`.

#[test]
fn command_name_is_to_gui() {
    let command = ToGuiCommand;
    assert_eq!(command.name(), "to gui");
}

#[test]
fn command_has_description() {
    let command = ToGuiCommand;
    assert!(!command.description().is_empty());
}

#[test]
fn signature_has_expected_flags() {
    let command = ToGuiCommand;
    let sig = command.signature();
    let flag_names: Vec<&str> = sig.named.iter().map(|f| f.long.as_str()).collect();
    assert!(flag_names.contains(&"no-transpose"), "missing --no-transpose flag");
    assert!(flag_names.contains(&"no-autosize"),  "missing --no-autosize flag");
    assert!(flag_names.contains(&"rfc3339"),      "missing --rfc3339 flag");
    assert!(flag_names.contains(&"filter"),        "missing --filter flag");
}

#[test]
fn plugin_exposes_one_command() {
    use nu_plugin::Plugin;
    let plugin = ToGuiPlugin;
    assert_eq!(plugin.commands().len(), 1);
    assert_eq!(plugin.commands()[0].name(), "to gui");
}
