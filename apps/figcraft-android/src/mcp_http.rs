//! Embedded Streamable HTTP / SSE MCP server for FigCraft Android (V5_hotfix).

use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{mpsc, Arc};
use std::thread;
use std::time::Duration;

const MAX_CONCURRENT_CONNECTIONS: usize = 8;

pub struct PendingMcpRequest {
    pub payload: Value,
    pub reply_tx: mpsc::Sender<Value>,
}

#[derive(Debug, Clone)]
pub struct McpServerConfig {
    pub bind_addr: SocketAddr,
    pub bearer_token: String,
    pub session_id: String,
    pub max_body_bytes: usize,
}

impl Default for McpServerConfig {
    fn default() -> Self {
        let token = generate_session_token();
        let suffix = token.get(3..19).unwrap_or(&token);
        Self {
            bind_addr: "127.0.0.1:47800".parse().expect("valid loopback addr"),
            session_id: format!("sess-{suffix}"),
            bearer_token: token,
            max_body_bytes: 16 * 1024 * 1024,
        }
    }
}

pub struct EmbeddedMcpServer {
    pub config: McpServerConfig,
    shutdown: Arc<AtomicBool>,
    pub rx: mpsc::Receiver<PendingMcpRequest>,
}

struct ActiveConnGuard(Arc<AtomicUsize>);
impl Drop for ActiveConnGuard {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

impl EmbeddedMcpServer {
    pub fn start(config: McpServerConfig) -> Result<Self, String> {
        let listener = TcpListener::bind(config.bind_addr)
            .map_err(|e| format!("failed to bind MCP HTTP server on {}: {e}", config.bind_addr))?;
        listener
            .set_nonblocking(true)
            .map_err(|e| format!("set_nonblocking failed: {e}"))?;

        let (tx, rx) = mpsc::channel::<PendingMcpRequest>();
        let shutdown = Arc::new(AtomicBool::new(false));
        let active_conns = Arc::new(AtomicUsize::new(0));

        let shutdown_bg = Arc::clone(&shutdown);
        let token = config.bearer_token.clone();
        let session_id = config.session_id.clone();
        let max_body = config.max_body_bytes;

        thread::Builder::new()
            .name("figcraft-mcp-http".into())
            .spawn(move || {
                while !shutdown_bg.load(Ordering::Relaxed) {
                    match listener.accept() {
                        Ok((mut stream, _peer)) => {
                            let prev = active_conns.fetch_add(1, Ordering::AcqRel);
                            if prev >= MAX_CONCURRENT_CONNECTIONS {
                                active_conns.fetch_sub(1, Ordering::AcqRel);
                                write_http(
                                    &mut stream,
                                    "503 Service Unavailable",
                                    "application/json",
                                    &session_id,
                                    r#"{"error":"too many concurrent MCP connections"}"#,
                                );
                                continue;
                            }
                            let guard = ActiveConnGuard(Arc::clone(&active_conns));
                            let tx_clone = tx.clone();
                            let token_clone = token.clone();
                            let sess_clone = session_id.clone();
                            let shutdown_conn = Arc::clone(&shutdown_bg);
                            thread::spawn(move || {
                                let _guard = guard;
                                handle_connection(
                                    stream,
                                    &tx_clone,
                                    &token_clone,
                                    &sess_clone,
                                    max_body,
                                    &shutdown_conn,
                                );
                            });
                        }
                        Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                            thread::sleep(Duration::from_millis(20));
                        }
                        Err(err) => {
                            log::warn!("MCP accept error: {err}");
                            thread::sleep(Duration::from_millis(100));
                        }
                    }
                }
            })
            .map_err(|e| format!("failed to spawn MCP server thread: {e}"))?;

        Ok(Self {
            config,
            shutdown,
            rx,
        })
    }

    pub fn stop(&self) {
        self.shutdown.store(true, Ordering::Relaxed);
    }
}

impl Drop for EmbeddedMcpServer {
    fn drop(&mut self) {
        self.stop();
    }
}

fn handle_connection(
    mut stream: TcpStream,
    tx: &mpsc::Sender<PendingMcpRequest>,
    expected_token: &str,
    session_id: &str,
    max_body_bytes: usize,
    shutdown: &AtomicBool,
) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(15)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(15)));

    let mut reader = BufReader::new(match stream.try_clone() {
        Ok(s) => s,
        Err(_) => return,
    });

    let mut request_line = String::new();
    if reader.read_line(&mut request_line).unwrap_or(0) == 0 {
        return;
    }
    let parts: Vec<&str> = request_line.split_whitespace().collect();
    if parts.len() < 2 {
        write_http(&mut stream, "400 Bad Request", "application/json", session_id, r#"{"error":"bad request"}"#);
        return;
    }
    let method = parts[0];
    let path = parts[1].split('?').next().unwrap_or(parts[1]);

    let mut content_length: usize = 0;
    let mut is_chunked = false;
    let mut auth_header = String::new();
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).unwrap_or(0) == 0 {
            break;
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            break;
        }
        if let Some((k, v)) = trimmed.split_once(':') {
            let key = k.trim().to_ascii_lowercase();
            let val = v.trim();
            if key == "content-length" {
                content_length = val.parse().unwrap_or(0);
            } else if key == "transfer-encoding" && val.to_ascii_lowercase().contains("chunked") {
                is_chunked = true;
            } else if key == "authorization" {
                auth_header = val.to_owned();
            }
        }
    }

    if method == "OPTIONS" {
        write_http(&mut stream, "204 No Content", "text/plain", session_id, "");
        return;
    }

    if path == "/health" && method == "GET" {
        write_http(
            &mut stream,
            "200 OK",
            "application/json",
            session_id,
            r#"{"status":"ok","transport":"streamable-http","version":"v5_hotfix"}"#,
        );
        return;
    }

    let expected_header = format!("Bearer {expected_token}");
    if auth_header != expected_header {
        write_http(
            &mut stream,
            "401 Unauthorized",
            "application/json",
            session_id,
            r#"{"error":"invalid bearer token"}"#,
        );
        return;
    }

    if path == "/mcp" && method == "GET" {
        let header = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nMcp-Session-Id: {session_id}\r\nAccess-Control-Allow-Origin: *\r\n\r\n: connected\n\n"
        );
        if stream.write_all(header.as_bytes()).is_err() {
            return;
        }
        let _ = stream.flush();
        for _ in 0..120 {
            if shutdown.load(Ordering::Relaxed) {
                break;
            }
            thread::sleep(Duration::from_secs(5));
            if stream.write_all(b": keepalive\n\n").is_err() || stream.flush().is_err() {
                break;
            }
        }
        return;
    }

    if path == "/mcp" && method == "POST" {
        let body = if is_chunked {
            match read_chunked_body(&mut reader, max_body_bytes) {
                Ok(b) => b,
                Err(msg) => {
                    write_http(&mut stream, "400 Bad Request", "application/json", session_id, &json!({"error": msg}).to_string());
                    return;
                }
            }
        } else {
            if content_length == 0 || content_length > max_body_bytes {
                write_http(&mut stream, "413 Payload Too Large", "application/json", session_id, r#"{"error":"invalid content-length"}"#);
                return;
            }
            let mut buf = vec![0u8; content_length];
            if reader.read_exact(&mut buf).is_err() {
                write_http(&mut stream, "400 Bad Request", "application/json", session_id, r#"{"error":"incomplete body"}"#);
                return;
            }
            buf
        };

        let payload: Value = match serde_json::from_slice(&body) {
            Ok(v) => v,
            Err(e) => {
                let err = json!({"jsonrpc":"2.0","id":null,"error":{"code":-32700,"message":format!("parse error: {e}")}});
                write_http(&mut stream, "400 Bad Request", "application/json", session_id, &err.to_string());
                return;
            }
        };

        let (reply_tx, reply_rx) = mpsc::channel();
        if tx.send(PendingMcpRequest { payload, reply_tx }).is_err() {
            write_http(&mut stream, "503 Service Unavailable", "application/json", session_id, r#"{"error":"ui loop unavailable"}"#);
            return;
        }

        match reply_rx.recv_timeout(Duration::from_secs(30)) {
            Ok(reply) => write_http(&mut stream, "200 OK", "application/json", session_id, &reply.to_string()),
            Err(_) => write_http(&mut stream, "504 Gateway Timeout", "application/json", session_id, r#"{"error":"ui frame timeout"}"#),
        }
    } else {
        write_http(&mut stream, "404 Not Found", "application/json", session_id, r#"{"error":"not found"}"#);
    }
}

pub fn read_chunked_body<R: BufRead>(reader: &mut R, max_bytes: usize) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    loop {
        let mut size_line = String::new();
        if reader.read_line(&mut size_line).map_err(|e| e.to_string())? == 0 {
            return Err("unexpected EOF in chunked body".into());
        }
        let hex = size_line.trim().split(';').next().unwrap_or("");
        let chunk_len = usize::from_str_radix(hex, 16).map_err(|_| "invalid chunk size".to_string())?;
        if chunk_len == 0 {
            loop {
                let mut trailer = String::new();
                if reader.read_line(&mut trailer).unwrap_or(0) == 0 || trailer.trim().is_empty() {
                    break;
                }
            }
            break;
        }
        if out.len().saturating_add(chunk_len) > max_bytes {
            return Err("chunked body exceeds max_bytes".into());
        }
        let start = out.len();
        out.resize(start + chunk_len, 0);
        reader.read_exact(&mut out[start..]).map_err(|e| e.to_string())?;
        let mut crlf = [0u8; 2];
        reader.read_exact(&mut crlf).map_err(|e| e.to_string())?;
    }
    Ok(out)
}

fn write_http(stream: &mut TcpStream, status: &str, content_type: &str, session_id: &str, body: &str) {
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nMcp-Session-Id: {session_id}\r\nAccess-Control-Allow-Origin: *\r\nAccess-Control-Allow-Headers: Authorization, Content-Type, Mcp-Session-Id\r\nAccess-Control-Allow-Methods: GET, POST, OPTIONS\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(response.as_bytes());
    let _ = stream.flush();
}

fn generate_session_token() -> String {
    let mut bytes = [0u8; 16];
    if let Ok(mut f) = std::fs::File::open("/dev/urandom") {
        if f.read_exact(&mut bytes).is_ok() {
            let mut hex = String::with_capacity(35);
            hex.push_str("fc-");
            for b in bytes {
                use std::fmt::Write as _;
                let _ = write!(&mut hex, "{b:02x}");
            }
            return hex;
        }
    }
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("fc-{:032x}", nanos ^ 0xA5A5_5A5A_1234_5678_9ABC_DEF0_1122_3344u128)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn decodes_chunked_http_body_with_trailers() {
        let raw = b"4\r\nWiki\r\n5\r\npedia\r\n0\r\nX-Trailer: 1\r\n\r\n";
        let mut cursor = Cursor::new(&raw[..]);
        let decoded = read_chunked_body(&mut cursor, 1024).unwrap();
        assert_eq!(decoded, b"Wikipedia");
    }

    #[test]
    fn generates_128bit_token() {
        let t = generate_session_token();
        assert!(t.starts_with("fc-"));
        assert_eq!(t.len(), 35);
    }
}
