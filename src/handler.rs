//! Connection handler: WebSocket <-> Telnet bidirectional proxy
//! Static files are embedded at compile time for single-binary deployment.

use std::sync::Arc;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::TcpStream;
use tokio::sync::Mutex;

use crate::http;
use crate::telnet::TelnetParser;
use crate::websocket::{self, Opcode, ReadResult};

const INDEX_HTML: &str = include_str!("../static/index.html");
const STYLE_CSS: &str = include_str!("../static/style.css");
const APP_JS: &str = include_str!("../static/app.js");

/// Entry point: handle a single TCP connection
pub async fn handle_connection(mut stream: TcpStream) {
    let (request, leftover) = match read_http_request(&mut stream).await {
        Ok(result) => result,
        Err(e) => {
            eprintln!("Failed to read HTTP request: {}", e);
            let _ = send_error_response(&mut stream, 400, "Bad Request", &e).await;
            return;
        }
    };

    if request.is_websocket_upgrade() {
        if let Err(e) = handle_websocket(stream, request, leftover).await {
            eprintln!("WebSocket handler error: {}", e);
        }
    } else {
        serve_static_file(stream, &request.path).await;
    }
}

/// Read a complete HTTP request from the TCP stream.
/// Returns the parsed request and any leftover bytes (start of WS frames).
async fn read_http_request(
    stream: &mut TcpStream,
) -> Result<(http::HttpRequest, Vec<u8>), String> {
    let mut buf = Vec::with_capacity(4096);
    let mut tmp = [0u8; 4096];

    loop {
        if let Some(req) = http::parse_request(&buf)? {
            let total_len = http::request_total_len(&buf)
                .ok_or("Failed to compute request length")?;
            let leftover = buf[total_len..].to_vec();
            return Ok((req, leftover));
        }

        let n = stream
            .read(&mut tmp)
            .await
            .map_err(|e| format!("Read error: {}", e))?;
        if n == 0 {
            return Err("Connection closed before complete request".to_string());
        }
        buf.extend_from_slice(&tmp[..n]);

        if buf.len() > 65536 {
            return Err("HTTP request headers too large".to_string());
        }
    }
}

/// Handle a WebSocket connection: upgrade handshake, then proxy to Telnet
async fn handle_websocket(
    mut stream: TcpStream,
    request: http::HttpRequest,
    leftover: Vec<u8>,
) -> Result<(), String> {
    let client_key = request
        .ws_key()
        .ok_or("Missing Sec-WebSocket-Key header")?;
    let accept_key = websocket::compute_accept_key(client_key);

    let response = http::build_ws_upgrade_response(&accept_key);
    stream
        .write_all(&response)
        .await
        .map_err(|e| format!("Failed to send WS upgrade: {}", e))?;

    // Read the first frame: target host:port
    let mut ws_reader = WsReader::new(stream, leftover);
    let first_frame = ws_reader.read_frame().await?;
    let raw_addr = String::from_utf8(first_frame.payload)
        .map_err(|e| format!("Invalid UTF-8 in target address: {}", e))?
        .trim()
        .to_string();

    let target_addr = normalize_telnet_addr(&raw_addr);
    eprintln!("Connecting to Telnet server: {}", target_addr);

    let telnet_stream = match TcpStream::connect(&target_addr).await {
        Ok(s) => s,
        Err(e) => {
            let error_msg = format!(
                "\r\n\r\n*** Failed to connect to {} ***\r\n\r\nError: {}\r\n",
                target_addr, e
            );
            let (mut stream, _) = ws_reader.into_stream_with_buf();
            let _ = stream.write_all(&websocket::write_text_frame(&error_msg)).await;
            let _ = stream.write_all(&websocket::write_close_frame(Some(1011))).await;
            return Err(format!("Failed to connect to {}: {}", target_addr, e));
        }
    };

    eprintln!("Connected to {}", target_addr);

    let (ws_stream, ws_leftover) = ws_reader.into_stream_with_buf();
    let (ws_read, ws_write) = ws_stream.into_split();
    let (telnet_read, telnet_write) = telnet_stream.into_split();

    let ws_write = Arc::new(Mutex::new(ws_write));
    let telnet_write = Arc::new(Mutex::new(telnet_write));
    let target_addr = Arc::new(target_addr);

    let t1_addr = target_addr.clone();
    let t2_addr = target_addr.clone();

    let telnet_to_ws = async {
        if let Err(e) = telnet_to_websocket(telnet_read, ws_write.clone(), telnet_write.clone()).await {
            eprintln!("[{}] Telnet->WS error: {}", t1_addr, e);
        }
    };

    let ws_to_telnet = async {
        let mut reader = WsHalfReader::from_read_half_with_buf(ws_read, ws_leftover);
        if let Err(e) = websocket_to_telnet(&mut reader, telnet_write.clone(), ws_write.clone()).await {
            eprintln!("[{}] WS->Telnet error: {}", t2_addr, e);
        }
    };

    // Exit when either direction completes
    tokio::select! {
        _ = telnet_to_ws => {},
        _ = ws_to_telnet => {},
    }

    eprintln!("[{}] Session ended", target_addr);
    Ok(())
}

/// Direction: Telnet server -> WebSocket (browser)
async fn telnet_to_websocket(
    mut telnet_read: OwnedReadHalf,
    ws_write: Arc<Mutex<OwnedWriteHalf>>,
    telnet_write: Arc<Mutex<OwnedWriteHalf>>,
) -> Result<(), String> {
    let mut parser = TelnetParser::new();
    let mut buf = [0u8; 8192];

    loop {
        let n = telnet_read
            .read(&mut buf)
            .await
            .map_err(|e| format!("Telnet read error: {}", e))?;

        if n == 0 {
            let close_frame = websocket::write_close_frame(Some(1000));
            let mut ww = ws_write.lock().await;
            let _ = ww.write_all(&close_frame).await;
            return Ok(());
        }

        let output = parser.process(&buf[..n]);

        // Send Telnet negotiation responses back to the BBS server
        if !output.responses.is_empty() {
            let mut tw = telnet_write.lock().await;
            tw.write_all(&output.responses)
                .await
                .map_err(|e| format!("Telnet write error (responses): {}", e))?;
        }

        // Forward clean data to browser as WebSocket binary frames
        if !output.data.is_empty() {
            let frame = websocket::write_binary_frame(&output.data);
            let mut ww = ws_write.lock().await;
            ww.write_all(&frame)
                .await
                .map_err(|e| format!("WebSocket write error: {}", e))?;
        }
    }
}

/// Direction: WebSocket (browser) -> Telnet server
async fn websocket_to_telnet(
    reader: &mut WsHalfReader,
    telnet_write: Arc<Mutex<OwnedWriteHalf>>,
    ws_write: Arc<Mutex<OwnedWriteHalf>>,
) -> Result<(), String> {

    loop {
        let frame = reader.read_frame().await?;

        match frame.opcode {
            Opcode::Text | Opcode::Binary => {
                let mut tw = telnet_write.lock().await;
                tw.write_all(&frame.payload)
                    .await
                    .map_err(|e| format!("Telnet write error (input): {}", e))?;
            }
            Opcode::Ping => {
                let pong = websocket::write_pong_frame(&frame.payload);
                let mut ww = ws_write.lock().await;
                ww.write_all(&pong)
                    .await
                    .map_err(|e| format!("WebSocket pong write error: {}", e))?;
            }
            Opcode::Close => return Ok(()),
            Opcode::Pong | Opcode::Continuation => {}
        }
    }
}

/// Serve a static file based on the request path
async fn serve_static_file(mut stream: TcpStream, path: &str) {
    let (content, mime) = match path {
        "/" | "/index.html" => (INDEX_HTML.as_bytes(), "text/html; charset=utf-8"),
        "/style.css" => (STYLE_CSS.as_bytes(), "text/css; charset=utf-8"),
        "/app.js" => (APP_JS.as_bytes(), "application/javascript; charset=utf-8"),
        _ => {
            let body = "404 Not Found".as_bytes();
            let response = http::build_response(
                404,
                "Not Found",
                &[("Content-Type", "text/plain; charset=utf-8")],
                body,
            );
            let _ = stream.write_all(&response).await;
            return;
        }
    };

    let response = http::build_response(
        200,
        "OK",
        &[("Content-Type", mime), ("Cache-Control", "no-cache")],
        content,
    );
    let _ = stream.write_all(&response).await;
}

/// Send an HTTP error response
async fn send_error_response(
    stream: &mut TcpStream,
    code: u16,
    text: &str,
    detail: &str,
) -> Result<(), String> {
    let body = format!("{}: {}", text, detail);
    let response = http::build_response(
        code,
        text,
        &[("Content-Type", "text/plain; charset=utf-8")],
        body.as_bytes(),
    );
    stream
        .write_all(&response)
        .await
        .map_err(|e| format!("Failed to send error response: {}", e))
}

/// Normalize telnet address: strip telnet:// prefix, default port to 23
fn normalize_telnet_addr(raw: &str) -> String {
    let raw = raw.trim();
    let without_scheme = if let Some(rest) = raw.strip_prefix("telnet://") {
        rest.trim()
    } else {
        raw
    };
    if without_scheme.contains(':') {
        without_scheme.to_string()
    } else {
        format!("{}:23", without_scheme)
    }
}

// ---------------------------------------------------------------------------
// WebSocket Reader: buffered frame reader over a TcpStream or ReadHalf
// ---------------------------------------------------------------------------

/// A buffered reader that parses WebSocket frames from a TcpStream
pub struct WsReader {
    stream: TcpStream,
    buf: Vec<u8>,
}

impl WsReader {
    /// Create a new WsReader with optional leftover bytes from HTTP parsing
    pub fn new(stream: TcpStream, leftover: Vec<u8>) -> Self {
        Self {
            stream,
            buf: leftover,
        }
    }

    /// Consume the reader, returning the stream and any remaining buffer
    pub fn into_stream_with_buf(self) -> (TcpStream, Vec<u8>) {
        (self.stream, self.buf)
    }

    /// Read and parse the next WebSocket frame
    pub async fn read_frame(&mut self) -> Result<websocket::WsFrame, String> {
        let mut tmp = [0u8; 8192];

        loop {
            match websocket::read_frame(&self.buf)? {
                ReadResult::Frame(frame) => {
                    let len = websocket::frame_len(&self.buf)?;
                    self.buf.drain(..len);
                    return Ok(frame);
                }
                ReadResult::Closed(_) => {
                    return Err("WebSocket closed by peer".to_string());
                }
                ReadResult::NeedMore => {
                    let n = self
                        .stream
                        .read(&mut tmp)
                        .await
                        .map_err(|e| format!("WebSocket read error: {}", e))?;
                    if n == 0 {
                        return Err("WebSocket connection closed".to_string());
                    }
                    self.buf.extend_from_slice(&tmp[..n]);
                }
            }
        }
    }
}

/// A WebSocket reader over an OwnedReadHalf (post-stream-split)
pub struct WsHalfReader {
    reader: OwnedReadHalf,
    buf: Vec<u8>,
}

impl WsHalfReader {
    /// Create a WsHalfReader with an initial buffer (carry-over from pre-split)
    pub fn from_read_half_with_buf(reader: OwnedReadHalf, buf: Vec<u8>) -> Self {
        Self { reader, buf }
    }

    /// Read and parse the next WebSocket frame
    pub async fn read_frame(&mut self) -> Result<websocket::WsFrame, String> {
        let mut tmp = [0u8; 8192];

        loop {
            match websocket::read_frame(&self.buf)? {
                ReadResult::Frame(frame) => {
                    let len = websocket::frame_len(&self.buf)?;
                    self.buf.drain(..len);
                    return Ok(frame);
                }
                ReadResult::Closed(_) => {
                    return Err("WebSocket closed by peer".to_string());
                }
                ReadResult::NeedMore => {
                    let n = self
                        .reader
                        .read(&mut tmp)
                        .await
                        .map_err(|e| format!("WebSocket read error: {}", e))?;
                    if n == 0 {
                        return Err("WebSocket connection closed".to_string());
                    }
                    self.buf.extend_from_slice(&tmp[..n]);
                }
            }
        }
    }
}
