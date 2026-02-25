//! GUI code using `gpui` and `gpui-component` to render table data.

use crate::TableData;
use gpui::*;
use gpui_component::{Root, StyledExt};
use gpui_component::table::{Table, TableDelegate, TableState, Column, ColumnSort};
use gpui_component::input::{InputState, InputEvent};
use anyhow::Result;

/// Delegate that wraps `TableData` and implements sorting/filtering.
pub struct NushellTableDelegate {
    all_rows: Vec<Vec<String>>,
    visible_rows: Vec<usize>,
    original_order: Vec<usize>,
    columns: Vec<Column>,
    filter: Option<String>,
}

impl NushellTableDelegate {
    /// `autosize` will adjust the column width based on the longest
    /// string in each column when the delegate is created.
    pub fn new(mut data: TableData, autosize: bool) -> Self {
        let count = data.rows.len();
        let mut columns: Vec<Column> = data.columns.iter().map(|c| Column::new(c.clone(), c.clone()).sortable()).collect();
        if autosize {
            for (col_ix, col) in columns.iter_mut().enumerate() {
                // compute max cell length in this column including header
                let max_len = data
                    .rows
                    .iter()
                    .map(|row| row.get(col_ix).map(|s| s.len()).unwrap_or(0))
                    .chain(std::iter::once(col.name.len()))
                    .max()
                    .unwrap_or(0);
                // approximate pixels: assume ~8px per char plus padding
                let width = ((max_len as f32) * 8.0 + 20.0).into();
                col.width = width;
            }
        }
        let original_order: Vec<usize> = (0..count).collect();
        NushellTableDelegate {
            all_rows: data.rows,
            visible_rows: original_order.clone(),
            original_order,
            columns,
            filter: None,
        }
    }

    fn apply_filter(&mut self) {
        if let Some(ref pat) = self.filter {
            let pat_lower = pat.to_lowercase();
            self.visible_rows = self
                .original_order
                .iter()
                .cloned()
                .filter(|&ix| {
                    self.all_rows[ix]
                        .iter()
                        .any(|cell| cell.to_lowercase().contains(&pat_lower))
                })
                .collect();
        } else {
            self.visible_rows = self.original_order.clone();
        }
    }

    pub fn set_filter(&mut self, pat: Option<String>) {
        self.filter = pat;
        self.apply_filter();
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

    fn render_td(&mut self, row_ix: usize, col_ix: usize, _: &mut Window, _: &mut Context<TableState<Self>>) -> impl IntoElement {
        let real_row = self.visible_rows[row_ix];
        let cell = &self.all_rows[real_row][col_ix];
        cell.clone()
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
}

impl ToGuiView {
    fn new(window: &mut Window, cx: &mut Context<ToGuiView>, table_data: TableData, initial_filter: Option<String>, autosize: bool) -> Self {
        let delegate = NushellTableDelegate::new(table_data, autosize);
        let table_state = cx.new(|cx| TableState::new(delegate, window, cx)
            .col_resizable(true)
            .col_movable(true)
            .sortable(true)
            .col_selectable(true)
            .row_selectable(true));

        let filter_input = cx.new(|cx| InputState::new(window, cx));

        // subscribe to filter changes
        cx.subscribe_in(&filter_input, window, move |view, input, event, _, cx| {
            if let InputEvent::Change = event {
                // `input` is an entity; read the underlying state to grab current text
                let s = input.read(cx).value().to_string();
                view.table_state.update(cx, |table, _| {
                    table.delegate_mut().set_filter(Some(s.clone()));
                });
            }
        }).detach();

        // if we were given an initial filter, apply it now
        if let Some(f) = initial_filter.clone() {
            filter_input.update(cx, |input, cx| input.set_value(f.clone(), window, cx));
            // also inform the delegate explicitly in case the subscription doesn't fire
            table_state.update(cx, |table, _| {
                table.delegate_mut().set_filter(Some(f.clone()));
            });
        }

        ToGuiView { filter_input, table_state }
    }
}

impl Render for ToGuiView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<ToGuiView>) -> impl IntoElement {
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
        let d = NushellTableDelegate::new(table, true);
        // default width if not autosized would be 100px
        assert!(d.columns[0].width > px(100.0));
    }

    #[test]
    fn autosize_can_be_disabled() {
        let table = make_table(vec!["a"], vec![vec!["loooong" ]]);
        let d = NushellTableDelegate::new(table, false);
        assert_eq!(d.columns[0].width, px(100.0));
    }

    #[test]
    fn sorting_changes_order() {
        let table = make_table(vec!["a", "b"], vec![vec!["2", "x"], vec!["1", "y"]]);
        let mut d = NushellTableDelegate::new(table, false);
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
        let mut d = NushellTableDelegate::new(table, false);
        d.set_filter(Some("ba".into()));
        assert_eq!(d.visible_rows, vec![1]);
        d.set_filter(None);
        assert_eq!(d.visible_rows, vec![0, 1]);
    }
}

/// Launch the GUI showing supplied table data.
#[cfg(not(test))]
pub fn run_table_gui(table: TableData, initial_filter: Option<String>, autosize: bool) -> Result<()> {
    let app = Application::new().with_assets(gpui_component_assets::Assets);

    app.run(move |cx| {
        gpui_component::init(cx);

        cx.spawn(async move |cx| {
            cx.open_window(WindowOptions::default(), move |window, cx| {
                let view = cx.new(|cx| ToGuiView::new(window, cx, table.clone(), initial_filter.clone(), autosize));
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
pub fn run_table_gui(_table: TableData, _filter: Option<String>, _autosize: bool) -> anyhow::Result<()> {
    // no-op; just verify call site compiles
    Ok(())
}
