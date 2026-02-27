use nu_plugin::EngineInterface;
use nu_protocol::{ast::PathMember, Config, Value};
use std::collections::HashMap;

use super::closure::closure_to_display_string;

fn format_with_config(v: &Value, config: Option<&Config>) -> String {
    if let Some(cfg) = config {
        v.to_expanded_string(", ", cfg)
    } else {
        v.to_expanded_string(", ", &Config::default())
    }
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

pub(super) fn value_to_string_with_engine(
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

#[allow(dead_code)]
pub(crate) fn value_to_string(v: &Value) -> String {
    value_to_string_with_engine(v, None, None, None, false)
}

#[allow(dead_code)]
pub(crate) fn value_to_string_with_plugin_engine(v: &Value, engine: &EngineInterface) -> String {
    let cfg = engine.get_config().ok();
    value_to_string_with_engine(v, Some(engine), None, cfg.as_ref().map(|v| &**v), false)
}
