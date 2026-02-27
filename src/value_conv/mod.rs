//! Conversion utilities for turning Nushell `Value`/`PipelineData` into
//! `TableData` suitable for display.

mod closure;
mod stringify;
mod table;

pub use closure::collect_closure_sources_with_plugin_engine;
pub use table::{
    values_to_table,
    values_to_table_with_closure_sources,
    values_to_table_with_closure_sources_and_config,
    values_to_table_with_plugin_engine,
};

#[cfg(test)]
mod tests {
    use super::*;
    use super::stringify::value_to_string;
    use nu_protocol::{engine::Closure, Record, Span, Value};

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
        let dt = Value::date(chrono::Utc::now().fixed_offset(), Span::unknown());
        let fs = Value::filesize(12345i64, Span::unknown());

        let table = values_to_table(&[dt.clone(), fs.clone()], false);
        assert_eq!(table.columns, vec!["value".to_string()]);
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
        assert_eq!(table.raw[0][0], Value::int(1, Span::unknown()));
    }

    #[test]
    fn single_record_transposes_by_default() {
        let rec = make_record(&[("foo", Value::string("bar", Span::unknown()))]);
        let table = values_to_table(&[rec], true);
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
        let mut inner_rec = Record::new();
        inner_rec.push("x".to_string(), Value::int(5, Span::unknown()));
        let cell_list = Value::list(vec![Value::record(inner_rec, Span::unknown())], Span::unknown());
        let mut row_rec = Record::new();
        row_rec.push("items".to_string(), cell_list);
        let table = values_to_table(&[Value::record(row_rec, Span::unknown())], false);
        assert!(table.rows[0][0].starts_with("["));
        assert!(table.rows[0][0].contains("x: 5"));
    }

    #[test]
    fn fallback_serializes_unhandled_values() {
        use nu_protocol::ShellError;

        let err_val = ShellError::GenericError {
            error: "bad".into(),
            msg: "bad".into(),
            span: None,
            help: None,
            inner: vec![],
        };
        let err = Value::error(err_val, Span::unknown());
        let s = value_to_string(&err);
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
