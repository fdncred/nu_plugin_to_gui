use crate::{ColorConfig, TableData};
use anyhow::{Result, anyhow};
use nu_protocol::Config;
use std::collections::HashMap;
use std::sync::{OnceLock, mpsc};

/// Complete payload needed to launch a GUI session.
///
/// Keeping this as one struct reduces long parameter lists and keeps
/// dispatch/main-thread launch paths consistent.
pub struct GuiLaunch {
    pub table: TableData,
    pub initial_filter: Option<String>,
    pub autosize: bool,
    pub color_config: ColorConfig,
    pub save_dir: String,
    pub closure_sources: HashMap<usize, String>,
    pub table_config: Config,
    pub rfc3339: bool,
    pub nerd_font_family: Option<String>,
}

static GUI_REQUEST_TX: OnceLock<mpsc::Sender<GuiLaunch>> = OnceLock::new();

/// Register the main-thread receiver used by plugin worker threads.
pub fn init_main_thread_dispatch(tx: mpsc::Sender<GuiLaunch>) {
    let _ = GUI_REQUEST_TX.set(tx);
}

pub fn has_main_thread_dispatch() -> bool {
    GUI_REQUEST_TX.get().is_some()
}

/// Enqueue a GUI launch on the process main thread.
///
/// This returns once enqueued so Nushell can immediately regain prompt control.
pub fn run_table_gui_on_main_thread(launch: GuiLaunch) -> Result<()> {
    let tx = GUI_REQUEST_TX
        .get()
        .ok_or_else(|| anyhow!("to gui: GUI main-thread dispatcher is not initialized"))?
        .clone();

    tx.send(launch).map_err(|send_err| {
        anyhow!("to gui: failed to send GUI request to main thread: {send_err}")
    })?;

    Ok(())
}
