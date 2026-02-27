#![recursion_limit = "256"]

//! Core library for the `to gui` nushell plugin.
//! Contains the plugin command, color/config helpers, GUI entrypoints, and
//! conversion utilities used by `src/main.rs`.

pub mod color_config;
pub mod color_utils;
#[cfg(not(test))]
pub mod gui;
#[cfg(not(test))]
pub mod gui_ansi;
#[cfg(not(test))]
pub mod gui_dispatch;
pub mod plugin_command;
pub mod table_data;
pub mod value_conv;
#[cfg(not(test))]
pub mod window_sizing;

#[cfg(not(test))]
pub use gui::{CellStyle, ColorConfig};

#[cfg(test)]
#[derive(Clone, Default)]
pub struct CellStyle {
    pub fg: Option<gpui::Rgba>,
    pub bg: Option<gpui::Rgba>,
    pub bold: bool,
}

#[cfg(test)]
#[derive(Clone, Default)]
pub struct ColorConfig {
    pub type_styles: std::collections::HashMap<String, CellStyle>,
    pub value_styles:
        std::collections::HashMap<String, std::collections::HashMap<String, CellStyle>>,
    pub default_style: CellStyle,
    pub use_ls_colors: bool,
    pub header_style: CellStyle,
    pub ls_colors: std::collections::HashMap<String, gpui::Rgba>,
}

pub use plugin_command::{ToGuiCommand, ToGuiPlugin};
pub use table_data::TableData;
