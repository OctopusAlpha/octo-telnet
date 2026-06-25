//! HTTP/1.1 request parser and response builder (RFC 7230)

#![allow(dead_code)]

use std::collections::HashMap;

pub struct HttpRequest {
    pub method: String,
    pub path: String,
    pub headers: HashMap<String, String>,
}

impl HttpRequest {
    /// Check if this is a WebSocket upgrade request
    pub fn is_websocket_upgrade(&self) -> bool {
        let upgrade = self
            .headers
            .get("upgrade")
            .map(|v| v.eq_ignore_ascii_case("websocket"))
            .unwrap_or(false);

        let connection = self
            .headers
            .get("connection")
            .map(|v| v.to_ascii_lowercase().contains("upgrade"))
            .unwrap_or(false);

        upgrade && connection
    }

    /// Get the Sec-WebSocket-Key header value
    pub fn ws_key(&self) -> Option<&str> {
        self.headers.get("sec-websocket-key").map(|v| v.as_str())
    }
}

/// Parse an HTTP request from a raw byte buffer.
/// Returns Ok(Some(req)) on complete request, Ok(None) if more data needed.
pub fn parse_request(buf: &[u8]) -> Result<Option<HttpRequest>, String> {
    let header_end = find_crlfcrlf(buf);
    if header_end.is_none() {
        return Ok(None);
    }
    let header_end = header_end.unwrap();

    let header_str = std::str::from_utf8(&buf[..header_end])
        .map_err(|e| format!("Invalid UTF-8 in headers: {}", e))?;

    let mut lines = header_str.lines();
    let request_line = lines.next().ok_or("Empty request")?;
    let parts: Vec<&str> = request_line.split_whitespace().collect();
    if parts.len() < 3 {
        return Err(format!("Malformed request line: {}", request_line));
    }

    let method = parts[0].to_string();
    let path = parts[1].to_string();

    let mut headers = HashMap::new();
    for line in lines {
        if line.is_empty() {
            continue;
        }
        if let Some(colon_pos) = line.find(':') {
            let key = line[..colon_pos].trim().to_ascii_lowercase();
            let value = line[colon_pos + 1..].trim().to_string();
            headers.insert(key, value);
        }
    }

    Ok(Some(HttpRequest {
        method,
        path,
        headers,
    }))
}

fn find_crlfcrlf(buf: &[u8]) -> Option<usize> {
    for i in 0..buf.len().saturating_sub(3) {
        if buf[i] == b'\r' && buf[i + 1] == b'\n' && buf[i + 2] == b'\r' && buf[i + 3] == b'\n' {
            return Some(i);
        }
    }
    None
}

/// Return the total bytes consumed by the HTTP request (including CRLFCRLF)
pub fn request_total_len(buf: &[u8]) -> Option<usize> {
    find_crlfcrlf(buf).map(|pos| pos + 4)
}

/// Build a raw HTTP response as bytes
pub fn build_response(status_code: u16, status_text: &str, headers: &[(&str, &str)], body: &[u8]) -> Vec<u8> {
    let mut result = Vec::new();

    let status_line = format!("HTTP/1.1 {} {}\r\n", status_code, status_text);
    result.extend_from_slice(status_line.as_bytes());

    for (key, value) in headers {
        let header_line = format!("{}: {}\r\n", key, value);
        result.extend_from_slice(header_line.as_bytes());
    }

    let content_length = format!("Content-Length: {}\r\n", body.len());
    result.extend_from_slice(content_length.as_bytes());

    result.extend_from_slice(b"\r\n");
    result.extend_from_slice(body);
    result
}

/// Build a WebSocket upgrade response (101 Switching Protocols)
pub fn build_ws_upgrade_response(accept_key: &str) -> Vec<u8> {
    format!(
        "HTTP/1.1 101 Switching Protocols\r\n\
         Upgrade: websocket\r\n\
         Connection: Upgrade\r\n\
         Sec-WebSocket-Accept: {}\r\n\
         \r\n",
        accept_key
    )
    .into_bytes()
}

/// Guess MIME type from file extension
pub fn mime_type(path: &str) -> &'static str {
    let ext = path.rsplit('.').next().unwrap_or("");
    match ext.to_ascii_lowercase().as_str() {
        "html" | "htm" => "text/html; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "js" => "application/javascript; charset=utf-8",
        "json" => "application/json; charset=utf-8",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "svg" => "image/svg+xml",
        "ico" => "image/x-icon",
        _ => "application/octet-stream",
    }
}
