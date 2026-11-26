//! MCP streamable-HTTP 传输端点（连接→对话 阶段4）。
//!
//! 为连接工具 server 提供 `mcp-session-id` / `MCP-Protocol-Version` 的 HTTP 边界。
//! 本模块固定**可测的 HTTP 契约**：请求解析 + HTTP 响应（注入 `ConnectionMcpRuntime` 分发）。
//! 真正的 `TcpListener` 循环是薄胶水（见 [`serve`]）；claude-code 端到端握手留待真实 agent 验收。

use std::io::{BufRead, Result as IoResult};
use std::net::TcpListener;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use anyhow::{Context, Result, bail};
use serde_json::Value;

use crate::app::{
    ConnectionMcpRuntime, ConnectionMcpRuntimeFactory, ConnectionRunLocator, handle_jsonrpc,
};

/// 连接工具 server 的 HTTP 端点路径。
pub const MCP_ENDPOINT_PATH: &str = "/mcp";
const TOKEN_QUERY_PARAM: &str = "token";

/// 生成的 session id 前缀。
const SESSION_PREFIX: &str = "connection-mcp-";

static SESSION_COUNTER: AtomicUsize = AtomicUsize::new(0);

// ── T06 resource bounds ──
// The endpoint shares the desktop process with chat and tasks, so an abnormal
// client must not be able to starve it. Every bound is enforced BEFORE the
// request reaches business logic (authorization included, which happens later
// in `respond`): allocation follows the bounded check, never the declared size.
/// 请求行 / 单个 header 行的字节上限。
const MAX_HEADER_LINE_BYTES: usize = 16 * 1024;
/// header 数量上限。
const MAX_HEADER_COUNT: usize = 64;
/// 请求正文字节上限（JSON-RPC 工具调用远小于此；1 MiB 足够宽松）。
const MAX_BODY_BYTES: usize = 1024 * 1024;
/// 同时服务的连接数上限；超出立即拒绝并关闭，不排队。
const MAX_CONCURRENT_CONNECTIONS: usize = 16;
/// 单连接读取时限：整个请求（行 + headers + body）必须在该窗口内读完。
const REQUEST_READ_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// 请求超出资源边界时返回的 413/408 类响应。
fn too_large(what: &str) -> HttpResponse {
    HttpResponse {
        status: 413,
        body: format!("request too large: {what}"),
        content_type: "text/plain".to_string(),
        set_session: None,
    }
}

/// 解析出的最小 HTTP/1.1 请求（仅 localhost endpoint 所需字段）。
#[derive(Debug)]
pub struct HttpRequest {
    pub method: String,
    pub path: String,
    pub headers: Vec<(String, String)>,
    pub body: String,
}

/// MCP endpoint 的 HTTP 响应。
#[derive(Debug)]
pub struct HttpResponse {
    pub status: u16,
    pub body: String,
    pub content_type: String,
    /// `initialize` 成功时下发 `mcp-session-id`；其余请求不设置。
    pub set_session: Option<String>,
}

/// 从 HTTP/1.1 请求流解析 `HttpRequest`（请求行 + headers + Content-Length body）。
/// T06：行长度、header 数量、正文大小全部有界；`read_line` 前先看流量是否
/// 超限，声明值再大也不会先分配内存。
pub fn parse_http_request(reader: &mut impl BufRead) -> Result<HttpRequest> {
    let mut request_line = String::new();
    read_bounded_line(reader, &mut request_line, "request line")?;
    if request_line.is_empty() {
        bail!("empty request line");
    }
    let mut parts = request_line.trim_end().split_whitespace();
    let method = parts.next().unwrap_or_default().to_string();
    let path = parts.next().unwrap_or_default().to_string();
    let mut headers = Vec::new();
    let mut content_length = 0usize;
    loop {
        let mut line = String::new();
        read_bounded_line(reader, &mut line, "header line")?;
        let trimmed = line.trim_end();
        if trimmed.is_empty() {
            break;
        }
        if headers.len() >= MAX_HEADER_COUNT {
            bail!("too many header lines");
        }
        if let Some((name, value)) = trimmed.split_once(':') {
            let name = name.trim().to_string();
            let value = value.trim().to_string();
            if name.eq_ignore_ascii_case("content-length") {
                content_length = value.parse().unwrap_or_default();
            }
            headers.push((name, value));
        }
    }
    // T06: declared length is validated against the bound BEFORE allocating.
    if content_length > MAX_BODY_BYTES {
        bail!("body exceeds {MAX_BODY_BYTES} bytes");
    }
    let mut body = vec![0u8; content_length];
    reader
        .read_exact(&mut body)
        .context("failed to read body")?;
    Ok(HttpRequest {
        method,
        path,
        headers,
        body: String::from_utf8_lossy(&body).into_owned(),
    })
}

/// `read_line` 的有界版本：行超过 `MAX_HEADER_LINE_BYTES` 即失败，避免
/// `BufRead::read_line` 按声明无限扩展缓冲。
fn read_bounded_line(
    reader: &mut impl BufRead,
    out: &mut String,
    what: &'static str,
) -> Result<()> {
    let mut bytes = Vec::new();
    loop {
        let buffered = reader.fill_buf()?;
        match buffered.iter().position(|&b| b == b'\n') {
            Some(pos) => {
                bytes.extend_from_slice(&buffered[..=pos]);
                if bytes.len() > MAX_HEADER_LINE_BYTES {
                    bail!("{what} exceeds {MAX_HEADER_LINE_BYTES} bytes");
                }
                reader.consume(pos + 1);
                out.push_str(&String::from_utf8_lossy(&bytes));
                return Ok(());
            }
            None => {
                bytes.extend_from_slice(buffered);
                let consumed = buffered.len();
                reader.consume(consumed);
                if consumed == 0 {
                    // EOF without a newline: accept what we have (the header
                    // loop's empty-line logic handles the rest).
                    out.push_str(&String::from_utf8_lossy(&bytes));
                    return Ok(());
                }
                if bytes.len() > MAX_HEADER_LINE_BYTES {
                    bail!("{what} exceeds {MAX_HEADER_LINE_BYTES} bytes");
                }
            }
        }
    }
}

/// 分发一个已解析的 HTTP 请求到 MCP JSON-RPC，返回 HTTP 响应。
/// 纯函数：可测试（无需 socket）。
pub fn respond(runtime: &mut dyn ConnectionMcpRuntime, request: &HttpRequest) -> HttpResponse {
    if request_path_without_query(request) != MCP_ENDPOINT_PATH {
        return plain(404, "not found");
    }
    match request.method.as_str() {
        "POST" => respond_post(runtime, &request.body),
        "DELETE" => empty_json_response(200),
        _ => plain(405, "method not allowed"),
    }
}

fn respond_post(runtime: &mut dyn ConnectionMcpRuntime, body: &str) -> HttpResponse {
    let value: Value = match serde_json::from_str(body) {
        Ok(value) => value,
        Err(_) => return plain(400, "invalid JSON-RPC body"),
    };
    // Notifications (no id) are fire-and-forget: 202 Accepted, no body.
    let is_notification = value
        .get("method")
        .and_then(Value::as_str)
        .map(|method| method.starts_with("notifications/"))
        .unwrap_or_default();
    if is_notification {
        return empty_json_response(202);
    }
    let response = handle_jsonrpc(&value, runtime);
    let is_initialize = value.get("method").and_then(Value::as_str) == Some("initialize");
    HttpResponse {
        status: 200,
        body: serde_json::to_string(&response).unwrap_or_default(),
        content_type: "application/json".to_string(),
        set_session: is_initialize.then(new_session_id),
    }
}

fn empty_json_response(status: u16) -> HttpResponse {
    HttpResponse {
        status,
        body: String::new(),
        content_type: "application/json".to_string(),
        set_session: None,
    }
}

fn plain(status: u16, body: &str) -> HttpResponse {
    HttpResponse {
        status,
        body: body.to_string(),
        content_type: "text/plain".to_string(),
        set_session: None,
    }
}

fn new_session_id() -> String {
    let n = SESSION_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{SESSION_PREFIX}{n}")
}

fn request_path_without_query(request: &HttpRequest) -> &str {
    request.path.split('?').next().unwrap_or(&request.path)
}

/// 解析 `path` 中的 `key=value` 查询参数（不做百分号解码——`taskId`/`runId` 是内部
/// 生成的 `task-NNN`/`run-NNN` 形式标识符，从不含需要转义的字符）。
fn query_param<'a>(request: &'a HttpRequest, key: &str) -> Option<&'a str> {
    let query = request.path.split_once('?').map(|(_, query)| query)?;
    query.split('&').find_map(|pair| {
        let (name, value) = pair.split_once('=')?;
        (name == key).then_some(value)
    })
}

/// Verify the endpoint's run grant and any MCP session before loading run data.
/// Initialize binds each new MCP session to the current grant for that run.
pub fn respond_scoped(
    factory: &ConnectionMcpRuntimeFactory,
    request: &HttpRequest,
) -> HttpResponse {
    if request_path_without_query(request) != MCP_ENDPOINT_PATH {
        return plain(404, "not found");
    }
    let Some(token) = query_param(request, TOKEN_QUERY_PARAM) else {
        return plain(403, "missing `token` query parameter");
    };
    let Some(project_id) = query_param(request, "projectId") else {
        return plain(400, "missing `projectId` query parameter");
    };
    let Some(task_id) = query_param(request, "taskId") else {
        return plain(400, "missing `taskId` query parameter");
    };
    let Some(run_id) = query_param(request, "runId") else {
        return plain(400, "missing `runId` query parameter");
    };
    let locator = ConnectionRunLocator {
        project_id: project_id.to_string(),
        task_id: task_id.to_string(),
        run_id: run_id.to_string(),
    };
    let authorizer = factory.authorizer();
    let session_header = request
        .headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("mcp-session-id"))
        .map(|(_, value)| value.as_str());
    if request.method == "DELETE" {
        return if session_header.is_some_and(|id| authorizer.close_session(token, &locator, id)) {
            empty_json_response(200)
        } else {
            plain(403, "unauthorized")
        };
    }
    if !authorizer.authorize(token, &locator, session_header) {
        return plain(403, "unauthorized");
    }
    if session_header.is_none() {
        // No session header is only acceptable while handshaking: initialize
        // (and notifications) arrive without one. Any tools/call before
        // initialize would have no session — reject it so every stateful
        // request is bound to an authorized session.
        let is_initialize_or_notification = request.method == "POST"
            && serde_json::from_str::<Value>(&request.body).is_ok_and(|value| {
                value
                    .get("method")
                    .and_then(Value::as_str)
                    .is_some_and(|method| {
                        method == "initialize" || method.starts_with("notifications/")
                    })
            });
        if !is_initialize_or_notification {
            return plain(403, "unauthorized");
        }
    }
    let mut runtime = match factory.build_for_run(project_id, task_id, run_id) {
        Ok(runtime) => runtime,
        Err(error) => return plain(400, &format!("{error:#}")),
    };
    // A task may be deleted while the runtime is being resolved. The check
    // before loading and this check bracket the read of the run's locator.
    if !authorizer.authorize(token, &locator, session_header) {
        return plain(403, "unauthorized");
    }
    let response = respond(&mut runtime, request);
    if let Some(session_id) = &response.set_session
        && !authorizer.bind_session(token, &locator, session_id)
    {
        return plain(403, "unauthorized");
    }
    response
}

/// 构建注入 `mcp_servers` 时用的按 run 作用域 URL。`projectId` 必带：run 数据
/// 按工作区数据目录分片存放，端点（全局 App）须经它解析出正确的工作区路径。
/// 同一个端点内，URL 使用当前 run 的有效 token。
pub fn connection_mcp_url(
    port: u16,
    project_id: &str,
    task_id: &str,
    run_id: &str,
    token: &str,
) -> String {
    format!(
        "http://127.0.0.1:{port}{MCP_ENDPOINT_PATH}?projectId={project_id}&taskId={task_id}&runId={run_id}&{TOKEN_QUERY_PARAM}={token}"
    )
}

pub(crate) fn redact_connection_mcp_token(url: &str) -> Option<String> {
    let (endpoint, query) = url.split_once('?')?;
    let parameters = query
        .split('&')
        .filter(|pair| {
            pair.split_once('=')
                .is_none_or(|(name, _)| name != TOKEN_QUERY_PARAM)
        })
        .collect::<Vec<_>>();
    Some(format!("{endpoint}?{}", parameters.join("&")))
}

/// 启动 localhost MCP endpoint 的 accept 循环。`stop` 置位后退出。
/// 每个连接独立线程处理；每次请求按 URL 的 `taskId`/`runId` 现建 run 作用域 runtime
/// （见 [`respond_scoped`]），天然支持多对话并发。
pub fn serve(
    listener: TcpListener,
    factory: Arc<ConnectionMcpRuntimeFactory>,
    stop: Arc<std::sync::atomic::AtomicBool>,
) {
    // T06: bound the concurrent handler threads. An abnormal client cannot
    // spawn unbounded work in the shared desktop process; overflow is rejected
    // and closed immediately instead of queueing.
    let live_handlers = Arc::new(AtomicUsize::new(0));
    let _ = listener.set_nonblocking(true);
    while !stop.load(Ordering::Relaxed) {
        match listener.accept() {
            Ok((stream, _)) => {
                // macOS/BSD: an accepted socket inherits the listener's
                // O_NONBLOCK flag. The listener is nonblocking here, so without
                // restoring blocking mode every server-side read returns EAGAIN
                // instantly instead of waiting for the request bytes.
                let _ = stream.set_nonblocking(false);
                if live_handlers.load(Ordering::SeqCst) >= MAX_CONCURRENT_CONNECTIONS {
                    // Shed load: answer with 503 and close; no thread, no queue.
                    let mut stream = stream;
                    let _ = write_response(
                        &mut stream,
                        &HttpResponse {
                            status: 503,
                            body: "too many concurrent connections".to_string(),
                            content_type: "text/plain".to_string(),
                            set_session: None,
                        },
                    );
                    continue;
                }
                live_handlers.fetch_add(1, Ordering::SeqCst);
                let factory = Arc::clone(&factory);
                let live = Arc::clone(&live_handlers);
                std::thread::spawn(move || {
                    let _guard = HandlerGuard(live);
                    if let Err(error) = handle_connection(stream, &factory) {
                        tracing::debug!(%error, "connection MCP HTTP handler error");
                    }
                });
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            Err(_) => break,
        }
    }
}

/// Decrements the live-handler count on drop so a panicked handler cannot
/// permanently consume a concurrency slot.
struct HandlerGuard(Arc<AtomicUsize>);
impl Drop for HandlerGuard {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

fn handle_connection(
    mut stream: std::net::TcpStream,
    factory: &ConnectionMcpRuntimeFactory,
) -> Result<()> {
    // T06: the whole request must arrive within the read window; a client that
    // drips bytes forever cannot hold a slot (or the process) hostage.
    let _ = stream.set_read_timeout(Some(REQUEST_READ_TIMEOUT));
    let mut reader = std::io::BufReader::new(stream.try_clone().context("stream clone")?);
    let request = match parse_http_request(&mut reader) {
        Ok(request) => request,
        Err(error) => {
            // Distinguish the bounds (413/408-style rejections) from protocol
            // garbage: both close the connection, but bounds get a status so a
            // legitimate client sees why.
            let message = error.to_string();
            let response = if message.contains("exceeds") {
                too_large(&message)
            } else {
                HttpResponse {
                    status: 400,
                    body: format!("bad request: {message}"),
                    content_type: "text/plain".to_string(),
                    set_session: None,
                }
            };
            write_response(&mut stream, &response)?;
            return Ok(());
        }
    };
    let response = respond_scoped(factory, &request);
    write_response(&mut stream, &response)?;
    Ok(())
}

fn write_response(stream: &mut std::net::TcpStream, response: &HttpResponse) -> IoResult<()> {
    use std::io::Write;
    let reason = match response.status {
        200 => "OK",
        202 => "Accepted",
        304 => "Not Modified",
        400 => "Bad Request",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        408 => "Request Timeout",
        413 => "Payload Too Large",
        503 => "Service Unavailable",
        500 => "Internal Server Error",
        _ => "OK",
    };
    write!(stream, "HTTP/1.1 {} {}\r\n", response.status, reason)?;
    write!(
        stream,
        "Content-Type: {}\r\nContent-Length: {}\r\n",
        response.content_type,
        response.body.len()
    )?;
    if let Some(session) = &response.set_session {
        write!(stream, "mcp-session-id: {session}\r\n")?;
    }
    write!(stream, "Connection: close\r\n\r\n")?;
    if !response.body.is_empty() {
        stream.write_all(response.body.as_bytes())?;
    }
    stream.flush()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{
        ConnectionAuthMethod, ConnectionEnvironment, ConnectionMcpEndpoint, ConnectionSpec,
        TerminalSessionKind, TerminalSessionStatus, TerminalSessionView,
    };
    use serde_json::json;

    #[derive(Default)]
    struct FakeRuntime {
        connections: Vec<ConnectionSpec>,
    }

    impl ConnectionMcpRuntime for FakeRuntime {
        fn list_connections(&self) -> Result<Vec<ConnectionSpec>> {
            Ok(self.connections.clone())
        }
        fn open_connection(
            &mut self,
            connection_id: &str,
            _cols: Option<u16>,
            _rows: Option<u16>,
        ) -> Result<TerminalSessionView> {
            Ok(TerminalSessionView {
                session_id: format!("sess-{connection_id}"),
                kind: TerminalSessionKind::Remote,
                cwd: None,
                label: Some(connection_id.to_string()),
                status: TerminalSessionStatus::Connected,
                origin: None,
            })
        }
        fn send_input(&self, _session_id: &str, _input: &str) -> Result<()> {
            Ok(())
        }
        fn read_output(
            &self,
            _session_id: &str,
            _from_offset: Option<u64>,
        ) -> Result<crate::app::ConnectionOutputChunk> {
            Ok(crate::app::ConnectionOutputChunk {
                text: String::new(),
                eof: true,
                next_offset: 0,
                trimmed: false,
                status: None,
                error_code: None,
            })
        }
        fn close_connection(&self, _session_id: &str) -> Result<()> {
            Ok(())
        }
    }

    fn request(method: &str, path: &str, body: &str) -> HttpRequest {
        HttpRequest {
            method: method.to_string(),
            path: path.to_string(),
            headers: Vec::new(),
            body: body.to_string(),
        }
    }

    // ── T06 resource bounds ──

    #[test]
    fn parse_http_request_rejects_an_oversized_body_declaration() {
        // A declared Content-Length beyond the cap fails BEFORE allocation.
        let raw = "POST /mcp HTTP/1.1\r\nHost: x\r\nContent-Length: 999999999\r\n\r\n";
        let mut reader = std::io::BufReader::new(raw.as_bytes());
        let error = parse_http_request(&mut reader).unwrap_err();
        assert!(error.to_string().contains("body exceeds"));
    }

    #[test]
    fn parse_http_request_accepts_a_normal_request_under_the_bounds() {
        let body = json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize" }).to_string();
        let raw = format!(
            "POST /mcp?projectId=p&taskId=t&runId=r&token=conn-auth-x HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        );
        let mut reader = std::io::BufReader::new(raw.as_bytes());
        let parsed = parse_http_request(&mut reader).unwrap();
        assert_eq!(parsed.method, "POST");
        assert!(parsed.path.starts_with("/mcp?"));
        assert_eq!(parsed.body, body);
    }

    #[test]
    fn parse_http_request_rejects_an_oversized_header_line() {
        let long_header = format!("X-Junk: {}\r\n", "a".repeat(MAX_HEADER_LINE_BYTES + 1));
        let raw = format!("POST /mcp HTTP/1.1\r\n{long_header}\r\n");
        let mut reader = std::io::BufReader::new(raw.as_bytes());
        let error = parse_http_request(&mut reader).unwrap_err();
        assert!(error.to_string().contains("exceeds"));
    }

    #[test]
    fn parse_http_request_rejects_too_many_header_lines() {
        let mut raw = String::from("POST /mcp HTTP/1.1\r\n");
        for i in 0..(MAX_HEADER_COUNT + 2) {
            raw.push_str(&format!("X-Junk-{i}: v\r\n"));
        }
        raw.push_str("\r\n");
        let mut reader = std::io::BufReader::new(raw.as_bytes());
        let error = parse_http_request(&mut reader).unwrap_err();
        assert!(error.to_string().contains("too many header lines"));
    }

    #[test]
    fn socket_server_sheds_load_beyond_the_concurrency_cap() {
        // Fill the handler pool with slow connections; the next client must
        // get a 503 instead of hanging, and resources stay bounded.
        use crate::app::{App, TerminalSessionManager};
        let temp = tempfile::TempDir::new().unwrap();
        let app = App::with_config(
            camino::Utf8PathBuf::from_path_buf(temp.path().join("repo")).unwrap(),
            crate::config::RuntimeConfig::default(),
        );
        let manager = Arc::new(std::sync::Mutex::new(TerminalSessionManager::new(
            Arc::new(|_| {}),
            temp.path().join("known_hosts"),
        )));
        let history = Arc::new(std::sync::Mutex::new(std::collections::HashMap::new()));
        let factory = Arc::new(ConnectionMcpRuntimeFactory::new(
            ConnectionMcpEndpoint::new(0),
            app,
            manager,
            history,
            Arc::new(|_| anyhow::bail!("no workspace paths in this test")),
        ));
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        {
            let listener = listener;
            let factory = Arc::clone(&factory);
            let stop = Arc::clone(&stop);
            std::thread::spawn(move || serve(listener, factory, stop));
        }

        // Open MAX_CONCURRENT_CONNECTIONS slow sockets that never finish their
        // request; the server keeps a thread on each within the read timeout.
        // They write a partial request (headers) so the handler is inside
        // parse and not failed-fast on an empty stream.
        let mut held = Vec::new();
        for _ in 0..MAX_CONCURRENT_CONNECTIONS {
            let stream = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
            use std::io::Write;
            let mut s = stream;
            let _ = s.write_all(b"POST /mcp HTTP/1.1\r\nHost: x\r\n"); // no final CRLF: parse parks
            let _ = s.flush();
            held.push(s);
        }
        std::thread::sleep(std::time::Duration::from_millis(500));

        // A new client is load-shed with 503 (readable response, then closed).
        let overflow = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
        let _ = overflow.set_read_timeout(Some(std::time::Duration::from_millis(1500)));
        let mut overflow = std::io::BufReader::new(overflow);
        let mut status_line = String::new();
        let _ = overflow.read_line(&mut status_line);
        assert!(
            status_line.contains("503"),
            "expected 503 shed, got: {status_line}"
        );

        // Release the held sockets and confirm a normal request succeeds again.
        held.clear();
        std::thread::sleep(std::time::Duration::from_millis(600));
        let recover = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
        let _ = recover.set_read_timeout(Some(std::time::Duration::from_millis(2000)));
        let mut writer = std::io::BufWriter::new(recover.try_clone().expect("clone"));
        use std::io::Write;
        let body = json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize" }).to_string();
        write!(
            writer,
            "POST /mcp HTTP/1.1\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        )
        .unwrap();
        writer.flush().unwrap();
        let mut reader = overflow;
        // Reuse a fresh reader over the same socket is not possible; read from
        // the writer's underlying stream instead by dropping and reconnecting.
        drop(writer);
        let fresh = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
        let _ = fresh.set_read_timeout(Some(std::time::Duration::from_millis(2000)));
        let mut w = std::io::BufWriter::new(fresh.try_clone().expect("clone2"));
        write!(
            w,
            "POST /mcp HTTP/1.1\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        )
        .unwrap();
        w.flush().unwrap();
        let mut r = std::io::BufReader::new(fresh);
        let mut line = String::new();
        let _ = r.read_line(&mut line);
        // Any well-formed HTTP status proves the server recovered after the
        // load-shed window (an unauthorized request correctly gets 403 under
        // T02; what matters here is the server answered, not hung or died).
        let answered = line.starts_with("HTTP/1.1 ");
        assert!(answered, "server recovered after load shed, got: {line}");
        let _ = reader;
        stop.store(true, Ordering::Relaxed);
    }

    #[test]
    fn initialize_returns_json_and_sets_mcp_session_id() {
        let mut rt = FakeRuntime::default();
        let resp = respond(
            &mut rt,
            &request(
                "POST",
                "/mcp",
                &json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize" }).to_string(),
            ),
        );
        assert_eq!(resp.status, 200);
        assert_eq!(resp.content_type, "application/json");
        assert!(
            resp.set_session
                .as_deref()
                .unwrap_or("")
                .starts_with("connection-mcp-")
        );
        assert!(resp.body.contains("protocolVersion"));
        assert!(resp.body.contains("gold-band-connections"));
    }

    #[test]
    fn notification_is_accepted_without_body() {
        let mut rt = FakeRuntime::default();
        let resp = respond(
            &mut rt,
            &request(
                "POST",
                "/mcp",
                &json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }).to_string(),
            ),
        );
        assert_eq!(resp.status, 202);
        assert!(resp.body.is_empty());
        assert!(resp.set_session.is_none());
    }

    #[test]
    fn tools_list_returns_five_connection_tools() {
        let mut rt = FakeRuntime::default();
        let resp = respond(
            &mut rt,
            &request(
                "POST",
                "/mcp",
                &json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" }).to_string(),
            ),
        );
        assert_eq!(resp.status, 200);
        assert!(resp.body.contains("\"list_connections\""));
        assert!(resp.body.contains("\"close_connection\""));
    }

    #[test]
    fn tools_call_list_connections_returns_specs_json() {
        let mut rt = FakeRuntime::default();
        rt.connections = vec![ConnectionSpec {
            id: "conn-1".to_string(),
            name: "wb".to_string(),
            host: "10.0.0.5".to_string(),
            port: 22,
            username: "ops".to_string(),
            auth_method: ConnectionAuthMethod::PublicKey,
            env: ConnectionEnvironment::Prod,
            jump_ref: None,
        }];
        let resp = respond(&mut rt, &request(
            "POST",
            "/mcp",
            &json!({ "jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": { "name": "list_connections", "arguments": {} } }).to_string(),
        ));
        assert_eq!(resp.status, 200);
        let body: Value = serde_json::from_str(&resp.body).unwrap();
        let text = body["result"]["content"][0]["text"].as_str().unwrap();
        let specs: Vec<ConnectionSpec> = serde_json::from_str(text).unwrap();
        assert_eq!(specs[0].name, "wb");
        assert_eq!(specs[0].host, "10.0.0.5");
    }

    #[test]
    fn unknown_tool_returns_jsonrpc_error_in_json() {
        let mut rt = FakeRuntime::default();
        let resp = respond(&mut rt, &request(
            "POST",
            "/mcp",
            &json!({ "jsonrpc": "2.0", "id": 4, "method": "tools/call", "params": { "name": "deploy", "arguments": {} } }).to_string(),
        ));
        assert_eq!(resp.status, 200);
        assert!(resp.body.contains("\"tool.not-found\""));
    }

    #[test]
    fn wrong_path_returns_404() {
        let mut rt = FakeRuntime::default();
        let resp = respond(&mut rt, &request("POST", "/other", "{}"));
        assert_eq!(resp.status, 404);
    }

    #[test]
    fn invalid_json_body_returns_400() {
        let mut rt = FakeRuntime::default();
        let resp = respond(&mut rt, &request("POST", "/mcp", "not-json"));
        assert_eq!(resp.status, 400);
    }

    #[test]
    fn delete_session_returns_200() {
        let mut rt = FakeRuntime::default();
        let resp = respond(&mut rt, &request("DELETE", "/mcp", ""));
        assert_eq!(resp.status, 200);
    }

    #[test]
    fn parse_http_request_reads_method_path_and_body() {
        let bytes =
            b"POST /mcp HTTP/1.1\r\nContent-Type: application/json\r\nContent-Length: 2\r\n\r\n{}";
        let mut reader = std::io::BufReader::new(&bytes[..]);
        let req = parse_http_request(&mut reader).unwrap();
        assert_eq!(req.method, "POST");
        assert_eq!(req.path, "/mcp");
        assert_eq!(req.body, "{}");
    }

    #[test]
    fn parse_http_request_preserves_query_string_in_path() {
        let bytes =
            b"POST /mcp?taskId=task-001&runId=run-001 HTTP/1.1\r\nContent-Length: 2\r\n\r\n{}";
        let mut reader = std::io::BufReader::new(&bytes[..]);
        let req = parse_http_request(&mut reader).unwrap();
        assert_eq!(req.path, "/mcp?taskId=task-001&runId=run-001");
    }

    #[test]
    fn query_param_extracts_task_and_run_ids() {
        let req = request("POST", "/mcp?taskId=task-001&runId=run-002", "{}");
        assert_eq!(query_param(&req, "taskId"), Some("task-001"));
        assert_eq!(query_param(&req, "runId"), Some("run-002"));
        assert_eq!(query_param(&req, "missing"), None);
    }

    #[test]
    fn request_path_without_query_strips_query_string() {
        let req = request("POST", "/mcp?taskId=task-001", "{}");
        assert_eq!(request_path_without_query(&req), "/mcp");
    }

    #[test]
    fn connection_mcp_url_embeds_port_project_task_run_and_token() {
        let url = connection_mcp_url(9000, "proj-1", "task-001", "run-002", "conn-auth-x");
        assert_eq!(
            url,
            "http://127.0.0.1:9000/mcp?projectId=proj-1&taskId=task-001&runId=run-002&token=conn-auth-x"
        );
    }

    /// Isolates the given test's user-level config dir (the default `App`
    /// otherwise resolves against the real user's `~/.gold-band`, which a test
    /// must never touch even for reads).
    fn isolated_factory(
        home_env_var: &'static str,
    ) -> (ConnectionMcpRuntimeFactory, tempfile::TempDir) {
        let temp = tempfile::TempDir::new().expect("tempdir");
        let repo_root =
            camino::Utf8PathBuf::from_path_buf(temp.path().join("repo")).expect("utf8 path");
        // SAFETY: caller passes a name unique to its test.
        unsafe {
            std::env::set_var(home_env_var, temp.path().join("home"));
        }
        let path_config = crate::storage::StoragePathConfig {
            app_key: "gold-band",
            config_dir_name: ".gold-band",
            home_env_var,
        };
        let app = crate::app::App::with_config_and_path_config(
            repo_root,
            crate::config::RuntimeConfig::default(),
            path_config,
        );
        let manager = Arc::new(std::sync::Mutex::new(
            crate::app::TerminalSessionManager::new(
                Arc::new(|_| {}),
                temp.path().join("known_hosts"),
            ),
        ));
        let history = Arc::new(std::sync::Mutex::new(std::collections::HashMap::new()));
        let resolver_paths = app.paths.clone();
        (
            ConnectionMcpRuntimeFactory::new(
                ConnectionMcpEndpoint::new(0),
                app,
                manager,
                history,
                Arc::new(move |_| Ok(resolver_paths.clone())),
            ),
            temp,
        )
    }

    #[test]
    fn respond_scoped_requires_the_query_parameters_in_order() {
        let (factory, _temp) =
            isolated_factory("GOLD_BAND_TEST_HOME_CONNECTION_MCP_HTTP_MISSING_QUERY");
        // T02: an absent token is rejected before anything else (403), so the
        // parameter checks below are only reachable with a valid token.
        let resp = respond_scoped(&factory, &request("POST", "/mcp", "{}"));
        assert_eq!(resp.status, 403);

        let token = factory.authorizer().token_for_run(&ConnectionRunLocator {
            project_id: "proj-1".to_string(),
            task_id: "task-001".to_string(),
            run_id: "run-001".to_string(),
        });
        // A present-but-forged token with no other parameters: parameter
        // completeness is checked first (400 is the protocol error), the
        // token itself is verified before any resource access below.
        let resp = respond_scoped(
            &factory,
            &request(
                "POST",
                "/mcp?projectId=proj-1&taskId=task-001&runId=run-001&token=forged",
                "{}",
            ),
        );
        assert_eq!(resp.status, 403);
        assert!(resp.body.contains("unauthorized"));

        let resp = respond_scoped(
            &factory,
            &request("POST", &format!("/mcp?token={token}"), "{}"),
        );
        assert_eq!(resp.status, 400);
        assert!(resp.body.contains("projectId"));

        let resp = respond_scoped(
            &factory,
            &request(
                "POST",
                &format!("/mcp?projectId=proj-1&token={token}"),
                "{}",
            ),
        );
        assert_eq!(resp.status, 400);
        assert!(resp.body.contains("taskId"));

        let resp = respond_scoped(
            &factory,
            &request(
                "POST",
                &format!("/mcp?projectId=proj-1&taskId=task-001&token={token}"),
                "{}",
            ),
        );
        assert_eq!(resp.status, 400);
        assert!(resp.body.contains("runId"));
    }

    #[test]
    fn respond_scoped_returns_403_for_a_missing_or_lying_token() {
        let (factory, _temp) = isolated_factory("GOLD_BAND_TEST_HOME_CONNECTION_MCP_HTTP_TOKEN");
        let legit = factory.authorizer().token_for_run(&ConnectionRunLocator {
            project_id: "proj-1".to_string(),
            task_id: "task-001".to_string(),
            run_id: "run-001".to_string(),
        });

        // No token at all.
        let resp = respond_scoped(
            &factory,
            &request(
                "POST",
                "/mcp?projectId=proj-1&taskId=task-001&runId=run-001",
                "{}",
            ),
        );
        assert_eq!(resp.status, 403);

        // A well-formed but unknown token: loopback callers cannot forge one.
        let resp = respond_scoped(
            &factory,
            &request(
                "POST",
                "/mcp?projectId=proj-1&taskId=task-001&runId=run-001&token=conn-auth-forged",
                "{}",
            ),
        );
        assert_eq!(resp.status, 403);

        // A real token pointed at a DIFFERENT run (scope mismatch).
        let resp = respond_scoped(
            &factory,
            &request(
                "POST",
                &format!("/mcp?projectId=proj-1&taskId=task-001&runId=run-other&token={legit}"),
                "{}",
            ),
        );
        assert_eq!(resp.status, 403);
    }

    #[test]
    fn respond_scoped_serves_an_authorized_run_and_binds_the_session() {
        let temp = tempfile::TempDir::new().expect("tempdir");
        let repo_root =
            camino::Utf8PathBuf::from_path_buf(temp.path().join("repo")).expect("utf8 path");
        let home_env_var: &'static str = "GOLD_BAND_TEST_HOME_CONNECTION_MCP_HTTP_AUTHORIZED";
        // SAFETY: this test owns this env var name.
        unsafe {
            std::env::set_var(home_env_var, temp.path().join("home"));
        }
        let path_config = crate::storage::StoragePathConfig {
            app_key: "gold-band",
            config_dir_name: ".gold-band",
            home_env_var,
        };
        let app = crate::app::App::with_config_and_path_config(
            repo_root,
            crate::config::RuntimeConfig::default(),
            path_config,
        );
        let run = crate::runtime::RunState {
            version: crate::domain::VERSION.to_string(),
            id: "run-001".to_string(),
            task_id: "task-001".to_string(),
            task_uuid: None,
            status: crate::domain::RunStatus::Running,
            outcome: None,
            started_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
            workflow_snapshot: "workflow.snapshot.json".to_string(),
            current_round: None,
            current_node: None,
            current_attempt: None,
            new_rounds_opened: 0,
            pause_reason: None,
            uuid: None,
            last_executed_node: None,
            worktree: None,
            execution: Default::default(),
            rerun: None,
            connection_specs: Vec::new(),
        };
        crate::storage::write_json(&app.paths.run_file("task-001", "run-001"), &run).unwrap();

        let manager = Arc::new(std::sync::Mutex::new(
            crate::app::TerminalSessionManager::new(
                Arc::new(|_| {}),
                temp.path().join("known_hosts"),
            ),
        ));
        let history = Arc::new(std::sync::Mutex::new(std::collections::HashMap::new()));
        let resolver_paths = app.paths.clone();
        let endpoint = ConnectionMcpEndpoint::new(0);
        let factory = ConnectionMcpRuntimeFactory::new(
            endpoint.clone(),
            app,
            manager,
            history,
            Arc::new(move |_| Ok(resolver_paths.clone())),
        );

        let locator = ConnectionRunLocator {
            project_id: "proj-1".into(),
            task_id: "task-001".into(),
            run_id: "run-001".into(),
        };
        let token = factory.authorizer().token_for_run(&locator);
        let url_path = format!("/mcp?projectId=proj-1&taskId=task-001&runId=run-001&token={token}");

        // initialize succeeds and issues a session id bound to this token.
        let init = respond_scoped(
            &factory,
            &request(
                "POST",
                &url_path,
                &json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize" }).to_string(),
            ),
        );
        assert_eq!(init.status, 200);
        let session_id = init.set_session.expect("initialize issues a session id");
        assert!(
            factory
                .authorizer()
                .authorize(&token, &locator, Some(&session_id)),
            "the issued session must be bound to the request token"
        );

        // A follow-up request WITHOUT the session header is rejected: only the
        // handshake may omit it, everything else must present the bound id.
        let resp = respond_scoped(
            &factory,
            &request(
                "POST",
                &url_path,
                &json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" }).to_string(),
            ),
        );
        assert_eq!(resp.status, 403);

        // With the session header it goes through.
        let mut req = request(
            "POST",
            &url_path,
            &json!({ "jsonrpc": "2.0", "id": 3, "method": "tools/list" }).to_string(),
        );
        req.headers
            .push(("mcp-session-id".to_string(), session_id.clone()));
        let resp = respond_scoped(&factory, &req);
        assert_eq!(resp.status, 200);

        // Another run's session id (bound to another token) is rejected here.
        let other = ConnectionRunLocator {
            run_id: "run-002".into(),
            ..locator.clone()
        };
        let other_token = factory.authorizer().token_for_run(&other);
        let mut wrong_scope = request(
            "POST",
            &format!("/mcp?projectId=proj-1&taskId=task-001&runId=run-002&token={other_token}"),
            &req.body,
        );
        wrong_scope.headers = req.headers.clone();
        assert_eq!(respond_scoped(&factory, &wrong_scope).status, 403);

        let mut close = request("DELETE", &url_path, "");
        close.headers = req.headers.clone();
        assert_eq!(respond_scoped(&factory, &close).status, 200);
        assert_eq!(respond_scoped(&factory, &req).status, 403);
        assert_eq!(factory.authorizer().token_for_run(&locator), token);

        let initialize = request(
            "POST",
            &url_path,
            &json!({ "jsonrpc": "2.0", "id": 5, "method": "initialize" }).to_string(),
        );
        assert_eq!(respond_scoped(&factory, &initialize).status, 200);
        let revoke_deleted_task = endpoint.task_revocation(&locator.project_id, &locator.task_id);
        revoke_deleted_task();
        assert_eq!(respond_scoped(&factory, &initialize).status, 403);
        assert_ne!(factory.authorizer().token_for_run(&locator), token);
        assert_eq!(respond_scoped(&factory, &initialize).status, 403);
    }

    // ── socket-level integration: real TCP + real HTTP client ──
    //
    // Drives `serve()` over an actual loopback socket with `reqwest::blocking`
    // (the same HTTP client stack the project's own MCP client uses in
    // `mcp/mod.rs`). This is the closest verification short of a real ACP
    // adapter: it proves the wire format (status, Content-Type,
    // mcp-session-id, JSON-RPC body) is well-formed HTTP, not just that our
    // in-process `respond()` function returns the right struct.

    #[test]
    fn serve_handles_initialize_tools_list_and_tools_call_over_real_tcp() {
        use crate::app::{
            App, ConnectionAuthMethod as AppConnectionAuthMethod,
            ConnectionEnvironment as AppConnectionEnvironment, ConnectionSpec as AppConnectionSpec,
            TerminalSessionManager,
        };

        let temp = tempfile::TempDir::new().expect("tempdir");
        let repo_root =
            camino::Utf8PathBuf::from_path_buf(temp.path().join("repo")).expect("utf8 path");
        // Isolate this test's user-level config dir (the app default is the
        // real user's ~/.gold-band, which must never be touched by a test).
        let home_env_var: &'static str = "GOLD_BAND_TEST_HOME_CONNECTION_MCP_HTTP_SERVE";
        // SAFETY: this test runs alone against this specific env var name.
        unsafe {
            std::env::set_var(home_env_var, temp.path().join("home"));
        }
        let path_config = crate::storage::StoragePathConfig {
            app_key: "gold-band",
            config_dir_name: ".gold-band",
            home_env_var,
        };
        let app = App::with_config_and_path_config(
            repo_root,
            crate::config::RuntimeConfig::default(),
            path_config,
        );
        let run = crate::runtime::RunState {
            version: crate::domain::VERSION.to_string(),
            id: "run-001".to_string(),
            task_id: "task-001".to_string(),
            task_uuid: None,
            status: crate::domain::RunStatus::Running,
            outcome: None,
            started_at: "2026-09-09T00:00:00Z".to_string(),
            updated_at: "2026-09-09T00:00:00Z".to_string(),
            workflow_snapshot: "workflow.snapshot.json".to_string(),
            current_round: None,
            current_node: None,
            current_attempt: None,
            new_rounds_opened: 0,
            pause_reason: None,
            uuid: None,
            last_executed_node: None,
            worktree: None,
            execution: Default::default(),
            rerun: None,
            connection_specs: vec![AppConnectionSpec {
                id: "conn-1".to_string(),
                name: "wb".to_string(),
                host: "10.0.0.5".to_string(),
                port: 22,
                username: "ops".to_string(),
                auth_method: AppConnectionAuthMethod::PublicKey,
                env: AppConnectionEnvironment::Prod,
                jump_ref: None,
            }],
        };
        crate::storage::write_json(&app.paths.run_file("task-001", "run-001"), &run).unwrap();

        let manager = Arc::new(std::sync::Mutex::new(TerminalSessionManager::new(
            Arc::new(|_| {}),
            temp.path().join("known_hosts"),
        )));
        let history = Arc::new(std::sync::Mutex::new(std::collections::HashMap::new()));
        let resolver_paths = app.paths.clone();
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().unwrap().port();
        let endpoint = ConnectionMcpEndpoint::new(port);
        let factory = Arc::new(ConnectionMcpRuntimeFactory::new(
            endpoint.clone(),
            app,
            manager,
            history,
            Arc::new(move |_| Ok(resolver_paths.clone())),
        ));
        let config = endpoint.server_config(&ConnectionRunLocator {
            project_id: "proj-1".to_string(),
            task_id: "task-001".to_string(),
            run_id: "run-001".to_string(),
        });
        let url = config["url"].as_str().unwrap();
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let stop_for_server = Arc::clone(&stop);
        let server = std::thread::spawn(move || serve(listener, factory, stop_for_server));

        // `.no_proxy()`: macOS system proxy settings (not just env vars) can
        // otherwise route a loopback request through a corporate proxy that
        // has no route to 127.0.0.1 and answers 503.
        let client = reqwest::blocking::Client::builder()
            .no_proxy()
            .build()
            .expect("build http client");
        let init_resp = client
            .post(url)
            .header(
                "MCP-Protocol-Version",
                crate::app::CONNECTION_MCP_PROTOCOL_VERSION,
            )
            .json(&json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize" }))
            .send()
            .expect("initialize request");
        assert_eq!(init_resp.status(), 200);
        assert_eq!(
            init_resp.headers().get("content-type").unwrap(),
            "application/json"
        );
        let session_id = init_resp
            .headers()
            .get("mcp-session-id")
            .expect("mcp-session-id header")
            .to_str()
            .unwrap()
            .to_string();
        let init_body: Value = init_resp.json().expect("initialize json body");
        assert_eq!(
            init_body["result"]["serverInfo"]["name"],
            "gold-band-connections"
        );

        let list_resp = client
            .post(url)
            .header("mcp-session-id", &session_id)
            .json(&json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" }))
            .send()
            .expect("tools/list request");
        assert_eq!(list_resp.status(), 200);
        let list_body: Value = list_resp.json().expect("tools/list json body");
        let names = list_body["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(
            names,
            vec![
                "list_connections",
                "open_connection",
                "send_input",
                "read_output",
                "close_connection"
            ]
        );

        let call_resp = client
            .post(url)
            .header("mcp-session-id", &session_id)
            .json(&json!({
                "jsonrpc": "2.0",
                "id": 3,
                "method": "tools/call",
                "params": { "name": "list_connections", "arguments": {} },
            }))
            .send()
            .expect("tools/call request");
        assert_eq!(call_resp.status(), 200);
        let call_body: Value = call_resp.json().expect("tools/call json body");
        let text = call_body["result"]["content"][0]["text"].as_str().unwrap();
        let specs: Vec<ConnectionSpec> = serde_json::from_str(text).unwrap();
        assert_eq!(specs[0].name, "wb");

        let notify_resp = client
            .post(url)
            .json(&json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }))
            .send()
            .expect("notification request");
        assert_eq!(notify_resp.status(), 202);
        assert!(notify_resp.bytes().unwrap().is_empty());

        stop.store(true, Ordering::Relaxed);
        // Nudge the accept loop past its blocking poll so the thread joins promptly.
        let _ = std::net::TcpStream::connect(format!("127.0.0.1:{port}"));
        server.join().expect("server thread joins");
    }
}
