use std::collections::BTreeMap;

use codex_protocol::protocol::W3cTraceContext;
use tracing::Span;

const MAX_TRACESTATE_MEMBERS: usize = 32;
const MAX_TRACESTATE_KEY_LEN: usize = 256;
const MAX_TRACESTATE_VALUE_LEN: usize = 256;

/// Placeholder for a distributed trace context.
///
/// Traces are never exported, so a context carries no trace or span identifiers.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Context;

pub fn current_span_w3c_trace_context() -> Option<W3cTraceContext> {
    span_w3c_trace_context(&Span::current())
}

pub fn span_w3c_trace_context(_span: &Span) -> Option<W3cTraceContext> {
    None
}

/// Serializes a trace context into a W3C trace carrier.
pub fn w3c_trace_context_from_context(_context: &Context) -> Option<W3cTraceContext> {
    None
}

/// Injects the W3C trace context for `span` into HTTP headers.
///
/// Returns `false` and leaves `headers` untouched because spans carry no trace context.
pub fn inject_span_w3c_trace_headers(_span: &Span, _headers: &mut http::HeaderMap) -> bool {
    false
}

pub fn current_span_trace_id() -> Option<String> {
    None
}

pub fn context_from_w3c_trace_context(_trace: &W3cTraceContext) -> Option<Context> {
    None
}

pub fn set_parent_from_w3c_trace_context(_span: &Span, _trace: &W3cTraceContext) -> bool {
    false
}

pub fn set_parent_from_context(_span: &Span, _context: Context) {}

pub fn traceparent_context_from_env() -> Option<Context> {
    None
}

/// Validates configured tracestate members before they are propagated in W3C trace context.
pub fn validate_tracestate_entries(
    entries: &BTreeMap<String, BTreeMap<String, String>>,
) -> Result<(), Box<dyn std::error::Error>> {
    // Reject malformed entries so propagated trace context remains acceptable
    // to W3C Trace Context extractors. Configured member fields are joined
    // into header values here and need stricter validation.
    if entries.len() > MAX_TRACESTATE_MEMBERS {
        return Err(invalid_tracestate_config(format!(
            "invalid configured tracestate: more than {MAX_TRACESTATE_MEMBERS} members"
        )));
    }
    for (key, fields) in entries {
        validate_tracestate_member(key, fields)?;
    }
    Ok(())
}

/// Validates one configured tracestate member and its encoded field value.
pub fn validate_tracestate_member(
    member_key: &str,
    fields: &BTreeMap<String, String>,
) -> Result<(), Box<dyn std::error::Error>> {
    let (key, value) = encode_tracestate_member_fields(member_key, fields)?;
    if !is_valid_tracestate_key(&key) {
        return Err(invalid_tracestate_config(format!(
            "invalid configured tracestate: invalid key {key}"
        )));
    }
    if value.len() > MAX_TRACESTATE_VALUE_LEN {
        return Err(invalid_tracestate_config(format!(
            "invalid configured tracestate: value for {key} is too long"
        )));
    }
    Ok(())
}

fn is_valid_tracestate_key(key: &str) -> bool {
    if key.is_empty() || key.len() > MAX_TRACESTATE_KEY_LEN {
        return false;
    }
    let is_tail_byte =
        |byte: u8| matches!(byte, b'a'..=b'z' | b'0'..=b'9' | b'_' | b'-' | b'*' | b'/');
    match key.split_once('@') {
        None => key.as_bytes()[0].is_ascii_lowercase() && key.bytes().all(is_tail_byte),
        Some((tenant, system)) => {
            !tenant.is_empty()
                && tenant.len() <= 241
                && (tenant.as_bytes()[0].is_ascii_lowercase()
                    || tenant.as_bytes()[0].is_ascii_digit())
                && tenant.bytes().all(is_tail_byte)
                && !system.is_empty()
                && system.len() <= 14
                && system.as_bytes()[0].is_ascii_lowercase()
                && system.bytes().all(is_tail_byte)
        }
    }
}

fn encode_tracestate_member_fields(
    member_key: &str,
    fields: &BTreeMap<String, String>,
) -> Result<(String, String), Box<dyn std::error::Error>> {
    // Configured fields are encoded into one opaque tracestate member value.
    // Validate both the field grammar and the final header value so malformed
    // config cannot produce propagated trace context that downstream W3C
    // extractors reject.
    let mut encoded = Vec::with_capacity(fields.len());
    for (field_key, value) in fields {
        if !is_configured_tracestate_field_key(field_key) {
            return Err(invalid_tracestate_config(format!(
                "invalid configured tracestate field key {member_key}.{field_key}"
            )));
        }
        if !is_configured_tracestate_field_value(value) {
            return Err(invalid_tracestate_config(format!(
                "invalid configured tracestate value for {member_key}.{field_key}"
            )));
        }
        encoded.push(format!("{field_key}:{value}"));
    }
    let value = encoded.join(";");
    if !is_header_safe_tracestate_member_value(&value) {
        return Err(invalid_tracestate_config(format!(
            "invalid configured tracestate value for {member_key}"
        )));
    }
    Ok((member_key.to_string(), value))
}

fn is_configured_tracestate_field_key(field_key: &str) -> bool {
    !field_key.is_empty()
        && field_key
            .bytes()
            .all(|byte| matches!(byte, b'!'..=b'~') && !matches!(byte, b':' | b';' | b',' | b'='))
}

fn is_configured_tracestate_field_value(value: &str) -> bool {
    value
        .bytes()
        .all(|byte| is_tracestate_member_value_byte(byte) && byte != b';')
}

fn is_header_safe_tracestate_member_value(value: &str) -> bool {
    value.is_empty()
        || (value.bytes().all(is_tracestate_member_value_byte)
            && value.as_bytes().last().is_some_and(|byte| *byte != b' '))
}

fn is_tracestate_member_value_byte(byte: u8) -> bool {
    matches!(byte, b' '..=b'~') && !matches!(byte, b',' | b'=')
}

fn invalid_tracestate_config(message: String) -> Box<dyn std::error::Error> {
    Box::new(std::io::Error::new(
        std::io::ErrorKind::InvalidInput,
        message,
    ))
}
