use crate::{ColorConfig, TableData};
use anyhow::{Result, anyhow};
use nu_protocol::Config;
use std::collections::HashMap;
use std::sync::{OnceLock, mpsc};

pub struct GuiRequest {
    pub table: TableData,
    pub initial_filter: Option<String>,
    pub autosize: bool,
    pub color_config: ColorConfig,
    pub save_dir: String,
    pub closure_sources: HashMap<usize, String>,
    pub table_config: Config,
    pub rfc3339: bool,
}

static GUI_REQUEST_TX: OnceLock<mpsc::Sender<GuiRequest>> = OnceLock::new();

pub fn init_main_thread_dispatch(tx: mpsc::Sender<GuiRequest>) {
    let _ = GUI_REQUEST_TX.set(tx);
}

pub fn has_main_thread_dispatch() -> bool {
    GUI_REQUEST_TX.get().is_some()
}

pub fn run_table_gui_on_main_thread(
    table: TableData,
    initial_filter: Option<String>,
    autosize: bool,
    color_config: ColorConfig,
    save_dir: String,
    closure_sources: HashMap<usize, String>,
    table_config: Config,
    rfc3339: bool,
) -> Result<()> {
    let tx = GUI_REQUEST_TX
        .get()
        .ok_or_else(|| anyhow!("to gui: GUI main-thread dispatcher is not initialized"))?
        .clone();

    tx.send(GuiRequest {
        table,
        initial_filter,
        autosize,
        color_config,
        save_dir,
        closure_sources,
        table_config,
        rfc3339,
    })
    .map_err(|send_err| anyhow!("to gui: failed to send GUI request to main thread: {send_err}"))?;

    Ok(())
}
