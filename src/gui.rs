//! GUI code using `gpui` and `gpui-component` to render table data.

use crate::TableData;
use nu_protocol::Value;
use gpui::*;
use gpui::Fill;
use gpui_component::{Root, StyledExt};
use std::rc::Rc;
use std::cell::RefCell;
use gpui_component::table::{Table, TableDelegate, TableState, TableEvent, Column, ColumnSort};
use gpui_component::input::{InputState, InputEvent};
use anyhow::Result;

// json value type is needed when implementing the Action trait manually;
// alias to avoid collision with `nu_protocol::Value` imported above.
use serde_json::Value as JsonValue;

// ---------------------------------------------------------------------------
// menu and action helpers
// ---------------------------------------------------------------------------

/// Action invoked by the File → Save menu item.  We implement the trait
/// manually so that the dependency footprint stays small; this struct carries
/// no data since the behaviour is driven by the closure that registers the
/// handler.
#[derive(Clone, PartialEq)]
struct SaveAction;

impl gpui::Action for SaveAction {
    fn boxed_clone(&self) -> Box<dyn gpui::Action> {
        Box::new(self.clone())
    }

    fn partial_eq(&self, action: &dyn gpui::Action) -> bool {
        action.as_any().downcast_ref::<SaveAction>().is_some()
    }

    fn name(&self) -> &'static str {
        "to-gui::save"
    }

    fn name_for_type() -> &'static str {
        "to-gui::save"
    }

    fn build(_value: JsonValue) -> gpui::Result<Box<dyn gpui::Action>>
    where
        Self: Sized,
    {
        Ok(Box::new(SaveAction))
    }
}

// register the action so `MenuItem::action` can construct it by name
gpui::register_action!(SaveAction);

/// Delegate that wraps `TableData` and implements sorting/filtering.
pub struct NushellTableDelegate {
    all_rows: Vec<Vec<String>>,
    raw_rows: Vec<Vec<Value>>,
    visible_rows: Vec<usize>,
    original_order: Vec<usize>,
    columns: Vec<Column>,
    filter: Option<String>,
    /// per-column patterns
    column_filters: Vec<Option<String>>,
    /// UI elements used for per-column filtering (one `InputState` per column)
    column_filter_inputs: Vec<Entity<InputState>>,
    /// Used by the view to remember which cell was clicked most recently so the
    /// double‑click handler can know which column to open.
    cell_click: Rc<RefCell<Option<(usize, usize)>>>,
    fg_color: Option<Rgba>,
    bg_color: Option<Fill>, // stored as a Fill for easy bg application
}

impl NushellTableDelegate {
    /// `autosize` will adjust the column width based on the longest
    /// string in each column when the delegate is created.  The additional
    /// parameters allow callers to provide the per-column filter input entities
    /// (created by the view) and a shared `cell_click` handle used to record the
    /// last clicked cell.
    pub fn new(
        data: TableData,
        autosize: bool,
        fg_color: Option<Rgba>,
        bg_color: Option<Fill>,
        column_filter_inputs: Vec<Entity<InputState>>,
        cell_click: Rc<RefCell<Option<(usize, usize)>>>,
    ) -> Self {
        let count = data.rows.len();
        let mut columns: Vec<Column> = data.columns.iter().map(|c| Column::new(c.clone(), c.clone()).sortable()).collect();
        if autosize {
            for (col_ix, col) in columns.iter_mut().enumerate() {
                let max_len = data
                    .rows
                    .iter()
                    .map(|row| row.get(col_ix).map(|s| s.len()).unwrap_or(0))
                    .chain(std::iter::once(col.name.len()))
                    .max()
                    .unwrap_or(0);
                let width = ((max_len as f32) * 8.0 + 20.0).into();
                col.width = width;
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
            column_filters: vec![None; count],
            column_filter_inputs,
            cell_click,
            fg_color,
            bg_color,
        }
    }

    fn apply_filter(&mut self) {
        let global = self.filter.as_ref().map(|s| s.to_lowercase());
        fn matches(cell: &str, pat: &str) -> bool {
            let pat = pat.to_lowercase();
            if let Some(rest) = pat.strip_prefix("is:") {
                cell.eq_ignore_ascii_case(rest)
            } else if let Some(rest) = pat.strip_prefix("contains:") {
                cell.to_lowercase().contains(rest)
            } else if let Some(rest) = pat.strip_prefix("starts-with:") {
                cell.to_lowercase().starts_with(rest)
            } else if let Some(rest) = pat.strip_prefix("ends-with:") {
                cell.to_lowercase().ends_with(rest)
            } else {
                cell.to_lowercase().contains(&pat)
            }
        }

        self.visible_rows = self
            .original_order
            .iter()
            .cloned()
            .filter(|&ix| {
                let row = &self.all_rows[ix];
                // global match any if set
                if let Some(ref pat) = global {
                    if !row.iter().any(|cell| cell.to_lowercase().contains(pat)) {
                        return false;
                    }
                }
                // per-column filters
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
}

impl TableDelegate for NushellTableDelegate {
    fn columns_count(&self, _: &App) -> usize {
        self.columns.len()
    }

    fn rows_count(&self, _: &App) -> usize {
        self.visible_rows.len()
    }

    fn column(&self, col_ix: usize, _: &App) -> &Column {
        &self.columns[col_ix]
    }

    fn render_th(&mut self, col_ix: usize, _window: &mut Window, _cx: &mut Context<TableState<Self>>) -> impl gpui::IntoElement {
        // show column name above a small filter input box
        let mut hdr = gpui::div().v_flex().gap_1().child(self.columns[col_ix].name.clone());
        if let Some(input) = self.column_filter_inputs.get(col_ix) {
            hdr = hdr.child(input.clone());
        }
        hdr
    }

    fn render_td(&mut self, row_ix: usize, col_ix: usize, _window: &mut Window, cx: &mut Context<TableState<Self>>) -> impl IntoElement {
        let real_row = self.visible_rows[row_ix];
        let cell = &self.all_rows[real_row][col_ix];
        let raw = &self.raw_rows[real_row][col_ix];
        let text = cell.clone();
        let mut div = gpui::div().size_full().child(text);
        // apply optional colors from configuration
        if let Some(bg) = &self.bg_color {
            div = div.bg(bg.clone());
        }
        if let Some(fg) = self.fg_color {
            div = div.text_color(fg);
        }
        // record the last clicked cell so the double-click handler knows which
        // column to open.  Only attach the listener for non‑scalar values since
        // that is the only case where we want to open a nested window.
        match raw {
            Value::Record { .. } | Value::List { .. } => {
                let click_ref = self.cell_click.clone();
                div = div.on_mouse_down(MouseButton::Left, cx.listener(move |_el, _e, _window, _cx| {
                    *click_ref.borrow_mut() = Some((row_ix, col_ix));
                }));
            }
            _ => {}
        }
        div
    }

    fn perform_sort(&mut self, col_ix: usize, sort: ColumnSort, _: &mut Window, _: &mut Context<TableState<Self>>) {
        let key = col_ix;
        match sort {
            ColumnSort::Ascending => self.visible_rows.sort_by(|a, b| self.all_rows[*a][key].cmp(&self.all_rows[*b][key])),
            ColumnSort::Descending => self.visible_rows.sort_by(|a, b| self.all_rows[*b][key].cmp(&self.all_rows[*a][key])),
            ColumnSort::Default => {
                self.visible_rows = self.original_order.clone();
            }
        }
    }
}

/// View that contains optional search box and the table.
struct ToGuiView {
    filter_input: Entity<InputState>,
    table_state: Entity<TableState<NushellTableDelegate>>,
    /// original table data so we can look up raw values for nested views
    _table_data: TableData,
    /// per-column filter input widgets (same ones that live in the delegate)
    _column_filter_inputs: Vec<Entity<InputState>>,
    /// tracks which cell was most recently clicked
    _cell_click: Rc<RefCell<Option<(usize, usize)>>>,
}

impl ToGuiView {
    fn new(
        window: &mut Window,
        cx: &mut Context<ToGuiView>,
        table_data: TableData,
        initial_filter: Option<String>,
        autosize: bool,
        fg: Option<Rgba>,
        bg: Option<Rgba>,
    ) -> Self {
        // create helper references that both view and delegate will share
        let cell_click: Rc<RefCell<Option<(usize, usize)>>> = Rc::new(RefCell::new(None));

        // input widgets for each column's filter box
        let mut column_filter_inputs: Vec<Entity<InputState>> = Vec::new();
        for _ in 0..table_data.columns.len() {
            let inp = cx.new(|cx| InputState::new(window, cx));
            column_filter_inputs.push(inp);
        }

        let delegate = NushellTableDelegate::new(
            table_data.clone(),
            autosize,
            fg,
            bg.map(|c| c.into()), // map rgba->fill
            column_filter_inputs.clone(),
            cell_click.clone(),
        );

        let table_state = cx.new(|cx| {
            TableState::new(delegate, window, cx)
                .col_resizable(true)
                .col_movable(true)
                .sortable(true)
                .col_selectable(true)
                .row_selectable(true)
        });

        let filter_input = cx.new(|cx| InputState::new(window, cx));

        // subscribe to global filter changes
        cx.subscribe_in(&filter_input, window, move |view, input, event, _, cx| {
            if let InputEvent::Change = event {
                let s = input.read(cx).value().to_string();
                view.table_state.update(cx, |table, _| {
                    table.delegate_mut().set_filter(Some(s.clone()));
                });
            }
        })
        .detach();

        // subscribe to each column filter input and drive the delegate
        for (col_ix, inp) in column_filter_inputs.iter().enumerate() {
            let table_state_clone = table_state.clone();
            cx.subscribe_in(inp, window, move |_view, input, event, _, cx| {
                if let InputEvent::Change = event {
                    let pat = input.read(cx).value().to_string();
                    table_state_clone.update(cx, |table, _| {
                        table.delegate_mut().set_column_filter(col_ix, Some(pat.clone()));
                    });
                }
            })
            .detach();
        }

        // if we were given an initial filter, apply it now (global only)
        if let Some(f) = initial_filter.clone() {
            filter_input.update(cx, |input, cx| input.set_value(f.clone(), window, cx));
            table_state.update(cx, |table, _| {
                table.delegate_mut().set_filter(Some(f.clone()));
            });
        }

        // listener for double-click row events; open nested window if appropriate
        let td_clone = table_data.clone();
        let autosize_copy = autosize;
        let fg_copy = fg;
        let bg_copy = bg;
        let cell_click_for_sub = cell_click.clone();
        cx.subscribe_in(&table_state, window, move |_view, _state, event, _, cx| {
            if let TableEvent::DoubleClickedRow(row_ix) = event {
                if let Some((r, c)) = *cell_click_for_sub.borrow() {
                    if r == *row_ix {
                        if let Some(raw) = td_clone.raw.get(r).and_then(|row| row.get(c)) {
                            match raw {
                                Value::Record { .. } | Value::List { .. } => {
                                    let nested = crate::value_conv::values_to_table(&[raw.clone()], true);
                                    // open a new window directly in the subscription context
                                    let _ = cx.open_window(WindowOptions::default(), move |window, cx| {
                                        let view = cx.new(|cx| ToGuiView::new(
                                            window,
                                            cx,
                                            nested.clone(),
                                            None,
                                            autosize_copy,
                                            fg_copy,
                                            bg_copy,
                                        ));
                                        cx.new(|cx| Root::new(view, window, cx))
                                    });
                                }
                                _ => {}
                            }
                        }
                    }
                }
            }
        })
        .detach();

        ToGuiView {
            filter_input,
            table_state,
            _table_data: table_data,
            _column_filter_inputs: column_filter_inputs,
            _cell_click: cell_click,
        }
    }
}

impl Render for ToGuiView {
    fn render(&mut self, _: &mut Window, _cx: &mut Context<ToGuiView>) -> impl IntoElement {
        gpui::div()
            .v_flex()
            .gap_2()
            .size_full()
            .child(self.filter_input.clone())
            .child(Table::new(&self.table_state)
                .stripe(true)
                .bordered(true)
                .scrollbar_visible(true, true))
    }
}

// tests for delegate behaviour
#[cfg(test)]
mod tests {
    use super::*;

    fn make_table(cols: Vec<&str>, rows: Vec<Vec<&str>>) -> TableData {
        TableData {
            columns: cols.into_iter().map(|s| s.to_string()).collect(),
            rows: rows
                .into_iter()
                .map(|r| r.into_iter().map(|s| s.to_string()).collect())
                .collect(),
        }
    }

    #[test]
    fn autosize_columns_wider_when_requested() {
        let table = make_table(vec!["a"], vec![vec!["loooong" ]]);
        let dummy_inputs = Vec::new();
        let dummy_click = Rc::new(RefCell::new(None));
        let d = NushellTableDelegate::new(table, true, None, None, dummy_inputs, dummy_click);
        // default width if not autosized would be 100px
        assert!(d.columns[0].width > px(100.0));
    }

    #[test]
    fn autosize_can_be_disabled() {
        let table = make_table(vec!["a"], vec![vec!["loooong" ]]);
        let dummy_inputs = Vec::new();
        let dummy_click = Rc::new(RefCell::new(None));
        let d = NushellTableDelegate::new(table, false, None, None, dummy_inputs, dummy_click);
        assert_eq!(d.columns[0].width, px(100.0));
    }

    #[test]
    fn column_filter_hides_rows() {
        let table = make_table(vec!["a", "b"], vec![vec!["foo","x"], vec!["bar","y"]]);
        let dummy_inputs = Vec::new();
        let dummy_click = Rc::new(RefCell::new(None));
        let mut d = NushellTableDelegate::new(table, false, None, None, dummy_inputs, dummy_click);
        d.set_column_filter(0, Some("ba".into()));
        assert_eq!(d.visible_rows, vec![1]);
        d.set_column_filter(1, Some("x".into()));
        // previous filter remains, so no rows
        assert!(d.visible_rows.is_empty());
        d.set_column_filter(0, None);
        assert_eq!(d.visible_rows, vec![0]);
    }

    // duplicate test removed; behaviour already verified above

    #[test]
    fn sorting_changes_order() {
        let table = make_table(vec!["a", "b"], vec![vec!["2", "x"], vec!["1", "y"]]);
        let mut d = NushellTableDelegate::new(table, false, None, None);
        // initial order
        assert_eq!(d.visible_rows, vec![0, 1]);
        // windows and contexts aren't used by our implementation, so we
        // create uninitialized values just to satisfy the signature.
        let mut win: Window = unsafe { std::mem::MaybeUninit::zeroed().assume_init() };
        let mut ctx: Context<NushellTableDelegate> = unsafe { std::mem::MaybeUninit::zeroed().assume_init() };
        d.perform_sort(0, ColumnSort::Ascending, &mut win, &mut ctx);
        assert_eq!(d.visible_rows, vec![1, 0]);
        d.perform_sort(0, ColumnSort::Default, &mut win, &mut ctx);
        assert_eq!(d.visible_rows, vec![0, 1]);
    }

    #[test]
    fn filtering_hides_rows() {
        let table = make_table(vec!["a"], vec![vec!["foo"], vec!["bar"]]);
        let dummy_inputs = Vec::new();
        let dummy_click = Rc::new(RefCell::new(None));
        let mut d = NushellTableDelegate::new(table, false, None, None, dummy_inputs, dummy_click);
        d.set_filter(Some("ba".into()));
        assert_eq!(d.visible_rows, vec![1]);
        d.set_filter(None);
        assert_eq!(d.visible_rows, vec![0, 1]);
    }

    #[test]
    fn column_filter_special_terms() {
        let table = make_table(vec!["a"], vec![vec!["abc"], vec!["ab"], vec!["xbc"]]);
        let dummy_inputs = Vec::new();
        let dummy_click = Rc::new(RefCell::new(None));
        let mut d = NushellTableDelegate::new(table, false, None, None, dummy_inputs, dummy_click);
        d.set_column_filter(0, Some("is:ab".into()));
        assert_eq!(d.visible_rows, vec![1]); // exact match only
        d.set_column_filter(0, Some("starts-with:ab".into()));
        assert_eq!(d.visible_rows, vec![0, 1]);
        d.set_column_filter(0, Some("ends-with:bc".into()));
        assert_eq!(d.visible_rows, vec![0, 2]);
        d.set_column_filter(0, Some("contains:bc".into()));
        assert_eq!(d.visible_rows, vec![0, 2]);
    }
}

// additional tests for newly added menu/ action machinery
#[cfg(test)]
mod menu_tests {
    use super::*;

    #[test]
    fn save_action_has_expected_name() {
        let a = SaveAction;
        assert_eq!(a.name(), "to-gui::save");
    }

    #[test]
    fn can_construct_menu_with_save_item() {
        let _menu = Menu {
            name: "File".into(),
            items: vec![MenuItem::action("Save", SaveAction)],
        };
    }

    #[test]
    fn run_table_gui_stub_accepts_colors() {
        let dummy = TableData::new(vec![], vec![], vec![]);
        let _ = run_table_gui(dummy, None, false, None, None);
    }
}

/// Launch the GUI showing supplied table data.
#[cfg(not(test))]
pub fn run_table_gui(
    table: TableData,
    initial_filter: Option<String>,
    autosize: bool,
    fg: Option<Rgba>,
    bg: Option<Rgba>,
) -> Result<()> {
    // we will need the table later for the save action handler
    let table_for_save = table.clone();

    let app = Application::new().with_assets(gpui_component_assets::Assets);

    app.run(move |cx| {
        gpui_component::init(cx);
        // bring menus to the front; without this the bar may be hidden on some
        // platforms.
        cx.activate(true);

        // build a basic menu bar; only File/Save is wired up for now
        cx.set_menus(vec![
            Menu {
                name: "File".into(),
                items: vec![
                    MenuItem::action("Save", SaveAction),
                    MenuItem::separator(),
                    // other file items could go here
                ],
            },
            Menu { name: "Edit".into(), items: vec![] },
            Menu { name: "View".into(), items: vec![] },
            Menu { name: "Options".into(), items: vec![] },
            Menu { name: "Window".into(), items: vec![] },
            Menu { name: "Help".into(), items: vec![] },
        ]);

        // global action listener for the save command; we clone the table so
        // the callback owns its own copy.
        let table_in_callback = table_for_save.clone();
        cx.on_action::<SaveAction>(move |_action, app| {
            let table_copy = table_in_callback.clone();
            let rx = app.prompt_for_new_path(&std::path::Path::new("."), Some("table.json"));
            let _ = app.spawn(async move |_app| {
                // rx.await returns Result<Result<Option<PathBuf>, Error>, Canceled>
                if let Ok(Ok(Some(path))) = rx.await {
                    if let Ok(json) = serde_json::to_string(&table_copy) {
                        let _ = std::fs::write(&path, json);
                    }
                }
            });
        });

        cx.spawn(async move |cx| {
            cx.open_window(WindowOptions::default(), move |window, cx| {
                let view = cx.new(|cx| ToGuiView::new(window, cx, table.clone(), initial_filter.clone(), autosize, fg, bg));
                cx.new(|cx| Root::new(view, window, cx))
            })?;

            Ok::<_, anyhow::Error>(())
        })
        .detach();
    });

    Ok(())
}

// during tests we don't actually bring up a window
#[cfg(test)]
pub fn run_table_gui(
    _table: TableData,
    _filter: Option<String>,
    _autosize: bool,
    _fg: Option<Rgba>,
    _bg: Option<Rgba>,
) -> anyhow::Result<()> {
    // no-op; just verify call site compiles
    Ok(())
}
