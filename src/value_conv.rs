//! Conversion utilities for turning Nushell `Value`/`PipelineData` into
//! `TableData` suitable for display.

use nu_protocol::{Config, Value, Span, ast::PathMember, PipelineData};
use nu_plugin::{EngineInterface, EvaluatedCall};
use crate::TableData;
use std::collections::HashMap;

fn format_with_config(v: &Value, config: Option<&Config>) -> String {
    if let Some(cfg) = config {
        v.to_expanded_string(", ", cfg)
    } else {
        v.to_expanded_string(", ", &Config::default())
    }
}

fn closure_to_source_string(engine: &EngineInterface, value: &Value) -> Option<String> {
    let Value::Closure { val: closure, .. } = value else {
        return None;
    };

    let value_span = value.span();
    if value_span != Span::unknown() && !value_span.is_empty() {
        if let Ok(bytes) = engine.get_span_contents(value_span) {
            let s = String::from_utf8_lossy(&bytes).to_string();
            if !s.trim().is_empty() {
                return Some(s);
            }
        }
    }

    let ir = engine.get_block_ir(closure.block_id).ok()?;
    let spans: Vec<Span> = ir
        .spans
        .iter()
        .copied()
        .filter(|span| !span.is_empty() && *span != Span::unknown())
        .collect();

    let mut snippets = Vec::new();
    for span in &spans {
        if let Ok(bytes) = engine.get_span_contents(*span) {
            let s = String::from_utf8_lossy(&bytes).trim().to_string();
            if !s.is_empty() {
                snippets.push(s);
            }
        }
    }
    if !snippets.is_empty() {
        return Some(snippets.join(" "));
    }

    let first = *spans.first()?;
    let mut start = first.start;
    let mut end = first.end;
    for span in spans.into_iter().skip(1) {
        start = start.min(span.start);
        end = end.max(span.end);
    }
    if end <= start {
        return None;
    }

    let source_span = Span::new(start, end);
    let bytes = engine.get_span_contents(source_span).ok()?;
    let source = String::from_utf8_lossy(&bytes).trim().to_string();
    if source.is_empty() {
        None
    } else {
        Some(source)
    }
}

fn highlight_with_nu(engine: &EngineInterface, source: &str, span: Span) -> Option<String> {
    let decl = engine.find_decl("nu-highlight").ok().flatten()?;
    let call = EvaluatedCall::new(span);
    let input = PipelineData::value(Value::string(source.to_string(), span), None);
    let out = engine.call_decl(decl, call, input, true, false).ok()?;
    let value = out.into_value(span).ok()?;
    match value {
        Value::String { val, .. } => Some(val),
        _ => value.coerce_string().ok(),
    }
}

fn closure_to_display_string(engine: &EngineInterface, value: &Value) -> Option<String> {
    let source = closure_to_source_string(engine, value)?;
    highlight_with_nu(engine, &source, value.span()).or(Some(source))
}

fn value_to_json_value_serialize(
    v: &Value,
    engine: Option<&EngineInterface>,
    closure_sources: Option<&HashMap<usize, String>>,
) -> Option<serde_json::Value> {
    match v {
        Value::Bool { val, .. } => Some(serde_json::Value::Bool(*val)),
        Value::Filesize { val, .. } => Some(serde_json::Value::Number(val.get().into())),
        Value::Duration { val, .. } => Some(serde_json::Value::Number((*val).into())),
        Value::Date { val, .. } => Some(serde_json::Value::String(val.to_string())),
        Value::Float { val, .. } => serde_json::Number::from_f64(*val).map(serde_json::Value::Number),
        Value::Int { val, .. } => Some(serde_json::Value::Number((*val).into())),
        Value::Nothing { .. } => Some(serde_json::Value::Null),
        Value::String { val, .. } => Some(serde_json::Value::String(val.clone())),
        Value::Glob { val, .. } => Some(serde_json::Value::String(val.clone())),
        Value::CellPath { val, .. } => Some(serde_json::Value::Array(
            val.members
                .iter()
                .map(|member| match member {
                    PathMember::String { val, .. } => serde_json::Value::String(val.clone()),
                    PathMember::Int { val, .. } => serde_json::Value::Number((*val as i64).into()),
                })
                .collect(),
        )),
        Value::List { vals, .. } => Some(serde_json::Value::Array(
            vals.iter()
                .map(|value| {
                    value_to_json_value_serialize(value, engine, closure_sources)
                        .unwrap_or(serde_json::Value::Null)
                })
                .collect(),
        )),
        Value::Closure { val, .. } => {
            let mut source = engine
                .and_then(|engine| closure_to_display_string(engine, v))
                .unwrap_or_default();
            if source.is_empty() {
                if let Some(cache) = closure_sources {
                    if let Some(cached) = cache.get(&val.block_id.get()) {
                        source = cached.clone();
                    }
                }
            }
            if source.is_empty() {
                source = format!("closure_{}", val.block_id.get());
            }
            Some(serde_json::Value::String(source))
        }
        Value::Range { .. } => Some(serde_json::Value::Null),
        Value::Binary { val, .. } => Some(serde_json::Value::Array(
            val.iter()
                .map(|byte| serde_json::Value::Number((*byte as u64).into()))
                .collect(),
        )),
        Value::Record { val, .. } => {
            let mut map = serde_json::Map::new();
            for (key, value) in val.as_ref().iter() {
                map.insert(
                    key.clone(),
                    value_to_json_value_serialize(value, engine, closure_sources)
                        .unwrap_or(serde_json::Value::Null),
                );
            }
            Some(serde_json::Value::Object(map))
        }
        Value::Custom { val, .. } => {
            let base = val.to_base_value(v.span()).ok()?;
            value_to_json_value_serialize(&base, engine, closure_sources)
        }
        Value::Error { .. } => None,
    }
}

fn value_to_string_with_engine(
    v: &Value,
    engine: Option<&EngineInterface>,
    closure_sources: Option<&HashMap<usize, String>>,
    config: Option<&Config>,
    rfc3339: bool,
) -> String {
    match v {
        Value::Date { .. } => {
            if rfc3339 {
                if let Value::Date { val, .. } = v {
                    return val.to_rfc3339();
                }
            }
            if let Some(cfg) = config {
                v.to_abbreviated_string(cfg)
            } else {
                v.to_abbreviated_string(&Config::default())
            }
        }
        Value::String { .. }
        | Value::Int { .. }
        | Value::Float { .. }
        | Value::Bool { .. }
        | Value::Filesize { .. }
        | Value::Duration { .. }
        | Value::Nothing { .. }
        | Value::Glob { .. }
        | Value::CellPath { .. }
        | Value::Binary { .. }
        | Value::Range { .. } => format_with_config(v, config),
        Value::Record { val: rec, .. } => {
            let pairs: Vec<String> = rec
                .as_ref()
                .iter()
                .map(|(k, v)| {
                    format!(
                        "{}: {}",
                        k,
                        value_to_string_with_engine(v, engine, closure_sources, config, rfc3339)
                    )
                })
                .collect();
            format!("{{{}}}", pairs.join(", "))
        }
        Value::List { vals, .. } => {
            let elems: Vec<String> = vals
                .iter()
                .map(|v| value_to_string_with_engine(v, engine, closure_sources, config, rfc3339))
                .collect();
            format!("[{}]", elems.join(", "))
        }
        Value::Closure { val, .. } => {
            if let Some(engine) = engine {
                if let Some(source) = closure_to_display_string(engine, v) {
                    return source;
                }
            }
            if let Some(cache) = closure_sources {
                if let Some(cached) = cache.get(&val.block_id.get()) {
                    return cached.clone();
                }
            }
            format!("closure_{}", val.block_id.get())
        }
        _ => {
            if let Some(json_value) = value_to_json_value_serialize(v, engine, closure_sources) {
                if let Ok(json) = serde_json::to_string(&json_value) {
                    return json;
                }
            }
            if let Ok(json) = serde_json::to_string(v) {
                json
            } else {
                format_with_config(v, config)
            }
        }
    }
}

/// Convert a `Value` into a human-readable string for display in a table cell.
#[allow(dead_code)]
pub(crate) fn value_to_string(v: &Value) -> String {
    value_to_string_with_engine(v, None, None, None, false)
}

/// Convert a `Value` into a display string with optional engine context.
#[allow(dead_code)]
pub(crate) fn value_to_string_with_plugin_engine(v: &Value, engine: &EngineInterface) -> String {
    let cfg = engine.get_config().ok();
    value_to_string_with_engine(v, Some(engine), None, cfg.as_ref().map(|v| &**v), false)
}

fn collect_closure_sources(value: &Value, engine: &EngineInterface, out: &mut HashMap<usize, String>) {
    match value {
        Value::Closure { val, .. } => {
            if let Some(source) = closure_to_display_string(engine, value) {
                out.entry(val.block_id.get()).or_insert(source);
            }
        }
        Value::List { vals, .. } => {
            for item in vals {
                collect_closure_sources(item, engine, out);
            }
        }
        Value::Record { val, .. } => {
            for (_, item) in val.as_ref().iter() {
                collect_closure_sources(item, engine, out);
            }
        }
        Value::Custom { val, .. } => {
            if let Ok(base) = val.to_base_value(value.span()) {
                collect_closure_sources(&base, engine, out);
            }
        }
        _ => {}
    }
}

pub fn collect_closure_sources_with_plugin_engine(
    values: &[Value],
    engine: &EngineInterface,
) -> HashMap<usize, String> {
    let mut out = HashMap::new();
    for value in values {
        collect_closure_sources(value, engine, &mut out);
    }
    out
}

/// Convert a slice of `Value` into a tabular representation.
///
/// Rules:
/// * If the values are records, columns are the union of all record keys.
/// * A list of records is treated as multiple rows as well.
/// * Scalar values (strings, ints, etc.) are placed in a single
///   column named `"value"`.
/// * Other complex values are stringified via `Debug`.
///
pub fn values_to_table(values: &[Value], transpose: bool) -> TableData {
    values_to_table_with_engine(values, transpose, None, None, None, false)
}

pub fn values_to_table_with_plugin_engine(
    values: &[Value],
    transpose: bool,
    engine: &EngineInterface,
    rfc3339: bool,
) -> TableData {
    let cfg = engine.get_config().ok();
    values_to_table_with_engine(
        values,
        transpose,
        Some(engine),
        None,
        cfg.as_ref().map(|v| &**v),
        rfc3339,
    )
}

pub fn values_to_table_with_closure_sources(
    values: &[Value],
    transpose: bool,
    closure_sources: &HashMap<usize, String>,
) -> TableData {
    values_to_table_with_engine(values, transpose, None, Some(closure_sources), None, false)
}

pub fn values_to_table_with_closure_sources_and_config(
    values: &[Value],
    transpose: bool,
    closure_sources: &HashMap<usize, String>,
    config: &Config,
    rfc3339: bool,
) -> TableData {
    values_to_table_with_engine(
        values,
        transpose,
        None,
        Some(closure_sources),
        Some(config),
        rfc3339,
    )
}

fn values_to_table_with_engine(
    values: &[Value],
    transpose: bool,
    engine: Option<&EngineInterface>,
    closure_sources: Option<&HashMap<usize, String>>,
    config: Option<&Config>,
    rfc3339: bool,
) -> TableData {
    // If requested, and we only have a single record at the top level, convert
    // it to a two‑column key/value table.  This is handy when people pipe a
    // lone record into `to-gui` and expect to see the fields laid out as rows.
    if transpose {
        if values.len() == 1 {
            if let Value::Record { val: rec, .. } = &values[0] {
                let rec = rec.as_ref();
                let cols = vec!["key".to_string(), "value".to_string()];
                let mut rows = Vec::new();
                let mut raw_rows = Vec::new();
                for (k, v) in rec.iter() {
                    rows.push(vec![k.clone(), value_to_string_with_engine(v, engine, closure_sources, config, rfc3339)]);
                    raw_rows.push(vec![Value::string(k.clone(), Span::unknown()), v.clone()]);
                }
                return TableData::new(cols, rows, raw_rows);
            }
        }
    }

    let mut cols_vec: Vec<String> = Vec::new();

    let mut push_unique = |key: &str| {
        if !cols_vec.iter().any(|k| k == key) {
            cols_vec.push(key.to_string());
        }
    };

    // first pass: collect column names from all records
    for val in values {
        if let Value::Record { val: rec, .. } = val {
            for (k, _) in rec.as_ref().iter() {
                push_unique(k);
            }
        } else if let Value::List { vals, .. } = val {
            for inner in vals {
                if let Value::Record { val: rec, .. } = inner {
                    for (k, _) in rec.as_ref().iter() {
                        push_unique(k);
                    }
                }
            }
        }
    }

    if cols_vec.is_empty() {
        cols_vec.push("value".into());
    }

    // We'll build parallel string and raw representations.
    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut raw_rows: Vec<Vec<Value>> = Vec::new();

    for v in values
        .iter()
        .flat_map(|val| match val {
            Value::List { vals, .. } => vals.clone(),
            _ => vec![val.clone()],
        })
    {
        match &v {
            Value::Record { val: rec, .. } => {
                let rec = rec.as_ref();
                let mut row = Vec::with_capacity(cols_vec.len());
                let mut raw_row = Vec::with_capacity(cols_vec.len());
                for key in &cols_vec {
                    if let Some(val) = rec.get(key.as_str()) {
                        row.push(value_to_string_with_engine(val, engine, closure_sources, config, rfc3339));
                        raw_row.push(val.clone());
                    } else {
                        row.push(String::new());
                        raw_row.push(Value::nothing(Span::unknown()));
                    }
                }
                rows.push(row);
                raw_rows.push(raw_row);
            }
            other => {
                rows.push(vec![value_to_string_with_engine(other, engine, closure_sources, config, rfc3339)]);
                raw_rows.push(vec![other.clone()]);
            }
        }
    }

    TableData::new(cols_vec, rows, raw_rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use nu_protocol::{Value, Span, Record, engine::Closure};

    fn make_record(pairs: &[(&str, Value)]) -> Value {
        let mut rec = Record::new();
        for (k, v) in pairs {
            rec.push(k.to_string(), v.clone());
        }
        Value::record(rec, Span::unknown())
    }

    #[test]
    fn scalar_values_produce_value_column() {
        let vals = vec![Value::int(1, Span::unknown()), Value::string("foo", Span::unknown())];
        let table = values_to_table(&vals, false);
        assert_eq!(table.columns, vec!["value".to_string()]);
        assert_eq!(table.rows.len(), 2);
        assert_eq!(table.rows[0][0], "1");
        assert_eq!(table.rows[1][0], "foo");
        assert_eq!(table.raw[1][0], Value::string("foo", Span::unknown()));
    }

    #[test]
    fn date_and_filesize_stringify_nicely() {
        // we don't need a full ISO parser here; just ensure we don't get
        // the debug struct output that was reported by the user.
        let dt = Value::date(chrono::Utc::now().fixed_offset(), Span::unknown());
        let fs = Value::filesize(12345i64, Span::unknown());

        let table = values_to_table(&[dt.clone(), fs.clone()], false);
        assert_eq!(table.columns, vec!["value".to_string()]);
        // both rows should not contain the word "Date {" or "Filesize {";
        // that's the debug output we were previously seeing.
        assert!(!table.rows[0][0].contains("Date {"));
        assert!(!table.rows[0][0].contains("Span"));
        assert!(!table.rows[1][0].contains("Filesize {"));
    }

    #[test]
    fn records_union_columns() {
        let r1 = make_record(&[("a", Value::int(1, Span::unknown())),
                               ("b", Value::string("x", Span::unknown()))]);
        let r2 = make_record(&[("b", Value::string("y", Span::unknown())),
                               ("c", Value::int(2, Span::unknown()))]);
        let table = values_to_table(&[r1, r2], false);
        assert_eq!(table.columns, vec!["a", "b", "c"]);
        assert_eq!(table.rows.len(), 2);
        assert_eq!(table.rows[0], vec!["1".to_string(), "x".to_string(), "".to_string()]);
        assert_eq!(table.rows[1], vec!["".to_string(), "y".to_string(), "2".to_string()]);
    }

    #[test]
    fn list_of_records_behaves_like_rows() {
        let r1 = make_record(&[("a", Value::int(1, Span::unknown()))]);
        let r2 = make_record(&[("a", Value::int(2, Span::unknown()))]);
        let list = Value::list(vec![r1.clone(), r2.clone()], Span::unknown());
        let table = values_to_table(&[list], false);
        assert_eq!(table.columns, vec!["a"]);
        assert_eq!(table.rows.len(), 2);
        // raw values preserved
        assert_eq!(table.raw[0][0], Value::int(1, Span::unknown()));
    }

    #[test]
    fn single_record_transposes_by_default() {
        let rec = make_record(&[("foo", Value::string("bar", Span::unknown()))]);
        let table = values_to_table(&[rec], true);
        // two columns: key and value, one row for the single field
        assert_eq!(table.columns, vec!["key".to_string(), "value".to_string()]);
        assert_eq!(table.rows, vec![vec!["foo".to_string(), "bar".to_string()]]);
    }

    #[test]
    fn transpose_disabled_leaves_record_as_columns() {
        let rec = make_record(&[("foo", Value::string("bar", Span::unknown()))]);
        let table = values_to_table(&[rec], false);
        assert_eq!(table.columns, vec!["foo".to_string()]);
        assert_eq!(table.rows, vec![vec!["bar".to_string()]]);
    }

    #[test]
    fn nested_structures_are_stringified() {
        // A record with a list-valued field: cell should be stringified "[...]"
        let mut inner_rec = Record::new();
        inner_rec.push("x".to_string(), Value::int(5, Span::unknown()));
        let cell_list = Value::list(vec![Value::record(inner_rec, Span::unknown())], Span::unknown());
        let mut row_rec = Record::new();
        row_rec.push("items".to_string(), cell_list);
        let table = values_to_table(&[Value::record(row_rec, Span::unknown())], false);
        // "items" column cell should be the stringified list
        assert!(table.rows[0][0].starts_with("["));
        assert!(table.rows[0][0].contains("x: 5"));
    }

    #[test]
    fn fallback_serializes_unhandled_values() {
        use nu_protocol::ShellError;
        // Create an error value which isn't specially handled in `value_to_string`.
        let err_val = ShellError::GenericError {
            error: "bad".into(),
            msg: "bad".into(),
            span: None,
            help: None,
            inner: vec![],
        };
        let err = Value::error(err_val, Span::unknown());
        let s = value_to_string(&err);
        // should begin with a JSON object rather than the debug variant
        assert!(s.trim_start().starts_with('{'));
        assert!(s.contains("bad"));
    }

    #[test]
    fn closure_without_engine_uses_stable_name() {
        let closure = Closure {
            block_id: nu_protocol::BlockId::new(42),
            captures: vec![],
        };
        let value = Value::closure(closure, Span::unknown());
        let s = value_to_string(&value);
        assert_eq!(s, "closure_42");
        assert!(!s.contains("\"Closure\""));
    }
}
