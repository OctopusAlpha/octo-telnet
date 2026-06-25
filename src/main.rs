//! Octo-Telnet: web-based Telnet client in Rust
//! Raw TCP listener, hand-parsed HTTP/1.1, WebSocket (RFC 6455),
//! Telnet protocol (RFC 854)

mod base64;
mod handler;
mod http;
mod sha1;
mod telnet;
mod websocket;

use std::env;

const DEFAULT_PORT: u16 = 2233;
const DEFAULT_HOST: &str = "0.0.0.0";

#[tokio::main]
async fn main() {
    let port: u16 = env::var("OCTO_TELNET_PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_PORT);

    let host = env::var("OCTO_TELNET_HOST").unwrap_or_else(|_| DEFAULT_HOST.to_string());
    let addr = format!("{}:{}", host, port);

    let listener = match tokio::net::TcpListener::bind(&addr).await {
        Ok(l) => l,
        Err(e) => {
            eprintln!("Failed to bind to {}: {}", addr, e);
            std::process::exit(1);
        }
    };

    eprintln!("Octo-Telnet server listening on http://{}", addr);
    eprintln!("Open this URL in your browser to start a Telnet session.");
    eprintln!("Press Ctrl+C to stop the server.");

    loop {
        match listener.accept().await {
            Ok((stream, peer_addr)) => {
                eprintln!("New connection from {}", peer_addr);
                tokio::spawn(async move {
                    handler::handle_connection(stream).await;
                });
            }
            Err(e) => {
                eprintln!("Accept error: {}", e);
            }
        }
    }
}
