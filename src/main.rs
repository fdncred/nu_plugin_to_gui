use nu_plugin::MsgPackSerializer;
use nu_plugin::serve_plugin;
use nu_plugin_to_gui::ToGuiPlugin;
use nu_plugin_to_gui::gui_dispatch::{self, GuiRequest};
use std::sync::mpsc;
use std::time::Duration;

fn main() {
    let (gui_tx, gui_rx) = mpsc::channel::<GuiRequest>();
    gui_dispatch::init_main_thread_dispatch(gui_tx);

    let plugin_thread = std::thread::spawn(|| {
        serve_plugin(&ToGuiPlugin, MsgPackSerializer);
    });

    loop {
        match gui_rx.recv_timeout(Duration::from_millis(100)) {
            Ok(request) => {
                let result = nu_plugin_to_gui::gui::run_table_gui(
                    request.table,
                    request.initial_filter,
                    request.autosize,
                    request.color_config,
                    request.save_dir,
                    request.closure_sources,
                    request.table_config,
                    request.rfc3339,
                );
                if let Err(err) = result {
                    eprintln!("to-gui: GUI launch failed: {err:#}");
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if plugin_thread.is_finished() {
                    break;
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }

    let _ = plugin_thread.join();
}
