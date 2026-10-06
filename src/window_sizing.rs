use crate::TableData;
use crate::settings::DEFAULT_FONT_SIZE;
use gpui::{Pixels, Size, px};

// Column metrics at the default font size; scaled for other sizes.
const CHAR_W: f32 = 8.0;
const CELL_EXTRA_W: f32 = 20.0;
const HEADER_EXTRA_W: f32 = 52.0;
const UNSIZED_COLUMN_W: f32 = 100.0;

/// Table row (and header row) height for a base font size.
pub(crate) fn row_height(font_size: f32) -> f32 {
    (font_size * 2.0).round()
}

/// Title bar height: room for the small search input (1.5rem) plus padding,
/// never below the kit's default height.
pub(crate) fn title_bar_height(font_size: f32) -> f32 {
    (font_size * 1.5 + 10.0).round().max(34.0)
}

/// Column width for the longest cell and the header, in characters.
pub(crate) fn autosize_column_width(cell_chars: usize, header_chars: usize, font_size: f32) -> f32 {
    let scale = font_size / DEFAULT_FONT_SIZE;
    let cell_w = (cell_chars as f32) * CHAR_W + CELL_EXTRA_W;
    let header_w = (header_chars as f32) * CHAR_W + HEADER_EXTRA_W;
    cell_w.max(header_w) * scale
}

/// Column width when autosize is off.
pub(crate) fn unsized_column_width(font_size: f32) -> f32 {
    UNSIZED_COLUMN_W * font_size / DEFAULT_FONT_SIZE
}

pub(crate) fn ideal_window_size(table: &TableData, autosize: bool, font_size: f32) -> Size<Pixels> {
    // Layout: title bar, table header, rows, status bar (see `gui.rs`).
    let scale = font_size / DEFAULT_FONT_SIZE;
    let status_bar_h = 26.0 * scale;
    const EXTRA: f32 = 12.0;
    const MARGIN_W: f32 = 32.0;
    // Room for traffic lights, navigation, search, and title-bar buttons.
    const TITLE_BAR_MIN_W: f32 = 640.0;
    const MAX_W: f32 = 1600.0;
    const MIN_H: f32 = 280.0;
    const MAX_H: f32 = 1024.0;

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
                autosize_column_width(max_len, col_name.len(), font_size)
            } else {
                unsized_column_width(font_size)
            }
        })
        .sum();

    let width = (total_col_w + MARGIN_W).clamp(TITLE_BAR_MIN_W * scale.max(1.0), MAX_W);
    let height = (title_bar_height(font_size)
        + row_height(font_size) * (table.rows.len() as f32 + 1.0)
        + status_bar_h
        + EXTRA)
        .clamp(MIN_H, MAX_H);

    Size {
        width: px(width),
        height: px(height),
    }
}
