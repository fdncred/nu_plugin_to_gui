//! Render the `to gui` window offscreen and save it as a PNG.
//!
//! Useful for README screenshots and for checking UI changes without a
//! Nushell session. `render_to_image` needs GPUI's test support, which the
//! `snapshot` feature enables:
//!
//! ```text
//! cargo run --example snapshot --features snapshot -- out.png
//! ```
//!
//! Options (after the output path):
//! - `--filter <text>`: start with a global search
//! - `--action <name>`: dispatch an action before capturing, e.g.
//!   `to-gui::toggle-filters`; may repeat
//! - `--nested`: add a record column to show drill-down cells
//! - `--font-size <px>`: base font size, as if set in config.nu

use gpui::*;
use nu_plugin_to_gui::gui_dispatch::GuiLaunch;
use nu_plugin_to_gui::{CellStyle, ColorConfig};
use nu_protocol::{Config, Record, Span, Value};
use std::collections::HashMap;
use std::time::Duration;

fn sample_values(nested: bool) -> Vec<Value> {
    let span = Span::unknown();
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or_default();
    let now = chrono::DateTime::from_timestamp(secs, 0)
        .unwrap_or_default()
        .fixed_offset();
    let entries: &[(&str, &str, i64, i64)] = &[
        (".cargo", "dir", 96, 3),
        (".github", "dir", 96, 40),
        ("Cargo.lock", "file", 182_311, 1),
        ("Cargo.toml", "file", 1_204, 1),
        ("README.md", "file", 4_871, 2),
        ("build.rs", "file", 2_310, 90),
        ("examples", "dir", 96, 0),
        ("rust-toolchain.toml", "file", 1_012, 12),
        ("src", "dir", 480, 0),
        ("target", "dir", 224, 0),
        ("tests", "dir", 96, 30),
    ];
    entries
        .iter()
        .map(|(name, ty, size, days_ago)| {
            let mut rec = Record::new();
            rec.push("name", Value::string(*name, span));
            rec.push("type", Value::string(*ty, span));
            rec.push("size", Value::filesize(*size, span));
            rec.push(
                "modified",
                Value::date(now - chrono::Duration::days(*days_ago), span),
            );
            if nested {
                let mut meta = Record::new();
                meta.push("owner", Value::string("nu", span));
                meta.push("readonly", Value::bool(false, span));
                rec.push("meta", Value::record(meta, span));
            }
            Value::record(rec, span)
        })
        .collect()
}

fn sample_colors() -> ColorConfig {
    let style = |hex: u32, bold: bool| CellStyle {
        fg: Some(rgb(hex)),
        bg: None,
        bold,
    };
    // Deliberately includes colors that are hard to read on a dark
    // background, to show the contrast adjustment: navy file sizes (Nushell's
    // `blue`), black `.lock` names, and black-on-yellow README highlights from
    // Nushell's default LS_COLORS.
    let mut type_styles = HashMap::new();
    type_styles.insert("filesize".to_string(), style(0x000080, false));
    type_styles.insert("date".to_string(), style(0xc678dd, false));
    type_styles.insert("bool".to_string(), style(0x56b6c2, false));
    let mut ls_colors = HashMap::new();
    ls_colors.insert("di".to_string(), style(0x61afef, true));
    ls_colors.insert("*.toml".to_string(), style(0xe5c07b, false));
    ls_colors.insert("*.lock".to_string(), style(0x000000, false));
    ls_colors.insert(
        "*.md".to_string(),
        CellStyle {
            fg: Some(rgb(0x000000)),
            bg: Some(rgb(0xd7d787)),
            bold: false,
        },
    );
    ls_colors.insert("*.rs".to_string(), style(0xe06c75, false));
    ColorConfig {
        type_styles,
        value_styles: HashMap::new(),
        default_style: CellStyle::default(),
        use_ls_colors: true,
        header_style: style(0x98c379, true),
        ls_colors,
    }
}

fn main() {
    let mut args = std::env::args().skip(1);
    let mut out = "snapshot.png".to_string();
    let mut filter = None;
    let mut actions = Vec::new();
    let mut nested = false;
    let mut font_size = nu_plugin_to_gui::settings::DEFAULT_FONT_SIZE;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--filter" => filter = args.next(),
            "--action" => actions.extend(args.next()),
            "--nested" => nested = true,
            "--font-size" => {
                font_size = args
                    .next()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(font_size)
            }
            _ => out = arg,
        }
    }

    let values = sample_values(nested);
    let table = nu_plugin_to_gui::value_conv::values_to_table(&values, true);
    let launch = GuiLaunch {
        table,
        initial_filter: filter,
        autosize: true,
        color_config: sample_colors(),
        save_dir: ".".to_string(),
        closure_sources: HashMap::new(),
        table_config: Config::default(),
        rfc3339: false,
        font_size,
    };

    nu_plugin_to_gui::gui::run_table_gui_with(launch, move |window, cx| {
        cx.spawn(async move |cx| {
            cx.background_executor()
                .timer(Duration::from_millis(600))
                .await;
            for name in &actions {
                let _ = window.update(cx, |_, window: &mut Window, cx| {
                    match cx.build_action(name, None) {
                        Ok(action) => window.dispatch_action(action, cx),
                        Err(err) => eprintln!("snapshot: unknown action {name}: {err}"),
                    }
                });
            }
            cx.background_executor()
                .timer(Duration::from_millis(800))
                .await;
            let _ = window.update(cx, |_, window: &mut Window, _| {
                match window.render_to_image() {
                    Ok(img) => match img.save(&out) {
                        Ok(()) => eprintln!("snapshot: wrote {out}"),
                        Err(err) => eprintln!("snapshot: save failed: {err}"),
                    },
                    Err(err) => eprintln!("snapshot: render failed: {err}"),
                }
            });
            cx.update(|cx| cx.quit());
        })
        .detach();
    })
    .expect("failed to launch GUI");
}
