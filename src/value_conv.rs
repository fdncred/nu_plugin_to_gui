//! Conversion utilities for turning Nushell `Value`/`PipelineData` into
//! `TableData` suitable for display.

use nu_protocol::Value;
use crate::TableData;
use std::collections::BTreeSet;

/// Convert a slice of `Value` into a tabular representation.
///
/// Rules:
/// * If the values are records, columns are the union of all record keys.
/// * A list of records is treated as multiple rows as well.
/// * Scalar values (strings, ints, etc.) are placed in a single
///   column named `"value"`.
/// * Other complex values are stringified via `Debug`.
///
pub fn values_to_table(values: &[Value]) -> TableData {
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

    fn value_to_string(v: &Value) -> String {
        use Value::*;
        match v {
            String { val, .. } => val.clone(),
            Int { val, .. } => val.to_string(),
            Float { val, .. } => val.to_string(),
            Bool { val, .. } => val.to_string(),
            Date { val, .. } => val.to_string(),
            Filesize { val, .. } => val.to_string(),
            // For any other values we still fall back to `Debug`.  This
            // isn't ideal for every possible datatype, but handling the
            // common ones (string/int/float/bool/date/filesize) is enough
            // to avoid the ugly output the user reported.
            _ => format!("{:?}", v),
        }
    }

    let rows: Vec<Vec<String>> = values
        .iter()
        .flat_map(|val| match val {
            Value::List { vals, .. } => vals.clone(),
            _ => vec![val.clone()],
        })
        .map(|v| match &v {
            Value::Record { val: rec, .. } => {
                let rec = rec.as_ref();
                let mut row = Vec::with_capacity(cols_vec.len());
                for key in &cols_vec {
                    if let Some(val) = rec.get(key.as_str()) {
                        row.push(value_to_string(val));
                    } else {
                        row.push(String::new());
                    }
                }
                row
            }
            other => vec![value_to_string(other)],
        })
        .collect();

    TableData::new(cols_vec, rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use nu_protocol::{Value, Span};

    fn make_record(pairs: &[(&str, Value)]) -> Value {
        let cols: Vec<String> = pairs.iter().map(|(k, _)| k.to_string()).collect();
        let vals: Vec<Value> = pairs.iter().map(|(_, v)| v.clone()).collect();
        Value::record(cols, vals, Span::unknown())
    }

    #[test]
    fn scalar_values_produce_value_column() {
        let vals = vec![Value::int(1, Span::unknown()), Value::string("foo", Span::unknown())];
        let table = values_to_table(&vals);
        assert_eq!(table.columns, vec!["value".to_string()]);
        assert_eq!(table.rows.len(), 2);
        assert_eq!(table.rows[0][0], "1");
        assert_eq!(table.rows[1][0], "foo");
    }

    #[test]
    fn date_and_filesize_stringify_nicely() {
        // we don't need a full ISO parser here; just ensure we don't get
        // the debug struct output that was reported by the user.
        let dt = Value::Date {
            val: chrono::Utc::now(),
            span: Span::unknown(),
        };
        let fs = Value::Filesize { val: 12345, span: Span::unknown() };
        
        let table = values_to_table(&[dt.clone(), fs.clone()]);
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
        let table = values_to_table(&[r1, r2]);
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
        let table = values_to_table(&[list]);
        assert_eq!(table.columns, vec!["a"]);
        assert_eq!(table.rows.len(), 2);
    }
}
