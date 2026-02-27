//! GUI code using `gpui` and `gpui-component` to render table data.
//!
//! # Navigation model
//! The view holds a stack of `TableData` snapshots.  When the user
//! double-clicks a cell that contains a record or list, the nested data is
//! pushed onto the stack and the table re-renders with that data.  A "Back"
//! button in the custom in-window toolbar lets the user return to the previous
//! table.

use crate::TableData;
use crate::color_utils::{style_cache_key, value_type_key};
use crate::gui_ansi::parse_ansi_segments;
use crate::window_sizing::ideal_window_size;
use nu_protocol::{Config, Value};
use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::{Root, StyledExt};
use gpui_component::button::{Button, ButtonVariants as _};
use gpui_component::table::{Table, TableDelegate, TableState, TableEvent, Column, ColumnSort};
use gpui_component::input::{Input, InputState, InputEvent};
use gpui_component::menu::{DropdownMenu as _, PopupMenu, PopupMenuItem};
use std::collections::HashMap;
use anyhow::Result;
use std::path::{Path, PathBuf};
use std::sync::Arc;

// json value type alias to avoid collision with `nu_protocol::Value`.
use serde_json::Value as JsonValue;

// ---------------------------------------------------------------------------
// Color configuration
// ---------------------------------------------------------------------------

/// Color assignments derived from `$env.config.color_config`.
/// Each entry maps a nushell value-type key (e.g. `"int"`, `"string"`) to an
/// `Rgba` color to use as the foreground for cells of that type.
#[derive(Clone, Default)]
pub struct CellStyle {
    /// Foreground color.
    pub fg: Option<Rgba>,
    /// Background color.
    pub bg: Option<Rgba>,
    /// Bold text.
    pub bold: bool,
}

#[derive(Clone, Default)]
pub struct ColorConfig {
    /// Cell styles keyed by nushell type name.
    pub type_styles: HashMap<String, CellStyle>,
    /// Dynamic per-value styles keyed by nushell type name then serialized value key.
    pub value_styles: HashMap<String, HashMap<String, CellStyle>>,
    /// Fallback style (e.g. from nushell `color_config.foreground`).
    pub default_style: CellStyle,
    /// Whether LS_COLORS should be applied to ls-like table columns.
    pub use_ls_colors: bool,
    /// Style for column headers (from `color_config.header`).
    pub header_style: CellStyle,
    /// Parsed `$LS_COLORS` entries (`di`, `ln`, `*.rs`, ...).
    pub ls_colors: HashMap<String, Rgba>,
}

fn numeric_string_key(s: &str) -> Option<&'static str> {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return None;
    }

    if trimmed.parse::<i64>().is_ok() {
        return Some("int");
    }

    if trimmed.parse::<f64>().is_ok() {
        return Some("float");
    }

    None
}

fn numeric_type_key_for_value(v: &Value) -> Option<&'static str> {
    match v {
        Value::Int { .. } | Value::Filesize { .. } | Value::Duration { .. } => Some("int"),
        Value::Float { .. } => Some("float"),
        Value::String { val, .. } => numeric_string_key(val),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Actions
// ---------------------------------------------------------------------------

macro_rules! define_action {
    ($name:ident, $id:literal) => {
        #[derive(Clone, PartialEq)]
        struct $name;

        impl gpui::Action for $name {
            fn boxed_clone(&self) -> Box<dyn gpui::Action> { Box::new(self.clone()) }
            fn partial_eq(&self, action: &dyn gpui::Action) -> bool {
                action.as_any().downcast_ref::<$name>().is_some()
            }
            fn name(&self) -> &'static str { $id }
            fn name_for_type() -> &'static str { $id }
            fn build(_value: JsonValue) -> gpui::Result<Box<dyn gpui::Action>> where Self: Sized {
                Ok(Box::new($name))
            }
        }

        gpui::register_action!($name);
    };
}

// Action for File → Save.
define_action!(SaveAction, "to-gui::save");
define_action!(CloseAction, "to-gui::close");
define_action!(UndoAction, "to-gui::undo");
define_action!(RedoAction, "to-gui::redo");
define_action!(CopyAction, "to-gui::copy");
define_action!(PasteAction, "to-gui::paste");
define_action!(ReloadAction, "to-gui::reload");
define_action!(ZoomInAction, "to-gui::zoom-in");
define_action!(ZoomOutAction, "to-gui::zoom-out");
define_action!(PreferencesAction, "to-gui::preferences");
define_action!(MinimizeAction, "to-gui::minimize");
define_action!(ZoomWindowAction, "to-gui::zoom-window");
define_action!(AboutAction, "to-gui::about");

// Action emitted by the "Back" button.
define_action!(BackAction, "to-gui::back");

// ---------------------------------------------------------------------------
// TableDelegate implementation
// ---------------------------------------------------------------------------

/// Delegate that provides data and rendering for the gpui-component `Table`.
pub struct NushellTableDelegate {
    pub all_rows: Vec<Vec<String>>,
    pub raw_rows: Vec<Vec<Value>>,
    pub visible_rows: Vec<usize>,
    pub original_order: Vec<usize>,
    pub columns: Vec<Column>,
    filter: Option<String>,
    column_filters: Vec<Option<String>>,
    color_config: ColorConfig,
    /// Per-column filter inputs rendered inside each column header.
    column_filter_inputs: Vec<Entity<InputState>>,
    /// Last right-clicked column index (used for cell-aware copy action).
    right_clicked_col: Option<usize>,
    /// Last clicked column index (used by double-click drilldown without forcing table scroll).
    last_clicked_col: Option<usize>,
}

impl NushellTableDelegate {
    pub fn new(
        data: TableData,
        autosize: bool,
        color_config: ColorConfig,
        column_filter_inputs: Vec<Entity<InputState>>,
    ) -> Self {
        let num_cols = data.columns.len();
        let count = data.rows.len();
        let mut columns: Vec<Column> = data
            .columns
            .iter()
            .map(|c| Column::new(c.clone(), c.clone()).sortable())
            .collect();

        for (col_ix, col) in columns.iter_mut().enumerate() {
            let numeric = data
                .raw
                .iter()
                .filter_map(|row| row.get(col_ix))
                .filter(|v| !matches!(v, Value::Nothing { .. }))
                .all(|v| numeric_type_key_for_value(v).is_some());
            if numeric {
                col.align = TextAlign::Right;
            }
        }

        if autosize {
            const CHAR_W: f32 = 8.0;
            const CELL_EXTRA_W: f32 = 20.0;
            const HEADER_EXTRA_W: f32 = 52.0;
            for (col_ix, col) in columns.iter_mut().enumerate() {
                let max_len = data
                    .rows
                    .iter()
                    .map(|row| row.get(col_ix).map(|s| s.len()).unwrap_or(0))
                    .max()
                    .unwrap_or(0);
                let cell_w = (max_len as f32) * CHAR_W + CELL_EXTRA_W;
                let header_w = (col.name.len() as f32) * CHAR_W + HEADER_EXTRA_W;
                col.width = cell_w.max(header_w).into();
            }
        }

        let original_order: Vec<usize> = (0..count).collect();
        NushellTableDelegate {
            all_rows: data.rows,
            raw_rows: data.raw,
            visible_rows: original_order.clone(),
            original_order,
            columns,
            filter: None,
            column_filters: vec![None; num_cols],
            color_config,
            column_filter_inputs,
            right_clicked_col: None,
            last_clicked_col: None,
        }
    }

    fn apply_filter(&mut self) {
        let global = self.filter.as_ref().map(|s| s.to_lowercase());

        fn matches(cell: &str, pat: &str) -> bool {
            let low = pat.to_lowercase();
            if let Some(rest) = low.strip_prefix("is:") {
                cell.eq_ignore_ascii_case(rest)
            } else if let Some(rest) = low.strip_prefix("contains:") {
                cell.to_lowercase().contains(rest)
            } else if let Some(rest) = low.strip_prefix("starts-with:") {
                cell.to_lowercase().starts_with(rest)
            } else if let Some(rest) = low.strip_prefix("ends-with:") {
                cell.to_lowercase().ends_with(rest)
            } else {
                cell.to_lowercase().contains(low.as_str())
            }
        }

        self.visible_rows = self
            .original_order
            .iter()
            .cloned()
            .filter(|&ix| {
                let row = &self.all_rows[ix];
                if let Some(ref pat) = global {
                    if !row.iter().any(|cell| cell.to_lowercase().contains(pat.as_str())) {
                        return false;
                    }
                }
                for (col_ix, filt) in self.column_filters.iter().enumerate() {
                    if let Some(pat) = filt {
                        if let Some(cell) = row.get(col_ix) {
                            if !matches(cell, pat) {
                                return false;
                            }
                        }
                    }
                }
                true
            })
            .collect();
    }

    pub fn set_filter(&mut self, pat: Option<String>) {
        self.filter = pat;
        self.apply_filter();
    }

    pub fn set_column_filter(&mut self, col: usize, pat: Option<String>) {
        if col < self.column_filters.len() {
            self.column_filters[col] = pat;
            self.apply_filter();
        }
    }

    fn cell_fg(&self, raw: &Value) -> Option<Rgba> {
        let key = value_type_key(raw);
        if let Some(style) = self
            .color_config
            .value_styles
            .get(key)
            .and_then(|by_value| by_value.get(&style_cache_key(raw)))
        {
            return style.fg;
        }
        self.color_config
            .type_styles
            .get(key)
            .and_then(|style| style.fg)
            .or_else(|| {
                numeric_type_key_for_value(raw)
                    .and_then(|numeric_key| self.color_config.type_styles.get(numeric_key))
                    .and_then(|style| style.fg)
            })
            .or(self.color_config.default_style.fg)
    }

    fn cell_bg(&self, raw: &Value) -> Option<Rgba> {
        let key = value_type_key(raw);
        if let Some(style) = self
            .color_config
            .value_styles
            .get(key)
            .and_then(|by_value| by_value.get(&style_cache_key(raw)))
        {
            return style.bg;
        }
        self.color_config
            .type_styles
            .get(key)
            .and_then(|style| style.bg)
            .or_else(|| {
                numeric_type_key_for_value(raw)
                    .and_then(|numeric_key| self.color_config.type_styles.get(numeric_key))
                    .and_then(|style| style.bg)
            })
            .or(self.color_config.default_style.bg)
    }

    fn cell_bold(&self, raw: &Value) -> bool {
        let key = value_type_key(raw);
        if let Some(style) = self
            .color_config
            .value_styles
            .get(key)
            .and_then(|by_value| by_value.get(&style_cache_key(raw)))
        {
            return style.bold;
        }
        self.color_config
            .type_styles
            .get(key)
            .map(|style| style.bold)
            .or_else(|| {
                numeric_type_key_for_value(raw)
                    .and_then(|numeric_key| self.color_config.type_styles.get(numeric_key))
                    .map(|style| style.bold)
            })
            .unwrap_or(self.color_config.default_style.bold)
    }

    fn cellpath_style(&self) -> Option<&CellStyle> {
        self.color_config
            .type_styles
            .get("cellpath")
            .or_else(|| self.color_config.type_styles.get("cell-path"))
    }

    fn is_transposed_key_column(&self, col_ix: usize) -> bool {
        col_ix == 0
            && self.columns.len() == 2
            && self.columns[0].name.eq_ignore_ascii_case("key")
            && self.columns[1].name.eq_ignore_ascii_case("value")
    }

    fn ls_key_for_row_type(&self, real_row: usize) -> Option<&'static str> {
        let type_col_ix = self
            .columns
            .iter()
            .position(|c| c.name.eq_ignore_ascii_case("type"))?;
        let row_ty = self
            .all_rows
            .get(real_row)
            .and_then(|r| r.get(type_col_ix))
            .map(|s| s.to_lowercase())
            .unwrap_or_default();
        match row_ty.as_str() {
            "dir" | "directory" => Some("di"),
            "symlink" | "link" => Some("ln"),
            "pipe" => Some("pi"),
            "socket" => Some("so"),
            "block" | "block_device" => Some("bd"),
            "char" | "char_device" => Some("cd"),
            "file" => Some("fi"),
            _ => None,
        }
    }

    fn ls_fg_for_name_cell(&self, real_row: usize, col_ix: usize) -> Option<Rgba> {
        if !self.color_config.use_ls_colors {
            return None;
        }
        let col_name = self.columns.get(col_ix).map(|c| c.name.to_lowercase())?;
        if col_name != "name" && col_name != "type" {
            return None;
        }

        if col_name == "name" {
            let name = self
                .all_rows
                .get(real_row)
                .and_then(|r| r.get(col_ix))
                .map(|s| s.as_str())
                .unwrap_or_default();

            if let Some(dot) = name.rfind('.') {
                if dot + 1 < name.len() {
                    let ext = &name[dot + 1..];
                    if let Some(c) = self.color_config.ls_colors.get(&format!("*.{}", ext)) {
                        return Some(*c);
                    }
                }
            }
        }

        if let Some(ls_key) = self.ls_key_for_row_type(real_row) {
            if let Some(c) = self.color_config.ls_colors.get(ls_key) {
                return Some(*c);
            }
        }

        self.color_config.ls_colors.get("fi").copied()
    }
}

impl TableDelegate for NushellTableDelegate {
    fn columns_count(&self, _: &App) -> usize { self.columns.len() }
    fn rows_count(&self, _: &App) -> usize { self.visible_rows.len() }
    fn column(&self, col_ix: usize, _: &App) -> &Column { &self.columns[col_ix] }

    fn render_th(
        &mut self,
        col_ix: usize,
        window: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> impl gpui::IntoElement {
        let name = self.columns[col_ix].name.clone();
        if let Some(inp) = self.column_filter_inputs.get(col_ix) {
            inp.update(cx, |state, cx| {
                state.set_placeholder(name.clone(), window, cx);
            });
        }

        let mut header = gpui::div().v_flex().gap_1().w_full();
        if let Some(inp) = self.column_filter_inputs.get(col_ix) {
            header = header.child(
                Input::new(inp)
                    .appearance(false)
                    .bordered(false)
                    .focus_bordered(false),
            );
        }
        if let Some(c) = self.color_config.header_style.fg {
            header = header.text_color(c);
        }
        if let Some(c) = self.color_config.header_style.bg {
            header = header.bg(c);
        }
        if self.color_config.header_style.bold {
            header = header.font_weight(FontWeight::BOLD);
        }
        header
    }

    fn render_td(
        &mut self,
        row_ix: usize,
        col_ix: usize,
        _window: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        let real_row = self.visible_rows[row_ix];
        let text = self.all_rows[real_row][col_ix].clone();
        let raw  = &self.raw_rows[real_row][col_ix];
        let key_style = if self.is_transposed_key_column(col_ix) {
            self.cellpath_style()
        } else {
            None
        };
        let fg   = self
            .ls_fg_for_name_cell(real_row, col_ix)
            .or_else(|| key_style.and_then(|style| style.fg))
            .or_else(|| self.cell_fg(raw));
        let bg   = key_style.and_then(|style| style.bg).or_else(|| self.cell_bg(raw));
        let bold = key_style
            .map(|style| style.bold)
            .unwrap_or_else(|| self.cell_bold(raw));
        let numeric = numeric_type_key_for_value(raw).is_some();
        let mut has_ansi_segments = false;

        let mut div = gpui::div().size_full();
        if let Some(segments) = parse_ansi_segments(&text) {
            has_ansi_segments = true;
            let mut text_row = gpui::div().h_flex().gap_0().w_full();
            for segment in segments.into_iter().filter(|seg| !seg.text.is_empty()) {
                let mut part = gpui::div().child(segment.text);
                if let Some(c) = segment.fg {
                    part = part.text_color(c);
                }
                if segment.bold {
                    part = part.font_weight(FontWeight::BOLD);
                }
                text_row = text_row.child(part);
            }
            div = div.child(text_row);
        } else {
            div = div.child(text);
        }
        if let Some(c) = fg {
            if !has_ansi_segments {
                div = div.text_color(c);
            }
        }
        if let Some(c) = bg { div = div.bg(c); }
        if bold && !has_ansi_segments { div = div.font_weight(FontWeight::BOLD); }
        if numeric {
            div = div.h_flex().justify_end();
        }
        div = div
            .on_mouse_down(MouseButton::Left, cx.listener(move |table, _, _, _cx| {
                table.delegate_mut().last_clicked_col = Some(col_ix);
            }))
            .on_mouse_down(MouseButton::Right, cx.listener(move |table, _, _, _cx| {
                table.delegate_mut().last_clicked_col = Some(col_ix);
                table.delegate_mut().right_clicked_col = Some(col_ix);
            }));
        div
    }

    fn perform_sort(
        &mut self,
        col_ix: usize,
        sort: ColumnSort,
        _: &mut Window,
        _: &mut Context<TableState<Self>>,
    ) {
        match sort {
            ColumnSort::Ascending  =>
                self.visible_rows.sort_by(|a, b| self.all_rows[*a][col_ix].cmp(&self.all_rows[*b][col_ix])),
            ColumnSort::Descending =>
                self.visible_rows.sort_by(|a, b| self.all_rows[*b][col_ix].cmp(&self.all_rows[*a][col_ix])),
            ColumnSort::Default    =>
                self.visible_rows = self.original_order.clone(),
        }
    }

    fn context_menu(
        &mut self,
        row_ix: usize,
        menu: PopupMenu,
        _window: &mut Window,
        _cx: &mut Context<TableState<Self>>,
    ) -> PopupMenu {
        let real_row = self.visible_rows.get(row_ix).copied().unwrap_or(row_ix);
        let col_ix = self.right_clicked_col.unwrap_or(0);
        let text = self
            .all_rows
            .get(real_row)
            .and_then(|r| r.get(col_ix))
            .cloned()
            .unwrap_or_default();
        menu.item(
            PopupMenuItem::new("Copy").on_click(move |_, _, cx| {
                cx.write_to_clipboard(ClipboardItem::new_string(text.clone()));
            }),
        )
    }
}

// ---------------------------------------------------------------------------
// View
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct NavPage {
    title: String,
    filter_input: Entity<InputState>,
    table_state: Entity<TableState<NushellTableDelegate>>,
}

/// The main view. Holds a navigation stack of live page states.
pub struct ToGuiView {
    /// Navigation stack of page snapshots including table/input state.
    nav_stack: Vec<NavPage>,
    filter_input: Entity<InputState>,
    table_state: Entity<TableState<NushellTableDelegate>>,
    save_dir: String,
    status_message: String,
    /// Copy of the root data used by the Save button.
    root_data: TableData,
    /// Closure source strings keyed by Nushell block id, captured at plugin entry.
    closure_sources: Arc<HashMap<usize, String>>,
    /// Nushell runtime config used for display formatting (dates/filesizes/etc).
    table_config: Arc<Config>,
    /// Whether datetime values should be rendered as RFC3339.
    rfc3339: bool,
}

impl ToGuiView {
    pub fn new(
        window: &mut Window,
        cx: &mut Context<ToGuiView>,
        table_data: TableData,
        initial_filter: Option<String>,
        autosize: bool,
        color_config: ColorConfig,
        save_dir: String,
        closure_sources: HashMap<usize, String>,
        table_config: Config,
        rfc3339: bool,
    ) -> Self {
        let root_data = table_data.clone();
        let closure_sources = Arc::new(closure_sources);
        let table_config = Arc::new(table_config);
        let (fi, ts) = Self::build_page(
            window,
            cx,
            &table_data,
            initial_filter,
            autosize,
            &color_config,
            closure_sources.clone(),
            table_config.clone(),
            rfc3339,
        );

        let root_page = NavPage {
            title: "root".into(),
            filter_input: fi.clone(),
            table_state: ts.clone(),
        };

        ToGuiView {
            nav_stack: vec![root_page],
            filter_input: fi,
            table_state: ts,
            save_dir,
            status_message: String::new(),
            root_data,
            closure_sources,
            table_config,
            rfc3339,
        }
    }

    fn root_json_string(&self) -> std::io::Result<String> {
        let data = &self.root_data;
        let json_rows: Vec<serde_json::Value> = data
            .rows
            .iter()
            .map(|row| {
                let obj: serde_json::Map<String, serde_json::Value> = data
                    .columns
                    .iter()
                    .zip(row.iter())
                    .map(|(k, v)| (k.clone(), serde_json::Value::String(v.clone())))
                    .collect();
                serde_json::Value::Object(obj)
            })
            .collect();

        serde_json::to_string_pretty(&json_rows)
            .map_err(|err| std::io::Error::new(std::io::ErrorKind::Other, err.to_string()))
    }

    fn save_root_json_to(&self, path: &Path) -> std::io::Result<()> {
        let json = self.root_json_string()?;
        std::fs::write(path, json)
    }

    fn start_save_as(&mut self, cx: &mut Context<ToGuiView>) {
        let base_dir = PathBuf::from(&self.save_dir);
        let suggested_name = "to-gui-output.json".to_string();
        let receiver = cx.prompt_for_new_path(&base_dir, Some(&suggested_name));

        cx.spawn(move |view: WeakEntity<ToGuiView>, async_cx: &mut AsyncApp| {
            let mut async_cx = async_cx.clone();
            async move {
            let chosen = match receiver.await {
                Ok(Ok(path_opt)) => path_opt,
                Ok(Err(err)) => {
                    let message = format!("Save failed: {}", err);
                    let _ = view.update(&mut async_cx, |view, cx| {
                        view.status_message = message;
                        cx.notify();
                    });
                    return;
                }
                Err(err) => {
                    let message = format!("Save failed: {}", err);
                    let _ = view.update(&mut async_cx, |view, cx| {
                        view.status_message = message;
                        cx.notify();
                    });
                    return;
                }
            };

            match chosen {
                Some(path) => {
                    let display = path.display().to_string();
                    let _ = view.update(&mut async_cx, |view, cx| {
                        match view.save_root_json_to(&path) {
                            Ok(()) => view.status_message = format!("Saved: {}", display),
                            Err(err) => view.status_message = format!("Save failed: {}", err),
                        }
                        cx.notify();
                    });
                }
                None => {
                    let _ = view.update(&mut async_cx, |view, cx| {
                        view.status_message = "Save canceled".to_string();
                        cx.notify();
                    });
                }
            }
        }
        }).detach();
    }

    /// Create the filter widgets and table-state entity for a given `TableData`.
    fn build_page(
        window: &mut Window,
        cx: &mut Context<ToGuiView>,
        data: &TableData,
        initial_filter: Option<String>,
        autosize: bool,
        cc: &ColorConfig,
        closure_sources: Arc<HashMap<usize, String>>,
        table_config: Arc<Config>,
        rfc3339: bool,
    ) -> (
        Entity<InputState>,
        Entity<TableState<NushellTableDelegate>>,
    ) {
        // Per-column filter inputs — owned by the delegate, rendered inside headers.
        let col_inputs: Vec<Entity<InputState>> = (0..data.columns.len())
            .map(|_| cx.new(|cx| InputState::new(window, cx)))
            .collect();
        // Keep a clone for subscriptions; the originals move into the delegate.
        let col_inputs_for_subs = col_inputs.clone();

        let delegate = NushellTableDelegate::new(data.clone(), autosize, cc.clone(), col_inputs);

        let ts = cx.new(|cx| {
            TableState::new(delegate, window, cx)
                .col_resizable(true)
                .col_movable(true)
                .sortable(true)
                .col_selectable(true)
                .row_selectable(true)
        });

        let fi = cx.new(|cx| InputState::new(window, cx));
        fi.update(cx, |input, cx| {
            input.set_placeholder("Global search", window, cx);
        });

        // Global filter subscription
        let ts2 = ts.clone();
        cx.subscribe_in(&fi, window, move |_v, input, event, _, cx| {
            if let InputEvent::Change = event {
                let s = input.read(cx).value().to_string();
                ts2.update(cx, |t, _| {
                    t.delegate_mut().set_filter(if s.is_empty() { None } else { Some(s) });
                });
            }
        }).detach();

        // Per-column filter subscriptions
        for (col_ix, inp) in col_inputs_for_subs.iter().enumerate() {
            let ts3 = ts.clone();
            cx.subscribe_in(inp, window, move |_v, input, event, _, cx| {
                if let InputEvent::Change = event {
                    let pat = input.read(cx).value().to_string();
                    ts3.update(cx, |t, _| {
                        t.delegate_mut().set_column_filter(
                            col_ix,
                            if pat.is_empty() { None } else { Some(pat) },
                        );
                    });
                }
            }).detach();
        }

        // Apply initial global filter
        if let Some(f) = initial_filter {
            fi.update(cx, |i, cx| i.set_value(f.clone(), window, cx));
            ts.update(cx, |t, _| t.delegate_mut().set_filter(Some(f)));
        }

        // Subscribe to DoubleClickedRow to navigate into nested values
        let data_clone = data.clone();
        let autosize_c  = autosize;
        let cc_clone    = cc.clone();
        let closure_sources_c = closure_sources.clone();
        let table_config_c = table_config.clone();
        cx.subscribe_in(&ts, window, move |view, _state, event, window, cx| {
            if let TableEvent::DoubleClickedRow(row_ix) = event {
                let row_ix = *row_ix;
                // Which column was clicked (fallback to selected/default 0)?
                let col_ix = view
                    .table_state
                    .read(cx)
                    .delegate()
                    .last_clicked_col
                    .or_else(|| view.table_state.read(cx).selected_col())
                    .unwrap_or(0);
                // Map to the actual data row (accounting for filtering)
                let real_row = view
                    .table_state
                    .read(cx)
                    .delegate()
                    .visible_rows
                    .get(row_ix)
                    .copied()
                    .unwrap_or(row_ix);

                // Navigate into the selected cell only.
                if let Some(raw_row) = data_clone.raw.get(real_row) {
                    if let Some(raw) = raw_row.get(col_ix).cloned() {
                        let col_name = data_clone.columns.get(col_ix).map_or("?", |s| s.as_str());
                        let title = format!("row[{}].{}", real_row, col_name);
                        match &raw {
                            Value::Record { .. } => {
                                let nested = crate::value_conv::values_to_table_with_closure_sources_and_config(
                                    &[raw.clone()],
                                    true,
                                    &closure_sources_c,
                                    &table_config_c,
                                    rfc3339,
                                );
                                view.push_page(window, cx, nested, title, autosize_c, &cc_clone);
                            }
                            Value::List { vals, .. } if !vals.is_empty() => {
                                let nested = crate::value_conv::values_to_table_with_closure_sources_and_config(
                                    vals,
                                    true,
                                    &closure_sources_c,
                                    &table_config_c,
                                    rfc3339,
                                );
                                view.push_page(window, cx, nested, title, autosize_c, &cc_clone);
                            }
                            _ => {}
                        }
                    }
                }
            }
        }).detach();

        (fi, ts)
    }

    fn push_page(
        &mut self,
        window: &mut Window,
        cx: &mut Context<ToGuiView>,
        data: TableData,
        title: String,
        autosize: bool,
        cc: &ColorConfig,
    ) {
        let (fi, ts) = Self::build_page(
            window,
            cx,
            &data,
            None,
            autosize,
            cc,
            self.closure_sources.clone(),
            self.table_config.clone(),
            self.rfc3339,
        );

        let page = NavPage {
            title,
            filter_input: fi.clone(),
            table_state: ts.clone(),
        };

        self.nav_stack.push(page);
        self.filter_input = fi;
        self.table_state = ts;
        cx.notify();
    }

    fn pop_page(&mut self, _window: &mut Window, cx: &mut Context<ToGuiView>) {
        if self.nav_stack.len() > 1 {
            self.nav_stack.pop();
            if let Some(page) = self.nav_stack.last().cloned() {
                self.filter_input = page.filter_input;
                self.table_state = page.table_state;
            }
            cx.notify();
        }
    }

    fn can_go_back(&self) -> bool { self.nav_stack.len() > 1 }

    fn current_title(&self) -> String {
        self.nav_stack
            .last()
            .map(|page| page.title.clone())
            .unwrap_or_default()
    }
}

impl Render for ToGuiView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<ToGuiView>) -> impl IntoElement {
        let can_back = self.can_go_back();
        let title    = self.current_title();
        let weak = cx.weak_entity();
        let weak2 = cx.weak_entity();

        let weak_file_save = cx.weak_entity();
        let weak_edit = cx.weak_entity();
        let weak_view = cx.weak_entity();
        let weak_options = cx.weak_entity();
        let weak_window = cx.weak_entity();
        let weak_help = cx.weak_entity();
        let menu_bar = gpui::div()
            .h_flex()
            .gap_2()
            .px_3()
            .py_2()
            .w_full()
            .border_b_1()
            .border_color(rgb(0x1f2937))
            .bg(rgb(0x111827))
            .child(
                Button::new("menu-file")
                    .ghost()
                    .label("File")
                    .text_color(rgb(0xf8fafc))
                    .dropdown_menu(move |menu, _, _| {
                        let save_weak = weak_file_save.clone();
                        let close_weak = weak_file_save.clone();
                        menu.item(PopupMenuItem::new("Save As…").on_click(move |_, _, cx| {
                            save_weak.update(cx, |view, cx| view.start_save_as(cx)).ok();
                        }))
                        .separator()
                        .item(PopupMenuItem::new("Close").on_click(move |_, _, cx| {
                            close_weak.update(cx, |view, cx| {
                                view.status_message = "Close is not implemented yet".to_string();
                                cx.notify();
                            }).ok();
                        }))
                    })
            )
            .child(
                Button::new("menu-edit")
                    .ghost()
                    .label("Edit")
                    .text_color(rgb(0xf8fafc))
                    .dropdown_menu(move |menu, _, _| {
                        let weak_edit_undo = weak_edit.clone();
                        let weak_edit_redo = weak_edit.clone();
                        let weak_edit_copy = weak_edit.clone();
                        menu.item(PopupMenuItem::new("Undo").on_click(move |_, _, cx| {
                            weak_edit_undo.update(cx, |view, cx| {
                                view.status_message = "Undo is not implemented yet".to_string();
                                cx.notify();
                            }).ok();
                        }))
                        .item(PopupMenuItem::new("Redo").on_click(move |_, _, cx| {
                            weak_edit_redo.update(cx, |view, cx| {
                                view.status_message = "Redo is not implemented yet".to_string();
                                cx.notify();
                            }).ok();
                        }))
                        .separator()
                        .item(PopupMenuItem::new("Copy").on_click(move |_, _, cx| {
                            weak_edit_copy.update(cx, |view, cx| {
                                view.status_message = "Use right-click on a cell to copy".to_string();
                                cx.notify();
                            }).ok();
                        }))
                    })
            )
            .child(
                Button::new("menu-view")
                    .ghost()
                    .label("View")
                    .text_color(rgb(0xf8fafc))
                    .dropdown_menu(move |menu, _, _| {
                        let weak_view_reload = weak_view.clone();
                        let weak_view_zoomin = weak_view.clone();
                        menu.item(PopupMenuItem::new("Refresh").on_click(move |_, _, cx| {
                            weak_view_reload.update(cx, |view, cx| {
                                view.status_message = "Refreshed".to_string();
                                cx.notify();
                            }).ok();
                        }))
                        .item(PopupMenuItem::new("Zoom In").on_click(move |_, _, cx| {
                            weak_view_zoomin.update(cx, |view, cx| {
                                view.status_message = "Zoom is not implemented yet".to_string();
                                cx.notify();
                            }).ok();
                        }))
                    })
            )
            .child(
                Button::new("menu-options")
                    .ghost()
                    .label("Options")
                    .text_color(rgb(0xf8fafc))
                    .dropdown_menu(move |menu, _, _| {
                        let weak_options_pref = weak_options.clone();
                        menu.item(PopupMenuItem::new("Preferences").on_click(move |_, _, cx| {
                            weak_options_pref.update(cx, |view, cx| {
                                view.status_message = "Preferences are not implemented yet".to_string();
                                cx.notify();
                            }).ok();
                        }))
                    })
            )
            .child(
                Button::new("menu-window")
                    .ghost()
                    .label("Window")
                    .text_color(rgb(0xf8fafc))
                    .dropdown_menu(move |menu, _, _| {
                        let weak_window_min = weak_window.clone();
                        menu.item(PopupMenuItem::new("Minimize").on_click(move |_, _, cx| {
                            weak_window_min.update(cx, |view, cx| {
                                view.status_message = "Minimize is not implemented yet".to_string();
                                cx.notify();
                            }).ok();
                        }))
                    })
            )
            .child(
                Button::new("menu-help")
                    .ghost()
                    .label("Help")
                    .text_color(rgb(0xf8fafc))
                    .dropdown_menu(move |menu, _, _| {
                        let weak_help_about = weak_help.clone();
                        menu.item(PopupMenuItem::new("About").on_click(move |_, _, cx| {
                            weak_help_about.update(cx, |view, cx| {
                                view.status_message = "to-gui plugin".to_string();
                                cx.notify();
                            }).ok();
                        }))
                    })
            );

        // In-window toolbar (visible on all platforms; primary on Windows/Linux)
        let toolbar = gpui::div()
            .h_flex()
            .gap_2()
            .px_3()
            .py_1()
            .w_full()
            .border_b_1()
            .border_color(rgb(0x1f2937))
            .bg(rgb(0x0f172a))
            .when(can_back, |el| {
                el.child(
                    gpui::div()
                        .id("back-btn")
                        .px_2()
                        .py_1()
                        .rounded(px(4.0))
                        .bg(rgb(0x1f2937))
                        .text_color(rgb(0xffffff))
                        .cursor_pointer()
                        .on_click(move |_, window, cx| {
                            weak.update(cx, |view, cx| view.pop_page(window, cx)).ok();
                        })
                        .child("← Back"),
                )
            })
            .child(
                gpui::div()
                    .flex_1()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(0xf8fafc))
                    .child(title),
            )
            .child(
                gpui::div()
                    .id("save-btn")
                    .px_2()
                    .py_1()
                    .rounded(px(4.0))
                    .bg(rgb(0x1f2937))
                    .text_color(rgb(0xffffff))
                    .cursor_pointer()
                    .on_click(move |_, _window, cx| {
                        weak2.update(cx, |view, cx| {
                            view.start_save_as(cx);
                        }).ok();
                    })
                    .child("💾 Save"),
            );

        // Global search in the status bar.
        let status_bar = gpui::div()
            .h_flex()
            .gap_1()
            .px_2()
            .py_1()
            .w_full()
            .border_t_1()
            .border_color(rgb(0xdddddd))
            .child(
                gpui::div()
                    .flex_shrink_0()
                    .w_40()
                    .child(
                        Input::new(&self.filter_input)
                            .cleanable(true)
                            .appearance(false)
                            .bordered(false)
                            .focus_bordered(false),
                    ),
            )
            .child(
                gpui::div()
                    .flex_1()
                    .text_color(rgb(0x555555))
                    .child(self.status_message.clone()),
            );

        gpui::div()
            .v_flex()
            .size_full()
            .child(menu_bar)
            .child(toolbar)
            .child(
                Table::new(&self.table_state)
                    .stripe(true)
                    .bordered(true)
                    .scrollbar_visible(true, true),
            )
            .child(status_bar)
    }
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

/// Launch the GUI.
#[cfg(not(test))]
pub fn run_table_gui(
    table: TableData,
    initial_filter: Option<String>,
    autosize: bool,
    color_config: ColorConfig,
    save_dir: String,
    closure_sources: HashMap<usize, String>,
    table_config: Config,
    rfc3339: bool,
) -> Result<()> {
    let app = Application::new().with_assets(gpui_component_assets::Assets);

    // Pre-compute the ideal size outside app.run so we can borrow `table`.
    let size = ideal_window_size(&table, autosize);

    app.run(move |cx| {
        gpui_component::init(cx);
        cx.activate(true);

        // On macOS the system menu bar picks this up.
        // On Windows/Linux it is a no-op, but the in-window toolbar above
        // provides the same functionality.
        cx.set_menus(vec![
            Menu {
                name: "File".into(),
                items: vec![
                    MenuItem::action("Save As…", SaveAction),
                    MenuItem::separator(),
                    MenuItem::action("Close", CloseAction),
                ],
            },
            Menu {
                name: "Edit".into(),
                items: vec![
                    MenuItem::action("Undo", UndoAction),
                    MenuItem::action("Redo", RedoAction),
                    MenuItem::separator(),
                    MenuItem::action("Copy", CopyAction),
                    MenuItem::action("Paste", PasteAction),
                ],
            },
            Menu {
                name: "View".into(),
                items: vec![
                    MenuItem::action("Reload", ReloadAction),
                    MenuItem::action("Zoom In", ZoomInAction),
                    MenuItem::action("Zoom Out", ZoomOutAction),
                ],
            },
            Menu {
                name: "Options".into(),
                items: vec![MenuItem::action("Preferences", PreferencesAction)],
            },
            Menu {
                name: "Window".into(),
                items: vec![
                    MenuItem::action("Minimize", MinimizeAction),
                    MenuItem::action("Zoom", ZoomWindowAction),
                ],
            },
            Menu {
                name: "Help".into(),
                items: vec![MenuItem::action("About", AboutAction)],
            },
        ]);

        let ts = table.clone();
        let save_dir2 = save_dir.clone();
        let closure_sources2 = closure_sources.clone();
        let table_config2 = table_config.clone();
        let rfc3339_2 = rfc3339;
        cx.on_action::<SaveAction>(move |_, _app| {
            let json_rows: Vec<serde_json::Value> = ts.rows.iter()
                .map(|row| {
                    let obj: serde_json::Map<String, serde_json::Value> =
                        ts.columns.iter()
                            .zip(row.iter())
                            .map(|(k, v)| (k.clone(), serde_json::Value::String(v.clone())))
                            .collect();
                    serde_json::Value::Object(obj)
                })
                .collect();
            if let Ok(json) = serde_json::to_string_pretty(&json_rows) {
                let path = std::path::PathBuf::from(&save_dir2).join("to-gui-output.json");
                let _ = std::fs::write(&path, json);
                eprintln!("to-gui: saved to {}", path.display());
            }
        });

        cx.on_action::<CloseAction>(|_, _| {
            eprintln!("to-gui: Close requested from native menu");
        });
        cx.on_action::<UndoAction>(|_, _| {
            eprintln!("to-gui: Undo requested from native menu");
        });
        cx.on_action::<RedoAction>(|_, _| {
            eprintln!("to-gui: Redo requested from native menu");
        });
        cx.on_action::<CopyAction>(|_, _| {
            eprintln!("to-gui: Copy requested from native menu");
        });
        cx.on_action::<PasteAction>(|_, _| {
            eprintln!("to-gui: Paste requested from native menu");
        });
        cx.on_action::<ReloadAction>(|_, _| {
            eprintln!("to-gui: Reload requested from native menu");
        });
        cx.on_action::<ZoomInAction>(|_, _| {
            eprintln!("to-gui: Zoom In requested from native menu");
        });
        cx.on_action::<ZoomOutAction>(|_, _| {
            eprintln!("to-gui: Zoom Out requested from native menu");
        });
        cx.on_action::<PreferencesAction>(|_, _| {
            eprintln!("to-gui: Preferences requested from native menu");
        });
        cx.on_action::<MinimizeAction>(|_, _| {
            eprintln!("to-gui: Minimize requested from native menu");
        });
        cx.on_action::<ZoomWindowAction>(|_, _| {
            eprintln!("to-gui: Window Zoom requested from native menu");
        });
        cx.on_action::<AboutAction>(|_, _| {
            eprintln!("to-gui: About requested from native menu");
        });

        // Center the window on the primary display at the computed size.
        let window_options = WindowOptions {
            window_bounds: Some(WindowBounds::centered(size, cx)),
            ..WindowOptions::default()
        };

        cx.spawn(async move |cx| {
            cx.open_window(window_options, move |window, cx| {
                let cc = color_config.clone();
                let save_dir = save_dir.clone();
                let view = cx.new(|cx| {
                    ToGuiView::new(
                        window,
                        cx,
                        table.clone(),
                        initial_filter.clone(),
                        autosize,
                        cc,
                        save_dir,
                        closure_sources2,
                        table_config2,
                        rfc3339_2,
                    )
                });
                cx.new(|cx| Root::new(view, window, cx))
            })?;
            Ok::<_, anyhow::Error>(())
        })
        .detach();
    });
    Ok(())
}

#[cfg(test)]
pub fn run_table_gui(
    _table: TableData,
    _filter: Option<String>,
    _autosize: bool,
    _color_config: ColorConfig,
    _save_dir: String,
    _closure_sources: HashMap<usize, String>,
    _table_config: Config,
    _rfc3339: bool,
) -> anyhow::Result<()> {
    Ok(())
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;

    fn make_table(cols: Vec<&str>, rows: Vec<Vec<&str>>) -> TableData {
        use nu_protocol::{Value, Span};
        let raw: Vec<Vec<Value>> = rows.iter()
            .map(|r| r.iter().map(|s| Value::string(s.to_string(), Span::unknown())).collect())
            .collect();
        TableData {
            columns: cols.into_iter().map(|s| s.to_string()).collect(),
            rows: rows.into_iter().map(|r| r.into_iter().map(|s| s.to_string()).collect()).collect(),
            raw,
        }
    }

    #[test]
    fn autosize_columns_wider_when_requested() {
        let table = make_table(vec!["a"], vec![vec!["loooong"]]);
        let d = NushellTableDelegate::new(table, true, ColorConfig::default(), vec![]);
        assert!(d.columns[0].width > px(100.0));
    }

    #[test]
    fn autosize_can_be_disabled() {
        let table = make_table(vec!["a"], vec![vec!["loooong"]]);
        let d = NushellTableDelegate::new(table, false, ColorConfig::default(), vec![]);
        assert_eq!(d.columns[0].width, px(100.0));
    }

    #[test]
    fn column_filter_hides_rows() {
        let table = make_table(vec!["a", "b"], vec![vec!["foo", "x"], vec!["bar", "y"]]);
        let mut d = NushellTableDelegate::new(table, false, ColorConfig::default(), vec![]);
        d.set_column_filter(0, Some("ba".into()));
        assert_eq!(d.visible_rows, vec![1]);
        d.set_column_filter(1, Some("x".into()));
        assert!(d.visible_rows.is_empty());
        d.set_column_filter(0, None);
        assert_eq!(d.visible_rows, vec![0]);
    }

    #[test]
    fn sorting_changes_order() {
        let table = make_table(vec!["a"], vec![vec!["2"], vec!["1"]]);
        let mut d = NushellTableDelegate::new(table, false, ColorConfig::default(), vec![]);
        assert_eq!(d.visible_rows, vec![0, 1]);
        d.visible_rows.sort_by(|a, b| d.all_rows[*a][0].cmp(&d.all_rows[*b][0]));
        assert_eq!(d.visible_rows, vec![1, 0]);
        d.visible_rows = d.original_order.clone();
        assert_eq!(d.visible_rows, vec![0, 1]);
    }

    #[test]
    fn filtering_hides_rows() {
        let table = make_table(vec!["a"], vec![vec!["foo"], vec!["bar"]]);
        let mut d = NushellTableDelegate::new(table, false, ColorConfig::default(), vec![]);
        d.set_filter(Some("ba".into()));
        assert_eq!(d.visible_rows, vec![1]);
        d.set_filter(None);
        assert_eq!(d.visible_rows, vec![0, 1]);
    }

    #[test]
    fn column_filter_special_terms() {
        let table = make_table(vec!["a"], vec![vec!["abc"], vec!["ab"], vec!["xbc"]]);
        let mut d = NushellTableDelegate::new(table, false, ColorConfig::default(), vec![]);
        d.set_column_filter(0, Some("is:ab".into()));
        assert_eq!(d.visible_rows, vec![1]);
        d.set_column_filter(0, Some("starts-with:ab".into()));
        assert_eq!(d.visible_rows, vec![0, 1]);
        d.set_column_filter(0, Some("ends-with:bc".into()));
        assert_eq!(d.visible_rows, vec![0, 2]);
        d.set_column_filter(0, Some("contains:bc".into()));
        assert_eq!(d.visible_rows, vec![0, 2]);
    }

    #[test]
    fn save_action_name() {
        assert_eq!(SaveAction.name(), "to-gui::save");
    }

    #[test]
    fn back_action_name() {
        assert_eq!(BackAction.name(), "to-gui::back");
    }

    #[test]
    fn value_type_key_mapping() {
        use nu_protocol::{Value, Span};
        assert_eq!(value_type_key(&Value::int(1, Span::unknown())),    "int");
        assert_eq!(value_type_key(&Value::float(1.0, Span::unknown())), "float");
        assert_eq!(value_type_key(&Value::string("", Span::unknown())), "string");
        assert_eq!(value_type_key(&Value::bool(true, Span::unknown())), "bool");
    }

    #[test]
    fn run_table_gui_stub() {
        let dummy = TableData::new(vec![], vec![], vec![]);
        let _ = run_table_gui(
            dummy,
            None,
            false,
            ColorConfig::default(),
            String::new(),
            HashMap::new(),
            Config::default(),
            false,
        );
    }

    #[test]
    fn ideal_window_size_grows_with_data() {
        let small = make_table(vec!["a"], vec![vec!["x"]]);
        let larger = make_table(
            vec!["alpha", "beta", "gamma"],
            (0..40).map(|_| vec!["val1", "val2", "val3"]).collect(),
        );
        let sz_small = ideal_window_size(&small, true);
        let sz_large = ideal_window_size(&larger, true);
        assert!(sz_large.width >= sz_small.width);
        assert!(sz_large.height > sz_small.height);
    }

    #[test]
    fn ideal_window_size_clamped() {
        // Even a very wide table should not exceed MAX_W
        let wide = make_table(
            (0..100).map(|_| "col").collect(),
            vec![],
        );
        let sz = ideal_window_size(&wide, false);
        assert!(sz.width <= px(1600.0));
        assert!(sz.width >= px(400.0));
    }
}
