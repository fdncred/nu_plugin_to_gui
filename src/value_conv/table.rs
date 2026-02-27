use crate::TableData;
use nu_plugin::EngineInterface;
use nu_protocol::{Config, Span, Value};
use std::collections::HashMap;

use super::stringify::value_to_string_with_engine;

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
