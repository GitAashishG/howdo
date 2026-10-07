use crate::error::Result;
use std::io::Read;

pub struct Response {
    pub status: u16,
    pub body: Vec<u8>,
}

/// Bound memory use even when an endpoint lies about its Content-Length.
pub fn send(request: minreq::Request, limit: usize) -> Result<Response> {
    let response = request
        .with_max_headers_size(64 * 1024)
        .with_max_status_line_length(8192)
        .send_lazy()
        .map_err(|e| format!("HTTP request failed: {e}"))?;
    let status = response.status_code;
    let chunked = response.headers.iter().any(|(name, value)| {
        name.eq_ignore_ascii_case("transfer-encoding") && value.eq_ignore_ascii_case("chunked")
    });
    let length = (!chunked)
        .then(|| {
            response
                .headers
                .iter()
                .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
                .and_then(|(_, value)| value.parse::<usize>().ok())
        })
        .flatten();
    if length.is_some_and(|length| length > limit) {
        return Err(format!("HTTP response exceeds the {limit}-byte limit.").into());
    }
    let mut body = Vec::new();
    response
        .take(limit as u64 + 1)
        .read_to_end(&mut body)
        .map_err(|e| format!("Failed to read HTTP response: {e}"))?;
    if body.len() > limit {
        return Err(format!("HTTP response exceeds the {limit}-byte limit.").into());
    }
    if length.is_some_and(|length| length != body.len()) {
        return Err("HTTP response body was truncated.".into());
    }
    Ok(Response { status, body })
}
