//! Conversion utilities for turning Nushell `Value`/`PipelineData` into
//! `TableData` suitable for display.

use nu_protocol::{Value, Span};
use crate::TableData;
use std::collections::BTreeSet;

/// Convert a `Value` into a human-readable string for display in a table cell.
pub(crate) fn value_to_string(v: &Value) -> String {
    match v {
        Value::String { val, .. } => val.clone(),
        Value::Int { val, .. } => val.to_string(),
        Value::Float { val, .. } => val.to_string(),
        Value::Bool { val, .. } => val.to_string(),
        Value::Date { val, .. } => val.to_string(),
        Value::Filesize { val, .. } => val.to_string(),
        Value::Record { val: rec, .. } => {
            let pairs: Vec<String> = rec
                .as_ref()
                .iter()
                .map(|(k, v)| format!("{}: {}", k, value_to_string(v)))
                .collect();
            format!("{{{}}}", pairs.join(", "))
        }
        Value::List { vals, .. } => {
            let elems: Vec<String> = vals.iter().map(value_to_string).collect();
            format!("[{}]", elems.join(", "))
        }
        _ => {
            // some values (closures, errors, etc.) look nicer when
            // serialized the same way `to json --serialize` does.
            if let Ok(json) = serde_json::to_string(v) {
                json
            } else {
                format!("{:?}", v)
            }
        },
    }
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
                    rows.push(vec![k.clone(), value_to_string(v)]);
                    raw_rows.push(vec![Value::string(k.clone(), Span::unknown()), v.clone()]);
                }
                return TableData::new(cols, rows, raw_rows);
            }
        }
    }

    let mut columns = BTreeSet::new();

    // first pass: collect column names from all records
    for val in values {
        if let Value::Record { val: rec, .. } = val {
            for (k, _) in rec.as_ref().iter() {
                columns.insert(k.clone());
            }
        } else if let Value::List { vals, .. } = val {
            for inner in vals {
                if let Value::Record { val: rec, .. } = inner {
                    for (k, _) in rec.as_ref().iter() {
                        columns.insert(k.clone());
                    }
                }
            }
        }
    }

    let mut cols_vec: Vec<String> = columns.into_iter().collect();

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
                        row.push(value_to_string(val));
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
                rows.push(vec![value_to_string(other)]);
                raw_rows.push(vec![other.clone()]);
            }
        }
    }

    TableData::new(cols_vec, rows, raw_rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use nu_protocol::{Value, Span, Record};

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
}
