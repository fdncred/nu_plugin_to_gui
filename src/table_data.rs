use nu_protocol::Value;

/// A simple representation of tabular data used by the GUI layer.
///
/// Columns are stored as a list of keys; each row is a vector of strings
/// with the same length as `columns`.  Empty strings indicate missing
/// values.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct TableData {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<String>>,
    /// original cell values corresponding to `rows` (same shape)
    pub raw: Vec<Vec<Value>>,
}

impl TableData {
    pub fn new(columns: Vec<String>, rows: Vec<Vec<String>>, raw: Vec<Vec<Value>>) -> Self {
        TableData { columns, rows, raw }
    }
}
