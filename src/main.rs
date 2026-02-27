use nu_plugin::MsgPackSerializer;
use nu_plugin::serve_plugin;
use nu_plugin_to_gui::ToGuiPlugin;
use nu_plugin_to_gui::gui_dispatch::{self, GuiLaunch};
use std::sync::mpsc;
use std::time::Duration;

fn main() {
    let (gui_tx, gui_rx) = mpsc::channel::<GuiLaunch>();
    gui_dispatch::init_main_thread_dispatch(gui_tx);

    let plugin_thread = std::thread::spawn(|| {
        serve_plugin(&ToGuiPlugin, MsgPackSerializer);
    });

    loop {
        match gui_rx.recv_timeout(Duration::from_millis(100)) {
            Ok(launch) => {
                let result = nu_plugin_to_gui::gui::run_table_gui(launch);
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
