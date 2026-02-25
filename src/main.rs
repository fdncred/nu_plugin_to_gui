use nu_plugin_to_gui::ToGuiPlugin;
use nu_plugin::serve_plugin;
use nu_plugin::MsgPackSerializer;

fn main() {
    serve_plugin(&ToGuiPlugin, MsgPackSerializer);
}
