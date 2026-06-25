# Octo-Telnet

A web-based Telnet client in Rust.  
Browser ↔ WebSocket ↔ Rust ↔ TCP ↔ BBS.

## Quick Start

Run `cargo run` and then visit `http://localhost:2233` in your browser. Enter a BBS address and click Connect.

## Features

- Connects to public Telnet BBS servers.
- Renders terminal output in the browser with retro CRT styling.
- Supports ANSI colors, cursor movement, and CJK text.
- Encoding selectable: UTF-8, GBK, CP437.

## Modules

| Module | Purpose |
|--------|---------|
| `sha1.rs` | SHA-1 from scratch (RFC 3174) |
| `base64.rs` | Base64 from scratch (RFC 4648) |
| `http.rs` | HTTP/1.1 parser and response builder |
| `websocket.rs` | WebSocket frame encode/decode (RFC 6455) |
| `telnet.rs` | Telnet IAC state machine (RFC 854) |
| `handler.rs` | Bidirectional WS ↔ TCP proxy |
| `static/` | CRT‑style terminal frontend |

## Encoding

The terminal receives raw bytes from the BBS. Decoding (UTF‑8, GBK, or CP437) is applied per complete chunk. ANSI escapes are stripped at the byte level before text conversion.

## License

MIT