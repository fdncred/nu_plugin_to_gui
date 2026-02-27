use nu_plugin::{EngineInterface, EvaluatedCall};
use nu_protocol::{PipelineData, Span, Value};
use std::collections::HashMap;

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

pub(super) fn closure_to_display_string(engine: &EngineInterface, value: &Value) -> Option<String> {
    let source = closure_to_source_string(engine, value)?;
    highlight_with_nu(engine, &source, value.span()).or(Some(source))
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
