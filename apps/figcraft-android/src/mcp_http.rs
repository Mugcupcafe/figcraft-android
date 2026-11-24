//! Embedded Streamable HTTP / SSE MCP server + standard OAuth 2.1 (PKCE + DCR)
//! for FigCraft Android (V30).
//!
//! - Keeps existing `Authorization: Bearer <mcp_token>` direct auth 100% intact for Muse, Gemini CLI, etc.
//! - Adds RFC 9728 (`/.well-known/oauth-protected-resource`), RFC 8414 (`/.well-known/oauth-authorization-server`),
//!   RFC 7591 (`/oauth/register`), `/oauth/authorize`, `/oauth/token`, and `/oauth/revoke` so Gemini Web/App
//!   ("MCP 서버에 연결") and other OAuth 2.1 MCP clients connect seamlessly.

use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

const MAX_CONCURRENT_CONNECTIONS: usize = 12;

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

impl McpServerConfig {
    pub fn with_token(bind_addr: SocketAddr, bearer_token: String) -> Self {
        let tok = if bearer_token.trim().is_empty() {
            generate_session_token()
        } else {
            bearer_token.trim().to_owned()
        };
        let suffix = tok.get(3..19).unwrap_or(&tok);
        Self {
            bind_addr,
            session_id: format!("sess-{suffix}"),
            bearer_token: tok,
            max_body_bytes: 16 * 1024 * 1024,
        }
    }
}

impl Default for McpServerConfig {
    fn default() -> Self {
        Self::with_token(
            "127.0.0.1:47800".parse().expect("valid loopback addr"),
            generate_session_token(),
        )
    }
}

#[derive(Clone)]
struct AuthCodeEntry {
    client_id: String,
    redirect_uri: String,
    code_challenge: String,
    code_challenge_method: String,
    created_at: Instant,
}

#[derive(Default)]
struct OAuthState {
    registered_clients: HashMap<String, String>, // client_id -> client_secret
    auth_codes: HashMap<String, AuthCodeEntry>,
    access_tokens: HashSet<String>,
    refresh_tokens: HashSet<String>,
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
        let oauth = Arc::new(Mutex::new(OAuthState::default()));

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
                                    None,
                                    r#"{"error":"too many concurrent MCP connections"}"#,
                                );
                                continue;
                            }
                            let guard = ActiveConnGuard(Arc::clone(&active_conns));
                            let tx_clone = tx.clone();
                            let token_clone = token.clone();
                            let sess_clone = session_id.clone();
                            let shutdown_conn = Arc::clone(&shutdown_bg);
                            let oauth_conn = Arc::clone(&oauth);
                            thread::spawn(move || {
                                let _guard = guard;
                                handle_connection(
                                    stream,
                                    &tx_clone,
                                    &token_clone,
                                    &sess_clone,
                                    max_body,
                                    &shutdown_conn,
                                    &oauth_conn,
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
    oauth: &Mutex<OAuthState>,
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
        write_http(&mut stream, "400 Bad Request", "application/json", session_id, None, r#"{"error":"bad request"}"#);
        return;
    }
    let method = parts[0];
    let raw_target = parts[1];
    let (path, query_str) = raw_target.split_once('?').unwrap_or((raw_target, ""));

    let mut content_length: usize = 0;
    let mut is_chunked = false;
    let mut auth_header = String::new();
    let mut host_header = String::new();
    let mut fwd_proto = String::new();
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
            } else if key == "host" {
                host_header = val.to_owned();
            } else if key == "x-forwarded-proto" {
                fwd_proto = val.to_owned();
            }
        }
    }

    if method == "OPTIONS" {
        write_http(&mut stream, "204 No Content", "text/plain", session_id, None, "");
        return;
    }

    let origin = infer_origin(&host_header, &fwd_proto);

    if path == "/health" && method == "GET" {
        write_http(
            &mut stream,
            "200 OK",
            "application/json",
            session_id,
            None,
            r#"{"status":"ok","transport":"streamable-http","oauth":"2.1","version":"v30"}"#,
        );
        return;
    }

    // --- Standard MCP OAuth 2.1 Discovery & Endpoints (public, before Bearer check) ---
    if method == "GET"
        && (path == "/.well-known/oauth-protected-resource"
            || path == "/.well-known/oauth-protected-resource/mcp")
    {
        let meta = json!({
            "resource": format!("{origin}/mcp"),
            "authorization_servers": [origin],
            "bearer_methods_supported": ["header"],
            "scopes_supported": ["mcp"]
        });
        write_http(&mut stream, "200 OK", "application/json", session_id, None, &meta.to_string());
        return;
    }

    if method == "GET"
        && (path == "/.well-known/oauth-authorization-server"
            || path == "/.well-known/openid-configuration")
    {
        let meta = json!({
            "issuer": origin,
            "authorization_endpoint": format!("{origin}/oauth/authorize"),
            "token_endpoint": format!("{origin}/oauth/token"),
            "registration_endpoint": format!("{origin}/oauth/register"),
            "revocation_endpoint": format!("{origin}/oauth/revoke"),
            "response_types_supported": ["code"],
            "grant_types_supported": ["authorization_code", "refresh_token"],
            "code_challenge_methods_supported": ["S256", "plain"],
            "token_endpoint_auth_methods_supported": ["none", "client_secret_post", "client_secret_basic"],
            "scopes_supported": ["mcp"]
        });
        write_http(&mut stream, "200 OK", "application/json", session_id, None, &meta.to_string());
        return;
    }

    if path == "/oauth/register" && method == "POST" {
        let body = match read_request_body(&mut reader, is_chunked, content_length, max_body_bytes) {
            Ok(b) => b,
            Err(e) => {
                write_http(&mut stream, "400 Bad Request", "application/json", session_id, None, &json!({"error":"invalid_request","error_description":e}).to_string());
                return;
            }
        };
        let mut req_json: Value = serde_json::from_slice(&body).unwrap_or_else(|_| json!({}));
        let client_id = format!("fc-client-{}", &generate_session_token()[3..15]);
        let client_secret = generate_session_token();
        if let Ok(mut st) = oauth.lock() {
            st.registered_clients.insert(client_id.clone(), client_secret.clone());
        }
        if let Some(obj) = req_json.as_object_mut() {
            obj.insert("client_id".into(), json!(client_id));
            obj.insert("client_secret".into(), json!(client_secret));
            obj.insert("client_id_issued_at".into(), json!(1700000000));
            obj.insert("client_secret_expires_at".into(), json!(0));
        }
        write_http(&mut stream, "201 Created", "application/json", session_id, None, &req_json.to_string());
        return;
    }

    if path == "/oauth/authorize" && method == "GET" {
        let params = parse_form_urlencoded(query_str);
        let html = render_authorize_html(&params, None);
        write_http(&mut stream, "200 OK", "text/html; charset=utf-8", session_id, None, &html);
        return;
    }

    if path == "/oauth/authorize" && method == "POST" {
        let body = match read_request_body(&mut reader, is_chunked, content_length, max_body_bytes) {
            Ok(b) => b,
            Err(e) => {
                write_http(&mut stream, "400 Bad Request", "text/plain; charset=utf-8", session_id, None, &e);
                return;
            }
        };
        let body_str = String::from_utf8_lossy(&body);
        let params = parse_form_urlencoded(&body_str);
        let action = params.get("action").map(String::as_str).unwrap_or("allow");
        let redirect_uri = params.get("redirect_uri").cloned().unwrap_or_default();
        let state = params.get("state").cloned().unwrap_or_default();

        if action == "deny" {
            if !redirect_uri.is_empty() {
                let sep = if redirect_uri.contains('?') { '&' } else { '?' };
                let mut loc = format!("{redirect_uri}{sep}error=access_denied");
                if !state.is_empty() {
                    loc.push_str(&format!("&state={}", url_encode(&state)));
                }
                write_redirect(&mut stream, &loc);
            } else {
                write_http(&mut stream, "403 Forbidden", "text/plain; charset=utf-8", session_id, None, "연결이 취소되었습니다.");
            }
            return;
        }

        let provided_tok = params.get("mcp_token").map(|s| s.trim()).unwrap_or("");
        if provided_tok != expected_token {
            let html = render_authorize_html(
                &params,
                Some("MCP 토큰이 일치하지 않습니다. FigCraft 앱 상단 MCP 창에서 토큰을 복사해 붙여넣어 주세요."),
            );
            write_http(&mut stream, "200 OK", "text/html; charset=utf-8", session_id, None, &html);
            return;
        }

        let code = format!("fccode_{}", &generate_session_token()[3..]);
        let entry = AuthCodeEntry {
            client_id: params.get("client_id").cloned().unwrap_or_default(),
            redirect_uri: redirect_uri.clone(),
            code_challenge: params.get("code_challenge").cloned().unwrap_or_default(),
            code_challenge_method: params
                .get("code_challenge_method")
                .cloned()
                .unwrap_or_else(|| "S256".to_owned()),
            created_at: Instant::now(),
        };
        if let Ok(mut st) = oauth.lock() {
            st.auth_codes.insert(code.clone(), entry);
        }

        let sep = if redirect_uri.contains('?') { '&' } else { '?' };
        let mut loc = format!("{redirect_uri}{sep}code={}", url_encode(&code));
        if !state.is_empty() {
            loc.push_str(&format!("&state={}", url_encode(&state)));
        }
        write_redirect(&mut stream, &loc);
        return;
    }

    if path == "/oauth/token" && method == "POST" {
        let body = match read_request_body(&mut reader, is_chunked, content_length, max_body_bytes) {
            Ok(b) => b,
            Err(e) => {
                write_http(&mut stream, "400 Bad Request", "application/json", session_id, None, &json!({"error":"invalid_request","error_description":e}).to_string());
                return;
            }
        };
        let body_str = String::from_utf8_lossy(&body);
        let params: HashMap<String, String> = if body_str.trim_start().starts_with('{') {
            serde_json::from_str::<Value>(&body_str)
                .ok()
                .and_then(|v| v.as_object().cloned())
                .map(|m| {
                    m.into_iter()
                        .filter_map(|(k, v)| v.as_str().map(|s| (k, s.to_owned())))
                        .collect()
                })
                .unwrap_or_default()
        } else {
            parse_form_urlencoded(&body_str)
        };

        let grant_type = params.get("grant_type").map(String::as_str).unwrap_or("");
        if grant_type == "authorization_code" {
            let code = params.get("code").map(String::as_str).unwrap_or("");
            let verifier = params.get("code_verifier").map(String::as_str).unwrap_or("");
            let redirect_uri = params.get("redirect_uri").map(String::as_str).unwrap_or("");

            let entry = oauth.lock().ok().and_then(|mut st| st.auth_codes.remove(code));
            let Some(entry) = entry else {
                write_http(&mut stream, "400 Bad Request", "application/json", session_id, None, r#"{"error":"invalid_grant","error_description":"unknown or used authorization code"}"#);
                return;
            };
            if entry.created_at.elapsed() > Duration::from_secs(600) {
                write_http(&mut stream, "400 Bad Request", "application/json", session_id, None, r#"{"error":"invalid_grant","error_description":"authorization code expired"}"#);
                return;
            }
            if !entry.redirect_uri.is_empty() && !redirect_uri.is_empty() && entry.redirect_uri != redirect_uri {
                write_http(&mut stream, "400 Bad Request", "application/json", session_id, None, r#"{"error":"invalid_grant","error_description":"redirect_uri mismatch"}"#);
                return;
            }
            if !entry.code_challenge.is_empty() {
                let ok_pkce = verify_pkce(&entry.code_challenge, &entry.code_challenge_method, verifier);
                if !ok_pkce {
                    write_http(&mut stream, "400 Bad Request", "application/json", session_id, None, r#"{"error":"invalid_grant","error_description":"PKCE code_verifier mismatch"}"#);
                    return;
                }
            }

            let access_token = format!("fc-oa-{}", &generate_session_token()[3..]);
            let refresh_token = format!("fc-rt-{}", &generate_session_token()[3..]);
            if let Ok(mut st) = oauth.lock() {
                st.access_tokens.insert(access_token.clone());
                st.refresh_tokens.insert(refresh_token.clone());
            }
            let resp = json!({
                "access_token": access_token,
                "token_type": "Bearer",
                "expires_in": 86400,
                "refresh_token": refresh_token,
                "scope": "mcp"
            });
            write_http(&mut stream, "200 OK", "application/json", session_id, None, &resp.to_string());
            return;
        } else if grant_type == "refresh_token" {
            let rt = params.get("refresh_token").map(String::as_str).unwrap_or("");
            let valid_rt = oauth.lock().map(|st| st.refresh_tokens.contains(rt)).unwrap_or(false);
            if !valid_rt {
                write_http(&mut stream, "400 Bad Request", "application/json", session_id, None, r#"{"error":"invalid_grant","error_description":"invalid refresh_token"}"#);
                return;
            }
            let access_token = format!("fc-oa-{}", &generate_session_token()[3..]);
            if let Ok(mut st) = oauth.lock() {
                st.access_tokens.insert(access_token.clone());
            }
            let resp = json!({
                "access_token": access_token,
                "token_type": "Bearer",
                "expires_in": 86400,
                "refresh_token": rt,
                "scope": "mcp"
            });
            write_http(&mut stream, "200 OK", "application/json", session_id, None, &resp.to_string());
            return;
        } else {
            write_http(&mut stream, "400 Bad Request", "application/json", session_id, None, r#"{"error":"unsupported_grant_type"}"#);
            return;
        }
    }

    if path == "/oauth/revoke" && method == "POST" {
        if let Ok(body) = read_request_body(&mut reader, is_chunked, content_length, max_body_bytes) {
            let body_str = String::from_utf8_lossy(&body);
            let params = parse_form_urlencoded(&body_str);
            if let Some(tok) = params.get("token") {
                if let Ok(mut st) = oauth.lock() {
                    st.access_tokens.remove(tok);
                    st.refresh_tokens.remove(tok);
                }
            }
        }
        write_http(&mut stream, "200 OK", "application/json", session_id, None, "{}");
        return;
    }

    // --- Protected MCP Endpoints (/mcp) ---
    let bearer_value = auth_header
        .strip_prefix("Bearer ")
        .or_else(|| auth_header.strip_prefix("bearer "))
        .unwrap_or("")
        .trim();

    let authorized = !bearer_value.is_empty()
        && (bearer_value == expected_token
            || oauth
                .lock()
                .map(|st| st.access_tokens.contains(bearer_value))
                .unwrap_or(false));

    if !authorized {
        let www_auth = format!(
            "Bearer realm=\"figcraft\", resource_metadata=\"{origin}/.well-known/oauth-protected-resource\""
        );
        write_http(
            &mut stream,
            "401 Unauthorized",
            "application/json",
            session_id,
            Some(&www_auth),
            r#"{"error":"unauthorized","error_description":"Provide Bearer token or complete MCP OAuth 2.1"}"#,
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
        let body = match read_request_body(&mut reader, is_chunked, content_length, max_body_bytes) {
            Ok(b) => b,
            Err(msg) => {
                write_http(&mut stream, "400 Bad Request", "application/json", session_id, None, &json!({"error": msg}).to_string());
                return;
            }
        };

        let payload: Value = match serde_json::from_slice(&body) {
            Ok(v) => v,
            Err(e) => {
                let err = json!({"jsonrpc":"2.0","id":null,"error":{"code":-32700,"message":format!("parse error: {e}")}});
                write_http(&mut stream, "400 Bad Request", "application/json", session_id, None, &err.to_string());
                return;
            }
        };

        let (reply_tx, reply_rx) = mpsc::channel();
        if tx.send(PendingMcpRequest { payload, reply_tx }).is_err() {
            write_http(&mut stream, "503 Service Unavailable", "application/json", session_id, None, r#"{"error":"ui loop unavailable"}"#);
            return;
        }

        match reply_rx.recv_timeout(Duration::from_secs(30)) {
            Ok(reply) => write_http(&mut stream, "200 OK", "application/json", session_id, None, &reply.to_string()),
            Err(_) => write_http(&mut stream, "504 Gateway Timeout", "application/json", session_id, None, r#"{"error":"ui frame timeout"}"#),
        }
    } else {
        write_http(&mut stream, "404 Not Found", "application/json", session_id, None, r#"{"error":"not found"}"#);
    }
}

fn infer_origin(host: &str, fwd_proto: &str) -> String {
    let h = if host.is_empty() { "127.0.0.1:47800" } else { host.trim() };
    let proto = if !fwd_proto.is_empty() {
        fwd_proto.split(',').next().unwrap_or("https").trim()
    } else if h.starts_with("127.0.0.1") || h.starts_with("localhost") || h.starts_with("192.168.") || h.starts_with("10.") {
        "http"
    } else {
        "https"
    };
    format!("{proto}://{h}")
}

fn read_request_body<R: BufRead>(
    reader: &mut R,
    is_chunked: bool,
    content_length: usize,
    max_body_bytes: usize,
) -> Result<Vec<u8>, String> {
    if is_chunked {
        read_chunked_body(reader, max_body_bytes)
    } else if content_length == 0 {
        Ok(Vec::new())
    } else if content_length > max_body_bytes {
        Err("payload too large".into())
    } else {
        let mut buf = vec![0u8; content_length];
        reader.read_exact(&mut buf).map_err(|_| "incomplete body".to_string())?;
        Ok(buf)
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

fn write_http(
    stream: &mut TcpStream,
    status: &str,
    content_type: &str,
    session_id: &str,
    www_authenticate: Option<&str>,
    body: &str,
) {
    let auth_line = match www_authenticate {
        Some(v) => format!("WWW-Authenticate: {v}\r\n"),
        None => String::new(),
    };
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nMcp-Session-Id: {session_id}\r\n{auth_line}Access-Control-Allow-Origin: *\r\nAccess-Control-Allow-Headers: Authorization, Content-Type, Mcp-Session-Id\r\nAccess-Control-Allow-Methods: GET, POST, OPTIONS\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(response.as_bytes());
    let _ = stream.flush();
}

fn write_redirect(stream: &mut TcpStream, location: &str) {
    let response = format!(
        "HTTP/1.1 302 Found\r\nLocation: {location}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
    );
    let _ = stream.write_all(response.as_bytes());
    let _ = stream.flush();
}

fn render_authorize_html(params: &HashMap<String, String>, error_msg: Option<&str>) -> String {
    let get = |k: &str| html_escape(params.get(k).map(String::as_str).unwrap_or(""));
    let err_html = match error_msg {
        Some(msg) => format!(r#"<div class="err">{}</div>"#, html_escape(msg)),
        None => String::new(),
    };
    format!(
        r#"<!DOCTYPE html>
<html lang="ko">
<head>
<meta charset="utf-8" />
<meta name="viewport" content="width=device-width, initial-scale=1" />
<title>FigCraft Mobile · AI 연결 허용</title>
<style>
  body {{ margin:0; font-family:-apple-system,BlinkMacSystemFont,'Segoe UI',sans-serif; background:#12151a; color:#eef3f8; display:flex; min-height:100vh; align-items:center; justify-content:center; padding:18px; }}
  .card {{ width:100%; max-width:420px; background:#1c2129; border:1px solid #2d3542; border-radius:18px; padding:24px; box-shadow:0 16px 40px rgba(0,0,0,.45); }}
  .badge {{ display:inline-block; background:rgba(84,238,192,.14); color:#54eec0; font-weight:700; font-size:12px; padding:4px 10px; border-radius:999px; margin-bottom:10px; }}
  h1 {{ font-size:20px; margin:0 0 8px; }}
  p {{ font-size:14px; color:#aab4c2; line-height:1.5; margin:0 0 16px; }}
  label {{ display:block; font-size:13px; color:#cbd5e1; margin-bottom:6px; font-weight:600; }}
  input[type=password], input[type=text] {{ width:100%; box-sizing:border-box; padding:12px 14px; border-radius:12px; border:1px solid #364152; background:#10141a; color:#fff; font-size:15px; outline:none; }}
  input:focus {{ border-color:#54eec0; }}
  .err {{ background:rgba(244,63,94,.16); border:1px solid rgba(244,63,94,.45); color:#fda4af; padding:10px 12px; border-radius:10px; font-size:13px; margin-bottom:14px; }}
  .btns {{ display:flex; gap:10px; margin-top:16px; }}
  button {{ flex:1; padding:12px; border-radius:12px; border:none; font-size:15px; font-weight:700; cursor:pointer; }}
  .allow {{ background:#54eec0; color:#0c1714; }}
  .deny {{ background:#2a313d; color:#d2dae6; }}
</style>
</head>
<body>
  <div class="card">
    <div class="badge">FigCraft Mobile MCP · OAuth 2.1</div>
    <h1>AI 에이전트 연결 허용</h1>
    <p>연결하려는 AI(Gemini 등)가 태블릿의 FigCraft 캔버스를 읽고 편집할 수 있도록 허용합니다.</p>
    {err_html}
    <form method="POST" action="/oauth/authorize">
      <input type="hidden" name="client_id" value="{}" />
      <input type="hidden" name="redirect_uri" value="{}" />
      <input type="hidden" name="state" value="{}" />
      <input type="hidden" name="code_challenge" value="{}" />
      <input type="hidden" name="code_challenge_method" value="{}" />
      <input type="hidden" name="scope" value="{}" />
      <label for="tok">FigCraft MCP 토큰</label>
      <input id="tok" name="mcp_token" type="text" placeholder="fc-..." autocomplete="off" required />
      <div class="btns">
        <button class="deny" type="submit" name="action" value="deny" formnovalidate>취소</button>
        <button class="allow" type="submit" name="action" value="allow">연결 허용</button>
      </div>
    </form>
  </div>
</body>
</html>"#,
        get("client_id"),
        get("redirect_uri"),
        get("state"),
        get("code_challenge"),
        get("code_challenge_method"),
        get("scope"),
    )
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn parse_form_urlencoded(input: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for pair in input.split('&') {
        if pair.is_empty() {
            continue;
        }
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        map.insert(url_decode(k), url_decode(v));
    }
    map
}

fn url_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b'%' if i + 2 < bytes.len() => {
                if let Ok(hex) = std::str::from_utf8(&bytes[i + 1..i + 3]) {
                    if let Ok(b) = u8::from_str_radix(hex, 16) {
                        out.push(b);
                        i += 3;
                        continue;
                    }
                }
                out.push(bytes[i]);
                i += 1;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn url_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            use std::fmt::Write as _;
            let _ = write!(&mut out, "%{b:02X}");
        }
    }
    out
}

fn verify_pkce(challenge: &str, method: &str, verifier: &str) -> bool {
    if verifier.is_empty() {
        return false;
    }
    if method.eq_ignore_ascii_case("plain") {
        return challenge == verifier;
    }
    let digest = sha256(verifier.as_bytes());
    let b64 = base64_url_nopad(&digest);
    challenge == b64
}

fn base64_url_nopad(input: &[u8]) -> String {
    const TBL: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::with_capacity((input.len() * 4 + 2) / 3);
    let mut i = 0;
    while i < input.len() {
        let b0 = input[i] as u32;
        let b1 = if i + 1 < input.len() { input[i + 1] as u32 } else { 0 };
        let b2 = if i + 2 < input.len() { input[i + 2] as u32 } else { 0 };
        let triple = (b0 << 16) | (b1 << 8) | b2;
        out.push(TBL[((triple >> 18) & 0x3F) as usize] as char);
        out.push(TBL[((triple >> 12) & 0x3F) as usize] as char);
        if i + 1 < input.len() {
            out.push(TBL[((triple >> 6) & 0x3F) as usize] as char);
        }
        if i + 2 < input.len() {
            out.push(TBL[(triple & 0x3F) as usize] as char);
        }
        i += 3;
    }
    out
}

fn sha256(data: &[u8]) -> [u8; 32] {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
        0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
        0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
        0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
        0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
        0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
        0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
    ];
    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
    ];
    let mut msg = data.to_vec();
    let bit_len = (data.len() as u64) * 8;
    msg.push(0x80);
    while (msg.len() % 64) != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bit_len.to_be_bytes());

    for chunk in msg.chunks_exact(64) {
        let mut w = [0u32; 64];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([chunk[i * 4], chunk[i * 4 + 1], chunk[i * 4 + 2], chunk[i * 4 + 3]]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh] = h;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let temp1 = hh.wrapping_add(s1).wrapping_add(ch).wrapping_add(K[i]).wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let temp2 = s0.wrapping_add(maj);
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(temp1);
            d = c;
            c = b;
            b = a;
            a = temp1.wrapping_add(temp2);
        }
        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e);
        h[5] = h[5].wrapping_add(f);
        h[6] = h[6].wrapping_add(g);
        h[7] = h[7].wrapping_add(hh);
    }
    let mut out = [0u8; 32];
    for (i, word) in h.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&word.to_be_bytes());
    }
    out
}

pub fn generate_session_token() -> String {
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

    #[test]
    fn verifies_pkce_s256() {
        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        let challenge = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM";
        assert!(verify_pkce(challenge, "S256", verifier));
        assert!(!verify_pkce(challenge, "S256", "wrong-verifier"));
    }
}
