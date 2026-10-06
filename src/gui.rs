//! GUI code using `gpui` and `gpui-component` to render table data.
//!
//! # Navigation model
//! The view holds a stack of live page states. When the user double-clicks a
//! cell containing a record or list, a nested page is pushed. Pressing Back
//! restores the previous page state (including table position/selection).

use crate::TableData;
use crate::color_utils::{ensure_contrast, style_cache_key, value_type_key};
use crate::gui_ansi::parse_ansi_segments;
use crate::gui_dispatch::GuiLaunch;
use crate::settings::{clamp_font_size, font_size_config_line};
use crate::window_sizing::{
    autosize_column_width, ideal_window_size, row_height, title_bar_height, unsized_column_width,
};
use anyhow::{Result, anyhow};
use gpui::assets::IconName as LucideIcon;
use gpui::base::Selectable as _;
use gpui::component::breadcrumb::{Breadcrumb, BreadcrumbItem};
use gpui::component::button::{Button, ButtonVariants as _};
use gpui::component::empty::{
    Empty, EmptyContent, EmptyDescription, EmptyHeader, EmptyMedia, EmptyTitle,
};
use gpui::component::input::{Input, InputEvent, InputState};
use gpui::component::kbd::Kbd;
use gpui::component::menu::{PopupMenu, PopupMenuItem};
use gpui::component::notification::Notification;
use gpui::component::status_bar::StatusBar;
use gpui::component::table::{
    Column, ColumnSort, DataTable, TableDelegate, TableEvent, TableState,
};
use gpui::component::{
    ActiveTheme as _, Disableable as _, Icon, IconName, Root, Sizable as _, StyledExt, Theme,
    ThemeMode, TitleBar, WindowExt as _, h_flex, v_flex,
};
use gpui::prelude::FluentBuilder as _;
use gpui::*;
use nu_protocol::{Config, Value};
use std::any::Any;
use std::collections::HashMap;
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
#[derive(Clone, Copy, Debug, Default)]
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
    pub ls_colors: HashMap<String, CellStyle>,
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
            fn boxed_clone(&self) -> Box<dyn gpui::Action> {
                Box::new(self.clone())
            }
            fn partial_eq(&self, action: &dyn gpui::Action) -> bool {
                action.as_any().downcast_ref::<$name>().is_some()
            }
            fn name(&self) -> &'static str {
                $id
            }
            fn name_for_type() -> &'static str {
                $id
            }
            fn build(_value: JsonValue) -> gpui::Result<Box<dyn gpui::Action>>
            where
                Self: Sized,
            {
                Ok(Box::new($name))
            }
        }

        gpui::register_action!($name);
    };
}

// Every action here is wired to a handler; menus only list what works.
define_action!(SaveAction, "to-gui::save");
define_action!(CloseWindowAction, "to-gui::close-window");
define_action!(QuitAction, "to-gui::quit");
define_action!(CopyAction, "to-gui::copy");
define_action!(FindAction, "to-gui::find");
define_action!(ToggleFiltersAction, "to-gui::toggle-filters");
define_action!(ClearFiltersAction, "to-gui::clear-filters");
define_action!(MinimizeAction, "to-gui::minimize");
define_action!(ZoomWindowAction, "to-gui::zoom-window");
define_action!(AboutAction, "to-gui::about");
define_action!(BackAction, "to-gui::back");
define_action!(ForwardAction, "to-gui::forward");
define_action!(IncreaseFontSizeAction, "to-gui::increase-font-size");
define_action!(DecreaseFontSizeAction, "to-gui::decrease-font-size");
define_action!(ResetFontSizeAction, "to-gui::reset-font-size");

/// Key context for the main view, used to scope key bindings.
const KEY_CONTEXT: &str = "ToGui";

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
    /// Render column headers as filter inputs instead of labels.
    show_filter_inputs: bool,
    /// Size columns to their content instead of a fixed width.
    autosize: bool,
    /// Base UI font size; column widths scale with it.
    font_size: f32,
}

impl NushellTableDelegate {
    pub fn new(
        data: TableData,
        autosize: bool,
        color_config: ColorConfig,
        column_filter_inputs: Vec<Entity<InputState>>,
        font_size: f32,
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

        let original_order: Vec<usize> = (0..count).collect();
        let mut delegate = NushellTableDelegate {
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
            show_filter_inputs: false,
            autosize,
            font_size,
        };
        delegate.size_columns();
        delegate
    }

    /// Change the font size and resize columns to match. The table must be
    /// refreshed afterwards to pick up the new widths.
    pub fn set_font_size(&mut self, font_size: f32) {
        self.font_size = font_size;
        self.size_columns();
    }

    fn size_columns(&mut self) {
        for (col_ix, col) in self.columns.iter_mut().enumerate() {
            let width = if self.autosize {
                let max_len = self
                    .all_rows
                    .iter()
                    .map(|row| row.get(col_ix).map(|s| s.len()).unwrap_or(0))
                    .max()
                    .unwrap_or(0);
                autosize_column_width(max_len, col.name.len(), self.font_size)
            } else {
                unsized_column_width(self.font_size)
            };
            col.width = px(width);
        }
    }

    pub fn total_rows(&self) -> usize {
        self.all_rows.len()
    }

    pub fn active_column_filters(&self) -> usize {
        self.column_filters.iter().filter(|f| f.is_some()).count()
    }

    pub fn has_filters(&self) -> bool {
        self.filter.is_some() || self.active_column_filters() > 0
    }

    pub fn clear_filters(&mut self) {
        self.filter = None;
        self.column_filters.iter_mut().for_each(|f| *f = None);
        self.apply_filter();
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
                if let Some(ref pat) = global
                    && !row
                        .iter()
                        .any(|cell| cell.to_lowercase().contains(pat.as_str()))
                {
                    return false;
                }
                for (col_ix, filt) in self.column_filters.iter().enumerate() {
                    if let Some(pat) = filt
                        && let Some(cell) = row.get(col_ix)
                        && !matches(cell, pat)
                    {
                        return false;
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

    fn ls_style_for_name_cell(&self, real_row: usize, col_ix: usize) -> Option<CellStyle> {
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

            if let Some(dot) = name.rfind('.')
                && dot + 1 < name.len()
            {
                let ext = &name[dot + 1..];
                if let Some(style) = self.color_config.ls_colors.get(&format!("*.{}", ext)) {
                    return Some(*style);
                }
            }
        }

        if let Some(ls_key) = self.ls_key_for_row_type(real_row)
            && let Some(style) = self.color_config.ls_colors.get(ls_key)
        {
            return Some(*style);
        }

        self.color_config.ls_colors.get("fi").copied()
    }
}

impl TableDelegate for NushellTableDelegate {
    fn columns_count(&self, _: &App) -> usize {
        self.columns.len()
    }
    fn rows_count(&self, _: &App) -> usize {
        self.visible_rows.len()
    }
    fn column(&self, col_ix: usize, _: &App) -> Column {
        self.columns[col_ix].clone()
    }

    fn render_th(
        &mut self,
        col_ix: usize,
        _window: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> impl gpui::IntoElement {
        let name = self.columns[col_ix].name.clone();
        if self.show_filter_inputs
            && let Some(inp) = self.column_filter_inputs.get(col_ix)
        {
            return gpui::div()
                .w_full()
                .pr_1()
                .child(Input::new(inp).xsmall().cleanable(true))
                .into_any_element();
        }

        let filtered = self.column_filters.get(col_ix).is_some_and(|f| f.is_some());
        let header_style = &self.color_config.header_style;
        let mut label = h_flex()
            .gap_1()
            .min_w_0()
            .font_weight(if header_style.bold {
                FontWeight::BOLD
            } else {
                FontWeight::MEDIUM
            })
            .text_color(cx.theme().muted_foreground)
            .child(gpui::div().truncate().child(name));
        if let Some(c) = header_style.fg {
            let header_bg = header_style
                .bg
                .unwrap_or_else(|| solid_color(cx.theme().table_head, cx));
            label = label.text_color(ensure_contrast(c, header_bg));
        }
        if let Some(c) = header_style.bg {
            label = label.bg(c);
        }
        if filtered {
            label = label.child(
                Icon::new(LucideIcon::Funnel)
                    .xsmall()
                    .text_color(cx.theme().blue),
            );
        }
        label.into_any_element()
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
        let raw = &self.raw_rows[real_row][col_ix];
        let key_style = if self.is_transposed_key_column(col_ix) {
            self.cellpath_style()
        } else {
            None
        };
        let ls_style = self.ls_style_for_name_cell(real_row, col_ix);
        let fg = ls_style
            .and_then(|style| style.fg)
            .or_else(|| key_style.and_then(|style| style.fg))
            .or_else(|| self.cell_fg(raw));
        let bg = key_style
            .and_then(|style| style.bg)
            .or_else(|| self.cell_bg(raw));
        // LS_COLORS backgrounds (e.g. README files) highlight the text itself,
        // as `ls` does in a terminal, rather than the whole cell.
        let chip_bg = ls_style.and_then(|style| style.bg);
        let bold = ls_style.is_some_and(|style| style.bold)
            || key_style
                .map(|style| style.bold)
                .unwrap_or_else(|| self.cell_bold(raw));
        // Keep every color readable against what it is drawn on.
        let cell_bg = bg.unwrap_or_else(|| table_background(cx));
        let fg = fg.map(|c| ensure_contrast(c, chip_bg.unwrap_or(cell_bg)));
        let numeric = numeric_type_key_for_value(raw).is_some();
        let mut has_ansi_segments = false;

        let mut div = gpui::div().size_full().flex().items_center();
        if let Some(segments) = parse_ansi_segments(&text) {
            has_ansi_segments = true;
            let mut text_row = gpui::div().h_flex().gap_0().w_full();
            for segment in segments.into_iter().filter(|seg| !seg.text.is_empty()) {
                let mut part = gpui::div().child(segment.text);
                if let Some(c) = segment.fg {
                    part = part.text_color(ensure_contrast(c, cell_bg));
                }
                if segment.bold {
                    part = part.font_weight(FontWeight::BOLD);
                }
                text_row = text_row.child(part);
            }
            div = div.child(text_row);
        } else if is_drillable(raw) {
            // Nested values open on double-click; show that they lead somewhere.
            div = div
                .h_flex()
                .gap_1()
                .justify_between()
                .text_color(cx.theme().muted_foreground)
                .child(gpui::div().truncate().child(text))
                .child(Icon::new(IconName::ChevronRight).xsmall());
        } else if let Some(chip) = chip_bg {
            div = div.child(
                gpui::div()
                    .ml(px(-3.))
                    .px(px(3.))
                    .rounded_sm()
                    .bg(chip)
                    .truncate()
                    .child(text),
            );
        } else {
            div = div.child(text);
        }
        if let Some(c) = fg
            && !has_ansi_segments
        {
            div = div.text_color(c);
        }
        if let Some(c) = bg {
            div = div.bg(c);
        }
        if bold && !has_ansi_segments {
            div = div.font_weight(FontWeight::BOLD);
        }
        if numeric {
            div = div.justify_end();
        }
        div = div
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |table, _, _, _cx| {
                    table.delegate_mut().last_clicked_col = Some(col_ix);
                }),
            )
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |table, _, _, _cx| {
                    table.delegate_mut().last_clicked_col = Some(col_ix);
                    table.delegate_mut().right_clicked_col = Some(col_ix);
                }),
            );
        div
    }

    fn perform_sort(
        &mut self,
        col_ix: usize,
        sort: ColumnSort,
        _: &mut Window,
        _: &mut Context<TableState<Self>>,
    ) {
        // Mirror the table's sort state so `TableState::refresh` (used when the
        // font size changes) keeps the sort indicator.
        for (ix, col) in self.columns.iter_mut().enumerate() {
            col.sort = Some(if ix == col_ix {
                sort
            } else {
                ColumnSort::Default
            });
        }
        match sort {
            ColumnSort::Ascending => self
                .visible_rows
                .sort_by(|a, b| self.all_rows[*a][col_ix].cmp(&self.all_rows[*b][col_ix])),
            ColumnSort::Descending => self
                .visible_rows
                .sort_by(|a, b| self.all_rows[*b][col_ix].cmp(&self.all_rows[*a][col_ix])),
            ColumnSort::Default => self.visible_rows = self.original_order.clone(),
        }
    }

    fn render_empty(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        let (icon, title, description) = if self.all_rows.is_empty() {
            (
                Icon::new(IconName::Inbox),
                "No data",
                "The pipeline produced nothing to show.",
            )
        } else {
            (
                Icon::new(LucideIcon::SearchX),
                "No matching rows",
                "Nothing matches the current search and filters.",
            )
        };
        let can_clear = !self.all_rows.is_empty() && self.has_filters();

        h_flex().size_full().justify_center().child(
            Empty::new()
                .header(
                    EmptyHeader::new()
                        .media(EmptyMedia::new().child(icon))
                        .title(EmptyTitle::new().child(title))
                        .description(
                            EmptyDescription::new()
                                .text_color(cx.theme().muted_foreground)
                                .child(description),
                        ),
                )
                .when(can_clear, |empty| {
                    empty.content(
                        EmptyContent::new().child(
                            Button::new("clear-filters")
                                .outline()
                                .small()
                                .label("Clear filters")
                                .on_click(|_, window, cx| {
                                    window.dispatch_action(Box::new(ClearFiltersAction), cx)
                                }),
                        ),
                    )
                }),
        )
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
            PopupMenuItem::new("Copy")
                .icon(IconName::Copy)
                .on_click(move |_, _, cx| {
                    cx.write_to_clipboard(ClipboardItem::new_string(text.clone()));
                }),
        )
    }
}

/// `color` as an opaque color, using the window background where it is
/// transparent (theme colors such as `table_head` may be).
fn solid_color(color: Hsla, cx: &App) -> Rgba {
    if color.a >= 1.0 {
        color.to_rgb()
    } else {
        cx.theme().background.to_rgb()
    }
}

/// The color table cells are drawn on.
fn table_background(cx: &App) -> Rgba {
    solid_color(cx.theme().table, cx)
}

/// Whether double-clicking a cell with this value opens a nested page.
fn is_drillable(v: &Value) -> bool {
    match v {
        Value::Record { .. } => true,
        Value::List { vals, .. } => !vals.is_empty(),
        _ => false,
    }
}

/// Format a count with thousands separators, e.g. `12,345`.
fn fmt_count(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{} {}", fmt_count(n), if n == 1 { one } else { many })
}

// ---------------------------------------------------------------------------
// View
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct NavPage {
    /// Breadcrumb label: `root`, a record key, or `row.column`.
    crumb: SharedString,
    filter_input: Entity<InputState>,
    table_state: Entity<TableState<NushellTableDelegate>>,
}

/// The main view. Holds a navigation stack of live page states.
pub struct ToGuiView {
    /// Pages from the root to the current page.
    nav_stack: Vec<NavPage>,
    /// Pages left by going back, most recent last.
    forward_stack: Vec<NavPage>,
    filter_input: Entity<InputState>,
    table_state: Entity<TableState<NushellTableDelegate>>,
    focus_handle: FocusHandle,
    /// Whether column headers show filter inputs.
    show_column_filters: bool,
    /// Current base font size.
    font_size: f32,
    /// Font size from config.nu; Reset returns to it.
    configured_font_size: f32,
    save_dir: String,
    status_message: String,
    /// Copy of the root data used by the Save button.
    root_data: TableData,
    /// Shared page construction settings.
    settings: Arc<ViewSettings>,
}

#[derive(Clone)]
struct ViewSettings {
    autosize: bool,
    color_config: ColorConfig,
    closure_sources: Arc<HashMap<usize, String>>,
    table_config: Arc<Config>,
    rfc3339: bool,
}

/// Breadcrumb label for a drilled-into cell, in Nushell cell-path style.
fn crumb_for_cell(data: &TableData, row: usize, col: usize) -> String {
    let is_key_value = data.columns.len() == 2
        && data.columns[0].eq_ignore_ascii_case("key")
        && data.columns[1].eq_ignore_ascii_case("value");
    if is_key_value && let Some(key) = data.rows.get(row).and_then(|r| r.first()) {
        return key.clone();
    }
    let col_name = data.columns.get(col).map_or("?", |s| s.as_str());
    format!("{row}.{col_name}")
}

impl ToGuiView {
    pub fn new(window: &mut Window, cx: &mut Context<ToGuiView>, launch: GuiLaunch) -> Self {
        let GuiLaunch {
            table: table_data,
            initial_filter,
            autosize,
            color_config,
            save_dir,
            closure_sources,
            table_config,
            rfc3339,
            font_size,
        } = launch;
        let font_size = clamp_font_size(font_size);

        let root_data = table_data.clone();
        let settings = Arc::new(ViewSettings {
            autosize,
            color_config,
            closure_sources: Arc::new(closure_sources),
            table_config: Arc::new(table_config),
            rfc3339,
        });

        let (fi, ts) = Self::build_page(
            window,
            cx,
            &table_data,
            initial_filter,
            &settings,
            font_size,
        );

        let root_page = NavPage {
            crumb: "root".into(),
            filter_input: fi.clone(),
            table_state: ts.clone(),
        };

        let focus_handle = cx.focus_handle();
        focus_handle.focus(window, cx);

        ToGuiView {
            nav_stack: vec![root_page],
            forward_stack: Vec::new(),
            filter_input: fi,
            table_state: ts,
            focus_handle,
            show_column_filters: false,
            font_size,
            configured_font_size: font_size,
            save_dir,
            status_message: String::new(),
            root_data,
            settings,
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
            .map_err(|err| std::io::Error::other(err.to_string()))
    }

    fn save_root_json_to(&self, path: &Path) -> std::io::Result<()> {
        let json = self.root_json_string()?;
        std::fs::write(path, json)
    }

    fn start_save_as(&mut self, window: &mut Window, cx: &mut Context<ToGuiView>) {
        let base_dir = PathBuf::from(&self.save_dir);
        let receiver = cx.prompt_for_new_path(&base_dir, Some("to-gui-output.json"));

        cx.spawn_in(window, async move |view, cx| {
            let chosen = match receiver.await {
                Ok(Ok(Some(path))) => Ok(path),
                Ok(Ok(None)) => return,
                Ok(Err(err)) => Err(err.to_string()),
                Err(err) => Err(err.to_string()),
            };
            let _ = view.update_in(cx, |view, window, cx| {
                let saved = chosen.and_then(|path| {
                    view.save_root_json_to(&path)
                        .map(|()| path)
                        .map_err(|err| err.to_string())
                });
                let note = match saved {
                    Ok(path) => {
                        view.status_message = format!("Saved {}", path.display());
                        let name = path.file_name().map_or_else(
                            || path.display().to_string(),
                            |n| n.to_string_lossy().into_owned(),
                        );
                        Notification::success(format!("Saved {name}"))
                    }
                    Err(err) => {
                        view.status_message = format!("Save failed: {err}");
                        Notification::error(format!("Save failed: {err}"))
                    }
                };
                window.push_notification(note, cx);
                cx.notify();
            });
        })
        .detach();
    }

    /// Create the filter widgets and table-state entity for a given `TableData`.
    fn build_page(
        window: &mut Window,
        cx: &mut Context<ToGuiView>,
        data: &TableData,
        initial_filter: Option<String>,
        settings: &Arc<ViewSettings>,
        font_size: f32,
    ) -> (Entity<InputState>, Entity<TableState<NushellTableDelegate>>) {
        // Per-column filter inputs — owned by the delegate, rendered inside headers.
        let col_inputs: Vec<Entity<InputState>> = data
            .columns
            .iter()
            .map(|name| {
                cx.new(|cx| {
                    let mut input = InputState::new(window, cx);
                    input.set_placeholder(name.clone(), window, cx);
                    input
                })
            })
            .collect();
        // Keep a clone for subscriptions; the originals move into the delegate.
        let col_inputs_for_subs = col_inputs.clone();

        let delegate = NushellTableDelegate::new(
            data.clone(),
            settings.autosize,
            settings.color_config.clone(),
            col_inputs,
            font_size,
        );

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
            input.set_placeholder("Search", window, cx);
        });

        // Global filter subscription
        let ts2 = ts.clone();
        cx.subscribe_in(&fi, window, move |_v, input, event, _, cx| {
            if let InputEvent::Change = event {
                let s = input.read(cx).value().to_string();
                ts2.update(cx, |t, cx| {
                    t.delegate_mut()
                        .set_filter(if s.is_empty() { None } else { Some(s) });
                    cx.notify();
                });
                cx.notify();
            }
        })
        .detach();

        // Per-column filter subscriptions
        for (col_ix, inp) in col_inputs_for_subs.iter().enumerate() {
            let ts3 = ts.clone();
            cx.subscribe_in(inp, window, move |_v, input, event, _, cx| {
                if let InputEvent::Change = event {
                    let pat = input.read(cx).value().to_string();
                    ts3.update(cx, |t, cx| {
                        t.delegate_mut().set_column_filter(
                            col_ix,
                            if pat.is_empty() { None } else { Some(pat) },
                        );
                        cx.notify();
                    });
                    cx.notify();
                }
            })
            .detach();
        }

        // Apply initial global filter
        if let Some(f) = initial_filter {
            fi.update(cx, |i, cx| i.set_value(f.clone(), window, cx));
            ts.update(cx, |t, _| t.delegate_mut().set_filter(Some(f)));
        }

        // Re-render the status bar when the selection changes.
        cx.observe(&ts, |_, _, cx| cx.notify()).detach();

        // Subscribe to DoubleClickedRow to navigate into nested values
        let data_clone = data.clone();
        let settings_c = settings.clone();
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
                if let Some(raw_row) = data_clone.raw.get(real_row)
                    && let Some(raw) = raw_row.get(col_ix).cloned()
                {
                    let crumb = crumb_for_cell(&data_clone, real_row, col_ix);
                    match &raw {
                        Value::Record { .. } => {
                            let nested =
                                crate::value_conv::values_to_table_with_closure_sources_and_config(
                                    std::slice::from_ref(&raw),
                                    true,
                                    &settings_c.closure_sources,
                                    &settings_c.table_config,
                                    settings_c.rfc3339,
                                );
                            view.push_page(window, cx, nested, crumb);
                        }
                        Value::List { vals, .. } if !vals.is_empty() => {
                            let nested =
                                crate::value_conv::values_to_table_with_closure_sources_and_config(
                                    vals,
                                    true,
                                    &settings_c.closure_sources,
                                    &settings_c.table_config,
                                    settings_c.rfc3339,
                                );
                            view.push_page(window, cx, nested, crumb);
                        }
                        _ => {}
                    }
                }
            }
        })
        .detach();

        (fi, ts)
    }

    fn push_page(
        &mut self,
        window: &mut Window,
        cx: &mut Context<ToGuiView>,
        data: TableData,
        crumb: String,
    ) {
        let (fi, ts) = Self::build_page(window, cx, &data, None, &self.settings, self.font_size);
        let show = self.show_column_filters;
        ts.update(cx, |t, _| t.delegate_mut().show_filter_inputs = show);

        self.forward_stack.clear();
        self.nav_stack.push(NavPage {
            crumb: crumb.into(),
            filter_input: fi,
            table_state: ts,
        });
        self.sync_current_page(cx);
    }

    fn sync_current_page(&mut self, cx: &mut Context<ToGuiView>) {
        if let Some(page) = self.nav_stack.last() {
            self.filter_input = page.filter_input.clone();
            self.table_state = page.table_state.clone();
        }
        cx.notify();
    }

    fn go_back(&mut self, cx: &mut Context<ToGuiView>) {
        if self.nav_stack.len() > 1
            && let Some(page) = self.nav_stack.pop()
        {
            self.forward_stack.push(page);
            self.sync_current_page(cx);
        }
    }

    fn go_forward(&mut self, cx: &mut Context<ToGuiView>) {
        if let Some(page) = self.forward_stack.pop() {
            self.nav_stack.push(page);
            self.sync_current_page(cx);
        }
    }

    /// Go back until the page at `depth` (0 = root) is current.
    fn go_to_depth(&mut self, depth: usize, cx: &mut Context<ToGuiView>) {
        while self.nav_stack.len() > depth + 1 {
            self.go_back(cx);
        }
    }

    fn focus_search(&mut self, window: &mut Window, cx: &mut Context<ToGuiView>) {
        let handle = self.filter_input.read(cx).focus_handle(cx);
        handle.focus(window, cx);
    }

    fn toggle_column_filters(&mut self, window: &mut Window, cx: &mut Context<ToGuiView>) {
        self.show_column_filters = !self.show_column_filters;
        let show = self.show_column_filters;
        for page in self.nav_stack.iter().chain(self.forward_stack.iter()) {
            page.table_state.update(cx, |t, cx| {
                t.delegate_mut().show_filter_inputs = show;
                cx.notify();
            });
        }

        let first_input = self
            .table_state
            .read(cx)
            .delegate()
            .column_filter_inputs
            .first()
            .cloned();
        match first_input {
            Some(input) if show => input.read(cx).focus_handle(cx).focus(window, cx),
            _ => self.focus_handle.focus(window, cx),
        }
        cx.notify();
    }

    fn clear_filters(&mut self, window: &mut Window, cx: &mut Context<ToGuiView>) {
        let inputs: Vec<Entity<InputState>> = std::iter::once(self.filter_input.clone())
            .chain(
                self.table_state
                    .read(cx)
                    .delegate()
                    .column_filter_inputs
                    .iter()
                    .cloned(),
            )
            .collect();
        for input in inputs {
            input.update(cx, |state, cx| state.set_value("", window, cx));
        }
        self.table_state.update(cx, |t, cx| {
            t.delegate_mut().clear_filters();
            cx.notify();
        });
        cx.notify();
    }

    /// Copy the selected cell, or the selected row as tab-separated text.
    fn copy_selection(&mut self, cx: &mut Context<ToGuiView>) {
        let text = {
            let table = self.table_state.read(cx);
            let d = table.delegate();
            if let Some((row, col)) = table.selected_cell() {
                d.visible_rows
                    .get(row)
                    .and_then(|&r| d.all_rows[r].get(col))
                    .cloned()
            } else if let Some(row) = table.selected_row() {
                d.visible_rows.get(row).map(|&r| d.all_rows[r].join("\t"))
            } else {
                None
            }
        };
        if let Some(text) = text {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
            self.status_message = "Copied to clipboard".to_string();
            cx.notify();
        }
    }

    /// Apply a new base font size to the theme and every page's columns.
    ///
    /// The change lasts for this window; config.nu holds the size used at
    /// launch (see [`Self::copy_font_size_setting`]).
    fn set_font_size(&mut self, font_size: f32, cx: &mut Context<ToGuiView>) {
        let font_size = clamp_font_size(font_size);
        if font_size == self.font_size {
            return;
        }
        self.font_size = font_size;
        Theme::update(cx, |theme| theme.font_size = px(font_size));
        for page in self.nav_stack.iter().chain(self.forward_stack.iter()) {
            page.table_state.update(cx, |t, cx| {
                t.delegate_mut().set_font_size(font_size);
                t.refresh(cx);
            });
        }
        cx.notify();
    }

    /// Copy the config.nu line that makes the current size the default.
    fn copy_font_size_setting(&mut self, window: &mut Window, cx: &mut Context<ToGuiView>) {
        cx.write_to_clipboard(ClipboardItem::new_string(font_size_config_line(
            self.font_size,
        )));
        window.push_notification(
            Notification::success(format!(
                "Paste into config.nu to open at {}px next time",
                self.font_size
            ))
            .title("Copied font size setting"),
            cx,
        );
    }

    fn render_font_size_control(&self, cx: &mut Context<ToGuiView>) -> impl IntoElement {
        let changed = self.font_size != self.configured_font_size;
        h_flex()
            .gap_0p5()
            .when(changed, |this| {
                this.child(
                    Button::new("copy-font-size")
                        .ghost()
                        .xsmall()
                        .icon(IconName::Copy)
                        .label("Copy setting")
                        .tooltip("Copy the config.nu line that keeps this font size")
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.copy_font_size_setting(window, cx)
                        })),
                )
            })
            .child(
                Button::new("font-smaller")
                    .ghost()
                    .xsmall()
                    .icon(LucideIcon::Minus)
                    .tooltip_with_action("Decrease font size", &DecreaseFontSizeAction, None)
                    .on_click(
                        cx.listener(|this, _, _, cx| this.set_font_size(this.font_size - 1.0, cx)),
                    ),
            )
            .child(
                Button::new("font-reset")
                    .ghost()
                    .xsmall()
                    .label(format!("{}px", self.font_size))
                    .tooltip_with_action("Reset font size", &ResetFontSizeAction, None)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.set_font_size(this.configured_font_size, cx)
                    })),
            )
            .child(
                Button::new("font-larger")
                    .ghost()
                    .xsmall()
                    .icon(LucideIcon::Plus)
                    .tooltip_with_action("Increase font size", &IncreaseFontSizeAction, None)
                    .on_click(
                        cx.listener(|this, _, _, cx| this.set_font_size(this.font_size + 1.0, cx)),
                    ),
            )
    }

    fn show_about(&mut self, window: &mut Window, cx: &mut Context<ToGuiView>) {
        window.open_dialog(cx, |dialog, _, cx| {
            dialog.title("to gui").w(px(380.)).child(
                v_flex()
                    .gap_2()
                    .text_sm()
                    .child(format!("Version {}", env!("CARGO_PKG_VERSION")))
                    .child(
                        gpui::div()
                            .text_color(cx.theme().muted_foreground)
                            .child("A Nushell plugin that opens pipeline data in a desktop table viewer. Built with GPUI Kit."),
                    ),
            )
        });
    }

    fn render_title_bar(&self, cx: &mut Context<ToGuiView>) -> impl IntoElement {
        let depth = self.nav_stack.len();
        let crumbs: Vec<BreadcrumbItem> = self
            .nav_stack
            .iter()
            .enumerate()
            .map(|(ix, page)| {
                let item = BreadcrumbItem::new(page.crumb.clone());
                if ix + 1 < depth {
                    let weak = cx.weak_entity();
                    item.on_click(move |_, _, cx| {
                        weak.update(cx, |view, cx| view.go_to_depth(ix, cx)).ok();
                    })
                } else {
                    item
                }
            })
            .collect();

        let search_empty = self.filter_input.read(cx).value().is_empty();
        let find_kbd = search_empty
            .then(|| Keystroke::parse("secondary-f").ok())
            .flatten()
            .map(Kbd::new);

        // Clicks on controls must not start a title-bar window drag.
        let controls = || {
            h_flex()
                .gap_1()
                .items_center()
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        };

        TitleBar::new()
            .h(px(title_bar_height(self.font_size)))
            .child(
                h_flex()
                    .gap_2()
                    .min_w_0()
                    .flex_1()
                    .child(
                        controls()
                            .child(
                                Button::new("nav-back")
                                    .ghost()
                                    .small()
                                    .icon(IconName::ChevronLeft)
                                    .disabled(depth <= 1)
                                    .tooltip_with_action("Back", &BackAction, None)
                                    .on_click(cx.listener(|this, _, _, cx| this.go_back(cx))),
                            )
                            .child(
                                Button::new("nav-forward")
                                    .ghost()
                                    .small()
                                    .icon(IconName::ChevronRight)
                                    .disabled(self.forward_stack.is_empty())
                                    .tooltip_with_action("Forward", &ForwardAction, None)
                                    .on_click(cx.listener(|this, _, _, cx| this.go_forward(cx))),
                            ),
                    )
                    .child(Breadcrumb::new().min_w_0().children(crumbs)),
            )
            .child(
                controls()
                    .pr_2()
                    .child(
                        gpui::div().w(px(240.)).child(
                            Input::new(&self.filter_input)
                                .small()
                                .cleanable(true)
                                .prefix(
                                    Icon::new(IconName::Search)
                                        .small()
                                        .text_color(cx.theme().muted_foreground),
                                )
                                .when_some(find_kbd, |input, kbd| input.suffix(kbd)),
                        ),
                    )
                    .child(
                        Button::new("toggle-filters")
                            .ghost()
                            .small()
                            .icon(LucideIcon::ListFilter)
                            .selected(self.show_column_filters)
                            .tooltip_with_action("Column filters", &ToggleFiltersAction, None)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.toggle_column_filters(window, cx)
                            })),
                    )
                    .child(
                        Button::new("save")
                            .ghost()
                            .small()
                            .icon(LucideIcon::Download)
                            .tooltip_with_action("Save as JSON…", &SaveAction, None)
                            .on_click(
                                cx.listener(|this, _, window, cx| this.start_save_as(window, cx)),
                            ),
                    ),
            )
    }
}

impl Render for ToGuiView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<ToGuiView>) -> impl IntoElement {
        let (visible, total, cols, col_filters, selected_row) = {
            let table = self.table_state.read(cx);
            let d = table.delegate();
            (
                d.visible_rows.len(),
                d.total_rows(),
                d.columns.len(),
                d.active_column_filters(),
                table.selected_row(),
            )
        };

        let rows_label = if visible == total {
            plural(total, "row", "rows")
        } else {
            format!("{} of {}", fmt_count(visible), plural(total, "row", "rows"))
        };

        let status_bar = StatusBar::new()
            .left(
                h_flex()
                    .gap_1()
                    .child(Icon::new(LucideIcon::Rows3).xsmall())
                    .child(format!(
                        "{rows_label} · {}",
                        plural(cols, "column", "columns")
                    )),
            )
            .when(col_filters > 0, |bar| {
                bar.left(
                    h_flex()
                        .gap_1()
                        .text_color(cx.theme().blue)
                        .child(Icon::new(LucideIcon::Funnel).xsmall())
                        .child(plural(col_filters, "column filter", "column filters")),
                )
            })
            .when(!self.status_message.is_empty(), |bar| {
                bar.right(self.status_message.clone())
            })
            .when_some(selected_row, |bar, row| {
                bar.right(format!("Row {}", fmt_count(row + 1)))
            })
            .right(self.render_font_size_control(cx));

        v_flex()
            .id("to-gui")
            .key_context(KEY_CONTEXT)
            .track_focus(&self.focus_handle)
            .on_action(
                cx.listener(|this, _: &SaveAction, window, cx| this.start_save_as(window, cx)),
            )
            .on_action(cx.listener(|_, _: &CloseWindowAction, window, _| window.remove_window()))
            .on_action(cx.listener(|this, _: &CopyAction, _, cx| this.copy_selection(cx)))
            .on_action(
                cx.listener(|this, _: &FindAction, window, cx| this.focus_search(window, cx)),
            )
            .on_action(cx.listener(|this, _: &ToggleFiltersAction, window, cx| {
                this.toggle_column_filters(window, cx)
            }))
            .on_action(cx.listener(|this, _: &ClearFiltersAction, window, cx| {
                this.clear_filters(window, cx)
            }))
            .on_action(cx.listener(|this, _: &BackAction, _, cx| this.go_back(cx)))
            .on_action(cx.listener(|this, _: &ForwardAction, _, cx| this.go_forward(cx)))
            .on_action(cx.listener(|_, _: &MinimizeAction, window, _| window.minimize_window()))
            .on_action(cx.listener(|_, _: &ZoomWindowAction, window, _| window.zoom_window()))
            .on_action(cx.listener(|this, _: &AboutAction, window, cx| this.show_about(window, cx)))
            .on_action(cx.listener(|this, _: &IncreaseFontSizeAction, _, cx| {
                this.set_font_size(this.font_size + 1.0, cx)
            }))
            .on_action(cx.listener(|this, _: &DecreaseFontSizeAction, _, cx| {
                this.set_font_size(this.font_size - 1.0, cx)
            }))
            .on_action(cx.listener(|this, _: &ResetFontSizeAction, _, cx| {
                this.set_font_size(this.configured_font_size, cx)
            }))
            .size_full()
            .child(self.render_title_bar(cx))
            .child(
                DataTable::new(&self.table_state)
                    .with_size(gpui::component::Size::Size(px(row_height(self.font_size))))
                    .stripe(false)
                    .bordered(false)
                    .scrollbar_visible(true, true),
            )
            .child(status_bar)
    }
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

#[cfg(not(test))]
fn panic_payload_to_string(payload: Box<dyn Any + Send>) -> String {
    if let Some(msg) = payload.downcast_ref::<String>() {
        return msg.clone();
    }
    if let Some(msg) = payload.downcast_ref::<&'static str>() {
        return (*msg).to_string();
    }
    "unknown panic payload".to_string()
}

// Icons outside the default component bundle. Embedding the full Lucide
// catalog would add several megabytes to the plugin binary.
gpui::assets::icon_assets!(
    ExtraIcons,
    [Download, Funnel, ListFilter, Minus, Plus, Rows3, SearchX]
);

/// The default component icons plus [`ExtraIcons`].
struct AppAssets;

impl AssetSource for AppAssets {
    fn load(&self, path: &str) -> gpui::Result<Option<std::borrow::Cow<'static, [u8]>>> {
        match ExtraIcons.load(path)? {
            Some(data) => Ok(Some(data)),
            None => gpui::assets::Assets.load(path),
        }
    }

    fn list(&self, path: &str) -> gpui::Result<Vec<SharedString>> {
        let mut paths = gpui::assets::Assets.list(path)?;
        paths.extend(ExtraIcons.list(path)?);
        Ok(paths)
    }
}

#[cfg(not(test))]
fn build_app() -> Result<Application> {
    let make_app = || gpui::application().with_assets(AppAssets);

    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    {
        let wayland = std::env::var_os("WAYLAND_DISPLAY").is_some_and(|v| !v.is_empty());
        let x11 = std::env::var_os("DISPLAY").is_some_and(|v| !v.is_empty());
        let force_wayland = std::env::var("TO_GUI_FORCE_WAYLAND")
            .map(|v| {
                let low = v.to_ascii_lowercase();
                matches!(low.as_str(), "1" | "true" | "yes" | "on")
            })
            .unwrap_or(false);

        if wayland && x11 && !force_wayland {
            match std::panic::catch_unwind(make_app) {
                Ok(app) => return Ok(app),
                Err(first_panic) => {
                    let first_error = panic_payload_to_string(first_panic);
                    unsafe {
                        std::env::remove_var("WAYLAND_DISPLAY");
                        std::env::remove_var("WAYLAND_SOCKET");
                    }

                    eprintln!(
                        "to-gui: Wayland initialization failed ({first_error}); retrying with X11 backend"
                    );

                    return std::panic::catch_unwind(make_app).map_err(|second_panic| {
                        anyhow!(
                            "to gui: failed to initialize GUI backends. Wayland error: {first_error}. X11 retry error: {}",
                            panic_payload_to_string(second_panic)
                        )
                    });
                }
            }
        }
    }

    std::panic::catch_unwind(make_app).map_err(|panic_payload| {
        anyhow!(
            "to gui: GUI initialization panicked: {}",
            panic_payload_to_string(panic_payload)
        )
    })
}

/// Launch the GUI.
#[cfg(not(test))]
pub fn run_table_gui(launch: GuiLaunch) -> Result<()> {
    run_table_gui_with(launch, |_, _| {})
}

/// Launch the GUI and call `on_open` once the window exists.
///
/// The hook lets tooling such as `examples/snapshot.rs` drive the window.
#[cfg(not(test))]
pub fn run_table_gui_with(
    launch: GuiLaunch,
    on_open: impl FnOnce(AnyWindowHandle, &mut App) + 'static,
) -> Result<()> {
    let app = build_app()?;

    // Pre-compute the ideal size outside app.run so we can borrow the table.
    let font_size = clamp_font_size(launch.font_size);
    let size = ideal_window_size(&launch.table, launch.autosize, font_size);

    app.run(move |cx| {
        gpui::init(cx);
        configure_theme(cx, font_size);
        bind_keys(cx);
        set_menus(cx);
        cx.activate(true);

        cx.on_window_closed(|cx, _| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();
        cx.on_action(|_: &QuitAction, cx| cx.quit());

        let window_options = WindowOptions {
            window_bounds: Some(WindowBounds::centered(size, cx)),
            window_min_size: Some(gpui::size(px(480.), px(240.))),
            titlebar: Some(TitlebarOptions {
                title: Some("to gui".into()),
                // Center the macOS traffic lights in the (font-scaled) title bar.
                traffic_light_position: Some(point(
                    px(9.),
                    px(((title_bar_height(font_size) - 16.) / 2.).round()),
                )),
                ..TitleBar::title_bar_options()
            }),
            ..TitleBar::window_options()
        };

        let opened = cx.open_window(window_options, move |window, cx| {
            let view = cx.new(|cx| ToGuiView::new(window, cx, launch));
            cx.new(|cx| Root::new(view, window, cx))
        });
        match opened {
            Ok(handle) => on_open(handle.into(), cx),
            Err(err) => eprintln!("to-gui: failed to open window: {err:#}"),
        }
    });
    Ok(())
}

#[cfg(not(test))]
fn configure_theme(cx: &mut App, font_size: f32) {
    // Nushell's color_config and LS_COLORS assume a dark terminal, so stay
    // dark regardless of the OS appearance.
    Theme::change(ThemeMode::Dark, None, cx);
    Theme::update(cx, |theme| {
        theme.font_size = px(font_size);
    });
}

#[cfg(not(test))]
fn bind_keys(cx: &mut App) {
    let ctx = Some(KEY_CONTEXT);
    cx.bind_keys([
        KeyBinding::new("secondary-s", SaveAction, ctx),
        KeyBinding::new("secondary-w", CloseWindowAction, ctx),
        KeyBinding::new("secondary-c", CopyAction, ctx),
        KeyBinding::new("secondary-f", FindAction, ctx),
        KeyBinding::new("secondary-shift-f", ToggleFiltersAction, ctx),
        KeyBinding::new("secondary-[", BackAction, ctx),
        KeyBinding::new("secondary-]", ForwardAction, ctx),
        KeyBinding::new("secondary-=", IncreaseFontSizeAction, ctx),
        KeyBinding::new("secondary-+", IncreaseFontSizeAction, ctx),
        KeyBinding::new("secondary--", DecreaseFontSizeAction, ctx),
        KeyBinding::new("secondary-0", ResetFontSizeAction, ctx),
        KeyBinding::new("secondary-q", QuitAction, None),
    ]);
    #[cfg(target_os = "macos")]
    cx.bind_keys([KeyBinding::new("cmd-m", MinimizeAction, ctx)]);
}

/// Native menus. On macOS the first menu becomes the application menu.
#[cfg(not(test))]
fn set_menus(cx: &mut App) {
    cx.set_menus(vec![
        Menu {
            name: "to gui".into(),
            disabled: false,
            items: vec![
                MenuItem::action("About to gui", AboutAction),
                MenuItem::separator(),
                MenuItem::action("Quit to gui", QuitAction),
            ],
        },
        Menu {
            name: "File".into(),
            disabled: false,
            items: vec![
                MenuItem::action("Save As…", SaveAction),
                MenuItem::separator(),
                MenuItem::action("Close Window", CloseWindowAction),
            ],
        },
        Menu {
            name: "Edit".into(),
            disabled: false,
            items: vec![
                MenuItem::action("Copy", CopyAction),
                MenuItem::separator(),
                MenuItem::action("Find…", FindAction),
                MenuItem::action("Column Filters", ToggleFiltersAction),
                MenuItem::action("Clear Filters", ClearFiltersAction),
            ],
        },
        Menu {
            name: "View".into(),
            disabled: false,
            items: vec![
                MenuItem::action("Increase Font Size", IncreaseFontSizeAction),
                MenuItem::action("Decrease Font Size", DecreaseFontSizeAction),
                MenuItem::action("Reset Font Size", ResetFontSizeAction),
            ],
        },
        Menu {
            name: "Go".into(),
            disabled: false,
            items: vec![
                MenuItem::action("Back", BackAction),
                MenuItem::action("Forward", ForwardAction),
            ],
        },
        Menu {
            name: "Window".into(),
            disabled: false,
            items: vec![
                MenuItem::action("Minimize", MinimizeAction),
                MenuItem::action("Zoom", ZoomWindowAction),
            ],
        },
    ]);
}

#[cfg(test)]
pub fn run_table_gui(_launch: GuiLaunch) -> anyhow::Result<()> {
    Ok(())
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;

    fn make_table(cols: Vec<&str>, rows: Vec<Vec<&str>>) -> TableData {
        use nu_protocol::{Span, Value};
        let raw: Vec<Vec<Value>> = rows
            .iter()
            .map(|r| {
                r.iter()
                    .map(|s| Value::string(s.to_string(), Span::unknown()))
                    .collect()
            })
            .collect();
        TableData {
            columns: cols.into_iter().map(|s| s.to_string()).collect(),
            rows: rows
                .into_iter()
                .map(|r| r.into_iter().map(|s| s.to_string()).collect())
                .collect(),
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
        d.visible_rows
            .sort_by(|a, b| d.all_rows[*a][0].cmp(&d.all_rows[*b][0]));
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
        use nu_protocol::{Span, Value};
        assert_eq!(value_type_key(&Value::int(1, Span::unknown())), "int");
        assert_eq!(value_type_key(&Value::float(1.0, Span::unknown())), "float");
        assert_eq!(
            value_type_key(&Value::string("", Span::unknown())),
            "string"
        );
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
        let wide = make_table((0..100).map(|_| "col").collect(), vec![]);
        let sz = ideal_window_size(&wide, false);
        assert!(sz.width <= px(1600.0));
        assert!(sz.width >= px(400.0));
    }
}
