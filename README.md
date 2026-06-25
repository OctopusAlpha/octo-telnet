# Octo-Telnet

```
╔══════════════════════════════════╗
║  ┌────────────────────────────┐  ║
║  │                            │  ║
║  │       octo-telnet          │  ║
║  │   raw bytes, bare hands    │  ║
║  │                            │  ║
║  │   > _                      │  ║
║  └────────────────────────────┘  ║
╚══════════════════════════════════╝
```

A web-based Telnet client in Rust.

Raw TCP, hand-parsed HTTP, hand-rolled WebSocket, and Telnet from the ground up.

```
Browser <--WS--> Rust <--TCP--> BBS
```

## Quick start

```bash
cargo run
```

Open `http://localhost:2233`, type a BBS address, click Connect.

## What it does

- Connects to public Telnet BBS servers
- Renders terminal output with retro CRT styling in the browser
- Handles ANSI colors, cursor movement, CJK text
- Encoding switchable: UTF-8, GBK, CP437

## What's inside

| Module | Layer |
|--------|-------|
| `sha1.rs` | SHA-1 from scratch (RFC 3174) |
| `base64.rs` | Base64 from scratch (RFC 4648) |
| `http.rs` | HTTP/1.1 parser and response builder |
| `websocket.rs` | WebSocket frame encode/decode (RFC 6455) |
| `telnet.rs` | Telnet IAC state machine (RFC 854) |
| `handler.rs` | Bidirectional WS <-> TCP proxy |
| `static/` | CRT-style terminal frontend |

## A byte is a byte

The terminal receives raw bytes from the BBS and decodes them at rest. UTF-8, GBK, CP437 -- the encoding is yours to choose, and it takes effect only when a full chunk lands. ANSI escapes are peeled off at the byte level before the text is touched.

## License

MIT
