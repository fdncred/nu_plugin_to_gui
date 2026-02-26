//! GUI code using `gpui` and `gpui-component` to render table data.
//!
//! # Navigation model
//! The view holds a stack of `TableData` snapshots.  When the user
//! double-clicks a cell that contains a record or list, the nested data is
//! pushed onto the stack and the table re-renders with that data.  A "Back"
//! button in the custom in-window toolbar lets the user return to the previous
//! table.

use crate::TableData;
use nu_protocol::Value;
use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::{Root, StyledExt};
use gpui_component::table::{Table, TableDelegate, TableState, TableEvent, Column, ColumnSort};
use gpui_component::input::{Input, InputState, InputEvent};
use gpui_component::menu::{PopupMenu, PopupMenuItem};
use std::collections::HashMap;
use anyhow::Result;

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
    /// Style for column headers (from `color_config.header`).
    pub header_style: CellStyle,
    /// Parsed `$LS_COLORS` entries (`di`, `ln`, `*.rs`, ...).
    pub ls_colors: HashMap<String, Rgba>,
}

// ---------------------------------------------------------------------------
// Actions
// ---------------------------------------------------------------------------

/// Action for File → Save.
#[derive(Clone, PartialEq)]
struct SaveAction;

impl gpui::Action for SaveAction {
    fn boxed_clone(&self) -> Box<dyn gpui::Action> { Box::new(self.clone()) }
    fn partial_eq(&self, action: &dyn gpui::Action) -> bool {
        action.as_any().downcast_ref::<SaveAction>().is_some()
    }
    fn name(&self) -> &'static str { "to-gui::save" }
    fn name_for_type() -> &'static str { "to-gui::save" }
    fn build(_value: JsonValue) -> gpui::Result<Box<dyn gpui::Action>> where Self: Sized {
        Ok(Box::new(SaveAction))
    }
}
gpui::register_action!(SaveAction);

/// Action emitted by the "Back" button.
#[derive(Clone, PartialEq)]
struct BackAction;

impl gpui::Action for BackAction {
    fn boxed_clone(&self) -> Box<dyn gpui::Action> { Box::new(self.clone()) }
    fn partial_eq(&self, action: &dyn gpui::Action) -> bool {
        action.as_any().downcast_ref::<BackAction>().is_some()
    }
    fn name(&self) -> &'static str { "to-gui::back" }
    fn name_for_type() -> &'static str { "to-gui::back" }
    fn build(_value: JsonValue) -> gpui::Result<Box<dyn gpui::Action>> where Self: Sized {
        Ok(Box::new(BackAction))
    }
}
gpui::register_action!(BackAction);

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

        if autosize {
            for (col_ix, col) in columns.iter_mut().enumerate() {
                let max_len = data
                    .rows
                    .iter()
                    .map(|row| row.get(col_ix).map(|s| s.len()).unwrap_or(0))
                    .chain(std::iter::once(col.name.len()))
                    .max()
                    .unwrap_or(0);
                col.width = ((max_len as f32) * 8.0 + 20.0).into();
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
        self.color_config
            .type_styles
            .get(key)
            .and_then(|style| style.fg)
    }

    fn cell_bg(&self, raw: &Value) -> Option<Rgba> {
        let key = value_type_key(raw);
        self.color_config
            .type_styles
            .get(key)
            .and_then(|style| style.bg)
    }

    fn cell_bold(&self, raw: &Value) -> bool {
        let key = value_type_key(raw);
        self.color_config
            .type_styles
            .get(key)
            .map(|style| style.bold)
            .unwrap_or(false)
    }

    fn ls_fg_for_name_cell(&self, real_row: usize, col_ix: usize) -> Option<Rgba> {
        let col_name = self.columns.get(col_ix).map(|c| c.name.to_lowercase())?;
        if col_name != "name" {
            return None;
        }

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

        let type_col_ix = self
            .columns
            .iter()
            .position(|c| c.name.eq_ignore_ascii_case("type"));
        if let Some(type_col_ix) = type_col_ix {
            let row_ty = self
                .all_rows
                .get(real_row)
                .and_then(|r| r.get(type_col_ix))
                .map(|s| s.to_lowercase())
                .unwrap_or_default();
            let ls_key = match row_ty.as_str() {
                "dir" | "directory" => Some("di"),
                "symlink" | "link" => Some("ln"),
                "pipe" => Some("pi"),
                "socket" => Some("so"),
                "block" | "block_device" => Some("bd"),
                "char" | "char_device" => Some("cd"),
                "file" => Some("fi"),
                _ => None,
            };
            if let Some(ls_key) = ls_key {
                if let Some(c) = self.color_config.ls_colors.get(ls_key) {
                    return Some(*c);
                }
            }
        }

        self.color_config.ls_colors.get("fi").copied()
    }
}

/// Map a `Value` variant to the nushell `color_config` key.
fn value_type_key(v: &Value) -> &'static str {
    match v {
        Value::Bool { .. }     => "bool",
        Value::Int { .. }      => "int",
        Value::Float { .. }    => "float",
        Value::String { .. }   => "string",
        Value::Filesize { .. } => "filesize",
        Value::Duration { .. } => "duration",
        Value::Date { .. }     => "date",
        Value::Range { .. }    => "range",
        Value::Record { .. }   => "record",
        Value::List { .. }     => "list",
        Value::Closure { .. }  => "closure",
        Value::Nothing { .. }  => "nothing",
        Value::Binary { .. }   => "binary",
        Value::CellPath { .. } => "cell-path",
        _                      => "string",
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
        let fg   = self.ls_fg_for_name_cell(real_row, col_ix).or_else(|| self.cell_fg(raw));
        let bg   = self.cell_bg(raw);
        let bold = self.cell_bold(raw);

        let mut div = gpui::div().size_full().child(text);
        if let Some(c) = fg { div = div.text_color(c); }
        if let Some(c) = bg { div = div.bg(c); }
        if bold { div = div.font_weight(FontWeight::BOLD); }
        div = div
            .on_mouse_down(MouseButton::Left, cx.listener(move |table, _, _, cx| {
                table.set_selected_col(col_ix, cx);
            }))
            .on_mouse_down(MouseButton::Right, cx.listener(move |table, _, _, cx| {
                table.set_selected_col(col_ix, cx);
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

/// The main view.  Holds a navigation stack and rebuilds the table on push/pop.
pub struct ToGuiView {
    /// Navigation stack: (data, breadcrumb title)
    nav_stack: Vec<(TableData, String)>,
    filter_input: Entity<InputState>,
    table_state: Entity<TableState<NushellTableDelegate>>,
    autosize: bool,
    color_config: ColorConfig,
    save_dir: String,
    status_message: String,
    /// Copy of the root data used by the Save button.
    root_data: TableData,
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
    ) -> Self {
        let root_data = table_data.clone();
        let (fi, ts) = Self::build_page(
            window, cx, &table_data, initial_filter, autosize, &color_config,
        );
        ToGuiView {
            nav_stack: vec![(table_data, "root".into())],
            filter_input: fi,
            table_state: ts,
            autosize,
            color_config,
            save_dir,
            status_message: String::new(),
            root_data,
        }
    }

    fn save_root_json(&self) -> std::io::Result<std::path::PathBuf> {
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

        let json = serde_json::to_string_pretty(&json_rows).unwrap_or_else(|_| "[]".to_string());
        let path = std::path::PathBuf::from(&self.save_dir).join("to-gui-output.json");
        std::fs::write(&path, json)?;
        Ok(path)
    }

    /// Create the filter widgets and table-state entity for a given `TableData`.
    fn build_page(
        window: &mut Window,
        cx: &mut Context<ToGuiView>,
        data: &TableData,
        initial_filter: Option<String>,
        autosize: bool,
        cc: &ColorConfig,
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
        cx.subscribe_in(&ts, window, move |view, _state, event, window, cx| {
            if let TableEvent::DoubleClickedRow(row_ix) = event {
                let row_ix = *row_ix;
                // Which column is selected (default 0)?
                let col_ix = view.table_state.read(cx).selected_col().unwrap_or(0);
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
                                let nested = crate::value_conv::values_to_table(&[raw.clone()], true);
                                view.push_page(window, cx, nested, title, autosize_c, &cc_clone);
                            }
                            Value::List { vals, .. } if !vals.is_empty() => {
                                let nested = crate::value_conv::values_to_table(vals, true);
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
        self.nav_stack.push((data.clone(), title));
        let (fi, ts) = Self::build_page(window, cx, &data, None, autosize, cc);
        self.filter_input = fi;
        self.table_state  = ts;
        cx.notify();
    }

    fn pop_page(&mut self, window: &mut Window, cx: &mut Context<ToGuiView>) {
        if self.nav_stack.len() > 1 {
            self.nav_stack.pop();
            let (data, _) = self.nav_stack.last().unwrap().clone();
            let cc = self.color_config.clone();
            let (fi, ts) = Self::build_page(window, cx, &data, None, self.autosize, &cc);
            self.filter_input = fi;
            self.table_state  = ts;
            cx.notify();
        }
    }

    fn can_go_back(&self) -> bool { self.nav_stack.len() > 1 }

    fn current_title(&self) -> String {
        self.nav_stack.last().map(|(_, t)| t.clone()).unwrap_or_default()
    }
}

impl Render for ToGuiView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<ToGuiView>) -> impl IntoElement {
        let can_back = self.can_go_back();
        let title    = self.current_title();
        let weak = cx.weak_entity();
        let weak2 = cx.weak_entity();

        let menu_item = |label: &'static str| {
            gpui::div()
                .px_2()
                .py_1()
                .rounded(px(3.0))
                .text_color(rgb(0x111111))
                .child(label)
        };

        let weak_save = cx.weak_entity();
        let menu_bar = gpui::div()
            .h_flex()
            .gap_2()
            .px_3()
            .py_1()
            .w_full()
            .border_b_1()
            .border_color(rgb(0xd5d5d5))
            .bg(rgb(0xf3f4f6))
            .child(
                menu_item("File")
                    .cursor_pointer()
                    .on_mouse_down(MouseButton::Left, move |_, _window, cx| {
                        weak_save
                            .update(cx, |view, cx| {
                                match view.save_root_json() {
                                    Ok(path) => {
                                        view.status_message = format!("Saved: {}", path.display());
                                        eprintln!("to-gui: saved to {}", path.display());
                                    }
                                    Err(err) => {
                                        view.status_message = format!("Save failed: {}", err);
                                        eprintln!("to-gui: save failed: {}", err);
                                    }
                                }
                                cx.notify();
                            })
                            .ok();
                    }),
            )
            .child(menu_item("Edit"))
            .child(menu_item("View"))
            .child(menu_item("Options"))
            .child(menu_item("Window"))
            .child(menu_item("Help"));

        // In-window toolbar (visible on all platforms; primary on Windows/Linux)
        let toolbar = gpui::div()
            .h_flex()
            .gap_2()
            .px_3()
            .py_1()
            .w_full()
            .border_b_1()
            .border_color(rgb(0xcccccc))
            .bg(rgb(0xf5f5f5))
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
                            match view.save_root_json() {
                                Ok(path) => {
                                    view.status_message = format!("Saved: {}", path.display());
                                    eprintln!("to-gui: saved to {}", path.display());
                                }
                                Err(err) => {
                                    view.status_message = format!("Save failed: {}", err);
                                    eprintln!("to-gui: save failed: {}", err);
                                }
                            }
                            cx.notify();
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
// Window sizing helpers
// ---------------------------------------------------------------------------

/// Compute a comfortable initial window size that fits the table content.
///
/// The width is the sum of all column widths (using the same autosize logic as
/// `NushellTableDelegate`) plus a small margin for scrollbar and borders.
/// The height accounts for the toolbar, filter row, header row, and all data
/// rows.  Both dimensions are clamped to a reasonable range so the window is
/// never tiny or larger than a standard monitor.
fn ideal_window_size(table: &TableData, autosize: bool) -> Size<Pixels> {
    const ROW_H: f32 = 36.0;   // data row
    const HEADER_H: f32 = 70.0; // column header row (includes embedded filter input)
    const FILTER_H: f32 = 42.0; // global-filter bar
    const TOOLBAR_H: f32 = 42.0;
    const EXTRA: f32 = 24.0;   // bottom padding / scrollbar
    const MARGIN_W: f32 = 32.0; // side padding / scrollbar
    const MIN_W: f32 = 400.0;
    const MAX_W: f32 = 1600.0;
    const MIN_H: f32 = 280.0;
    const MAX_H: f32 = 1024.0;

    let total_col_w: f32 = table.columns.iter().enumerate().map(|(col_ix, col_name)| {
        if autosize {
            let max_len = table
                .rows
                .iter()
                .map(|row| row.get(col_ix).map(|s| s.len()).unwrap_or(0))
                .chain(std::iter::once(col_name.len()))
                .max()
                .unwrap_or(0);
            (max_len as f32) * 8.0 + 20.0
        } else {
            100.0 // default Column width
        }
    }).sum();

    let width = (total_col_w + MARGIN_W).clamp(MIN_W, MAX_W);
    let height = (TOOLBAR_H + FILTER_H + HEADER_H
        + (table.rows.len() as f32) * ROW_H
        + EXTRA)
        .clamp(MIN_H, MAX_H);

    Size { width: px(width), height: px(height) }
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
                items: vec![MenuItem::action("Save", SaveAction), MenuItem::separator()],
            },
            Menu { name: "Edit".into(), items: vec![] },
            Menu { name: "View".into(), items: vec![] },
            Menu { name: "Options".into(), items: vec![] },
            Menu { name: "Window".into(), items: vec![] },
            Menu { name: "Help".into(), items: vec![] },
        ]);

        let ts = table.clone();
        let save_dir2 = save_dir.clone();
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
        let d = NushellTableDelegate::new(table, true, ColorConfig::default());
        assert!(d.columns[0].width > px(100.0));
    }

    #[test]
    fn autosize_can_be_disabled() {
        let table = make_table(vec!["a"], vec![vec!["loooong"]]);
        let d = NushellTableDelegate::new(table, false, ColorConfig::default());
        assert_eq!(d.columns[0].width, px(100.0));
    }

    #[test]
    fn column_filter_hides_rows() {
        let table = make_table(vec!["a", "b"], vec![vec!["foo", "x"], vec!["bar", "y"]]);
        let mut d = NushellTableDelegate::new(table, false, ColorConfig::default());
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
        let mut d = NushellTableDelegate::new(table, false, ColorConfig::default());
        assert_eq!(d.visible_rows, vec![0, 1]);
        d.visible_rows.sort_by(|a, b| d.all_rows[*a][0].cmp(&d.all_rows[*b][0]));
        assert_eq!(d.visible_rows, vec![1, 0]);
        d.visible_rows = d.original_order.clone();
        assert_eq!(d.visible_rows, vec![0, 1]);
    }

    #[test]
    fn filtering_hides_rows() {
        let table = make_table(vec!["a"], vec![vec!["foo"], vec!["bar"]]);
        let mut d = NushellTableDelegate::new(table, false, ColorConfig::default());
        d.set_filter(Some("ba".into()));
        assert_eq!(d.visible_rows, vec![1]);
        d.set_filter(None);
        assert_eq!(d.visible_rows, vec![0, 1]);
    }

    #[test]
    fn column_filter_special_terms() {
        let table = make_table(vec!["a"], vec![vec!["abc"], vec!["ab"], vec!["xbc"]]);
        let mut d = NushellTableDelegate::new(table, false, ColorConfig::default());
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
        let _ = run_table_gui(dummy, None, false, ColorConfig::default());
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
