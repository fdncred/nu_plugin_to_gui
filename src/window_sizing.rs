use crate::TableData;
use gpui::{Pixels, Size, px};

pub(crate) fn ideal_window_size(table: &TableData, autosize: bool) -> Size<Pixels> {
    const ROW_H: f32 = 36.0;
    const HEADER_H: f32 = 70.0;
    const FILTER_H: f32 = 42.0;
    const TOOLBAR_H: f32 = 42.0;
    const MENU_H: f32 = 44.0;
    const EXTRA: f32 = 24.0;
    const MARGIN_W: f32 = 32.0;
    const MIN_W: f32 = 400.0;
    const MENUBAR_MIN_W: f32 = 640.0;
    const MAX_W: f32 = 1600.0;
    const MIN_H: f32 = 280.0;
    const MAX_H: f32 = 1024.0;
    const CHAR_W: f32 = 8.0;
    const CELL_EXTRA_W: f32 = 20.0;
    const HEADER_EXTRA_W: f32 = 52.0;

    let total_col_w: f32 = table
        .columns
        .iter()
        .enumerate()
        .map(|(col_ix, col_name)| {
            if autosize {
                let max_len = table
                    .rows
                    .iter()
                    .map(|row| row.get(col_ix).map(|s| s.len()).unwrap_or(0))
                    .max()
                    .unwrap_or(0);
                let cell_w = (max_len as f32) * CHAR_W + CELL_EXTRA_W;
                let header_w = (col_name.len() as f32) * CHAR_W + HEADER_EXTRA_W;
                cell_w.max(header_w)
            } else {
                100.0
            }
        })
        .sum();

    let width = (total_col_w + MARGIN_W).clamp(MIN_W.max(MENUBAR_MIN_W), MAX_W);
    let height =
        (MENU_H + TOOLBAR_H + FILTER_H + HEADER_H + (table.rows.len() as f32) * ROW_H + EXTRA)
            .clamp(MIN_H, MAX_H);

    Size {
        width: px(width),
        height: px(height),
    }
}
