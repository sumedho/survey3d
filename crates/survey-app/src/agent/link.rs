//! The agent link: the desktop app serving the Model Context Protocol on a
//! local port, so an agent works on the project on screen and every change
//! shows as it happens.
//!
//! * Transport: MCP's Streamable HTTP, `POST http://127.0.0.1:<port>/mcp`
//!   with one JSON-RPC message, answered with JSON (202 for notifications).
//!   Loopback TCP works the same on Linux, macOS and Windows.
//! * Security: bound to 127.0.0.1 only; the Host header must name the
//!   loopback address and any Origin must be local (against DNS rebinding);
//!   every request carries `Authorization: Bearer <token>`, a random token kept
//!   in a per-user file only the user can read.
//! * Threads: connections are served on their own threads; each request is
//!   handed to the UI thread ([`Link::requests`]), which runs it on the
//!   session on screen ([`serve_on`]) between frames and replies.
//! * Clients that only start stdio servers use `survey3d-mcp --attach`, which
//!   relays stdin/stdout to the link ([`relay`]).

use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::protocol;
use super::server::Server;
use crate::Session;

/// The port the link listens on unless set otherwise.
pub const DEFAULT_PORT: u16 = 3850;

/// Longest a request waits for the app to answer (an image cache can take a while).
const ANSWER_TIMEOUT: Duration = Duration::from_secs(600);
/// Largest request body read.
const MAX_BODY: usize = 64 * 1024 * 1024;

/// Where an agent finds the link: written by the app, read by the relay.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LinkInfo {
    pub port: u16,
    pub token: String,
    pub url: String,
}

/// The per-user link file: `$SURVEY3D_AGENT_LINK`, else in the user's
/// configuration folder (`%APPDATA%\survey3d`, `~/Library/Application
/// Support/survey3d` or `$XDG_CONFIG_HOME/survey3d`).
pub fn info_path() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("SURVEY3D_AGENT_LINK") {
        return Some(PathBuf::from(p));
    }
    let base = if cfg!(windows) {
        PathBuf::from(std::env::var_os("APPDATA")?)
    } else if cfg!(target_os = "macos") {
        PathBuf::from(std::env::var_os("HOME")?).join("Library/Application Support")
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?
    };
    Some(base.join("survey3d").join("agent-link.json"))
}

pub fn read_info() -> Option<LinkInfo> {
    let text = std::fs::read_to_string(info_path()?).ok()?;
    serde_json::from_str(&text).ok()
}

/// The token agents present: kept from the last time (so a client set up
/// with it keeps working), else a new random one.
pub fn token() -> String {
    read_info()
        .map(|i| i.token)
        .filter(|t| t.len() >= 32)
        .unwrap_or_else(random_token)
}

/// 256 random bits as hex, from the operating system's hash seeds.
fn random_token() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    (0..4u64)
        .map(|i| {
            let mut h = RandomState::new().build_hasher();
            h.write_u64(i);
            h.write_u128(now);
            h.write_u32(std::process::id());
            format!("{:016x}", h.finish())
        })
        .collect()
}

/// Writes the link file, readable only by the user where the system allows.
pub fn write_info(info: &LinkInfo) -> io::Result<PathBuf> {
    let path = info_path().ok_or_else(|| io::Error::other("no configuration folder for the link file"))?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let text = serde_json::to_string_pretty(info).map_err(io::Error::other)?;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    options.open(&path)?.write_all(text.as_bytes())?;
    Ok(path)
}

// ====================================================================== the server

/// One message from an agent, for the UI thread to answer.
pub struct Request {
    pub message: Value,
    reply: Sender<Option<Value>>,
}

impl Request {
    /// The reply (`None` for a notification).
    pub fn answer(self, reply: Option<Value>) {
        let _ = self.reply.send(reply);
    }
}

/// The running link.
pub struct Link {
    pub port: u16,
    pub token: String,
    requests: Receiver<Request>,
    stop: Arc<AtomicBool>,
}

impl Drop for Link {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

impl Link {
    /// Listens on 127.0.0.1:`port` (0 for any free port); `wake` is called
    /// when a request arrives, so the UI can run it.
    pub fn start(port: u16, token: String, wake: Arc<dyn Fn() + Send + Sync>) -> io::Result<Self> {
        let listener = TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], port)))?;
        let port = listener.local_addr()?.port();
        listener.set_nonblocking(true)?;
        let (tx, requests) = channel();
        let stop = Arc::new(AtomicBool::new(false));
        let (stop2, token2) = (stop.clone(), token.clone());
        std::thread::Builder::new()
            .name("agent-link".into())
            .spawn(move || {
                while !stop2.load(Ordering::Relaxed) {
                    match listener.accept() {
                        Ok((stream, _)) => {
                            let (tx, token, wake, stop) =
                                (tx.clone(), token2.clone(), wake.clone(), stop2.clone());
                            let _ =
                                std::thread::Builder::new()
                                    .name("agent-link-conn".into())
                                    .spawn(move || {
                                        serve_connection(stream, port, &token, &tx, wake.as_ref(), &stop)
                                    });
                        }
                        Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                            std::thread::sleep(Duration::from_millis(50))
                        }
                        Err(_) => std::thread::sleep(Duration::from_millis(200)),
                    }
                }
            })?;
        Ok(Self {
            port,
            token,
            requests,
            stop,
        })
    }

    pub fn url(&self) -> String {
        format!("http://127.0.0.1:{}/mcp", self.port)
    }

    /// Requests waiting for an answer.
    pub fn requests(&self) -> impl Iterator<Item = Request> + '_ {
        self.requests.try_iter()
    }
}

/// An HTTP request as read.
struct HttpRequest {
    method: String,
    path: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl HttpRequest {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

/// Reads one request; `None` at the end of the connection.
fn read_request(r: &mut impl BufRead) -> io::Result<Option<HttpRequest>> {
    let mut line = String::new();
    if r.read_line(&mut line)? == 0 {
        return Ok(None);
    }
    let mut parts = line.split_whitespace();
    let (Some(method), Some(path)) = (parts.next(), parts.next()) else {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "bad request line"));
    };
    let (method, path) = (method.to_string(), path.to_string());
    let mut headers = Vec::new();
    loop {
        let mut h = String::new();
        if r.read_line(&mut h)? == 0 {
            return Ok(None);
        }
        let h = h.trim_end();
        if h.is_empty() {
            break;
        }
        if let Some((k, v)) = h.split_once(':') {
            headers.push((k.trim().to_string(), v.trim().to_string()));
        }
    }
    let mut req = HttpRequest {
        method,
        path,
        headers,
        body: Vec::new(),
    };
    if let Some(len) = req.header("Content-Length") {
        let len: usize = len
            .parse()
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "bad length"))?;
        if len > MAX_BODY {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "request too large"));
        }
        req.body = vec![0; len];
        r.read_exact(&mut req.body)?;
    } else if req
        .header("Transfer-Encoding")
        .is_some_and(|t| t.eq_ignore_ascii_case("chunked"))
    {
        loop {
            let mut size = String::new();
            r.read_line(&mut size)?;
            let n = usize::from_str_radix(size.trim().split(';').next().unwrap_or("0"), 16)
                .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "bad chunk"))?;
            if n == 0 {
                let mut end = String::new();
                r.read_line(&mut end)?;
                break;
            }
            if req.body.len() + n > MAX_BODY {
                return Err(io::Error::new(io::ErrorKind::InvalidData, "request too large"));
            }
            let mut chunk = vec![0; n + 2];
            r.read_exact(&mut chunk)?;
            req.body.extend_from_slice(&chunk[..n]);
        }
    }
    Ok(Some(req))
}

fn write_response(w: &mut impl Write, status: &str, headers: &[(&str, &str)], body: &[u8]) -> io::Result<()> {
    let mut head = format!("HTTP/1.1 {status}\r\nContent-Length: {}\r\n", body.len());
    for (k, v) in headers {
        head.push_str(&format!("{k}: {v}\r\n"));
    }
    head.push_str("\r\n");
    w.write_all(head.as_bytes())?;
    w.write_all(body)?;
    w.flush()
}

/// Whether a Host header names this loopback port.
fn host_ok(host: Option<&str>, port: u16) -> bool {
    let Some(host) = host else { return false };
    ["127.0.0.1", "localhost", "[::1]"]
        .iter()
        .any(|h| host.eq_ignore_ascii_case(h) || host.eq_ignore_ascii_case(&format!("{h}:{port}")))
}

/// Whether an Origin (when there is one) is a local page.
fn origin_ok(origin: Option<&str>) -> bool {
    let Some(origin) = origin else { return true };
    let rest = origin.split_once("://").map_or(origin, |(_, r)| r);
    let host = if rest.starts_with('[') {
        rest.split(']')
            .next()
            .map(|h| format!("{h}]"))
            .unwrap_or_default()
    } else {
        rest.split([':', '/']).next().unwrap_or_default().to_string()
    };
    ["127.0.0.1", "localhost", "[::1]"]
        .iter()
        .any(|h| host.eq_ignore_ascii_case(h))
}

/// Compares without stopping at the first difference.
fn same(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn serve_connection(
    stream: TcpStream,
    port: u16,
    token: &str,
    tx: &Sender<Request>,
    wake: &(dyn Fn() + Send + Sync),
    stop: &AtomicBool,
) {
    let _ = stream.set_nonblocking(false);
    let _ = stream.set_read_timeout(Some(Duration::from_secs(300)));
    let Ok(write) = stream.try_clone() else { return };
    let mut w = io::BufWriter::new(write);
    let mut r = BufReader::new(stream);
    while !stop.load(Ordering::Relaxed) {
        let req = match read_request(&mut r) {
            Ok(Some(req)) => req,
            Ok(None) => return,
            Err(_) => {
                let _ = write_response(
                    &mut w,
                    "400 Bad Request",
                    &[("Connection", "close")],
                    b"bad request",
                );
                return;
            }
        };
        let close = req
            .header("Connection")
            .is_some_and(|c| c.eq_ignore_ascii_case("close"));
        let json = [("Content-Type", "application/json")];
        let result = if !host_ok(req.header("Host"), port) || !origin_ok(req.header("Origin")) {
            write_response(&mut w, "403 Forbidden", &[], b"only local clients may connect")
        } else if req.path.trim_end_matches('/') != "/mcp" {
            write_response(&mut w, "404 Not Found", &[], b"the endpoint is /mcp")
        } else if !req
            .header("Authorization")
            .and_then(|a| a.strip_prefix("Bearer "))
            .is_some_and(|t| same(t.trim(), token))
        {
            write_response(
                &mut w,
                "401 Unauthorized",
                &[("WWW-Authenticate", "Bearer")],
                b"a bearer token is required (see the agent link in survey3d's settings)",
            )
        } else if req.method != "POST" {
            write_response(&mut w, "405 Method Not Allowed", &[("Allow", "POST")], b"")
        } else {
            match serde_json::from_slice::<Value>(&req.body) {
                Err(e) => {
                    let body = json!({ "jsonrpc": "2.0", "id": null, "error": { "code": -32700, "message": format!("Parse error: {e}") } });
                    write_response(&mut w, "400 Bad Request", &json, body.to_string().as_bytes())
                }
                Ok(message) => {
                    let (reply_tx, reply_rx) = channel();
                    if tx
                        .send(Request {
                            message,
                            reply: reply_tx,
                        })
                        .is_err()
                    {
                        return;
                    }
                    wake();
                    match reply_rx.recv_timeout(ANSWER_TIMEOUT) {
                        Ok(Some(reply)) => {
                            write_response(&mut w, "200 OK", &json, reply.to_string().as_bytes())
                        }
                        Ok(None) => write_response(&mut w, "202 Accepted", &[], b""),
                        Err(_) => write_response(
                            &mut w,
                            "503 Service Unavailable",
                            &[],
                            b"survey3d did not answer in time",
                        ),
                    }
                }
            }
        };
        if result.is_err() || close {
            return;
        }
    }
}

// ====================================================================== the app side

/// Tools that only look: they run alongside a tool the user is using.
const LOOKING: &[&str] = &[
    "project_info",
    "query",
    "render",
    "solve_curve",
    "setout",
    "convert_coordinates",
];

/// What an agent request did, for the status bar and the command log.
#[derive(Clone, Debug, PartialEq)]
pub struct Served {
    /// The tool called (`None` for the handshake and other messages).
    pub tool: Option<String>,
    /// The first line of its result, or its error.
    pub summary: String,
    pub error: bool,
    /// The agent opened or created another project.
    pub replaced_project: bool,
}

/// Answers one message on `session` (the app's) through `server`, which
/// lends the session for the call: the session's command log is kept (a line
/// is added saying what the agent did), and a tool the user is in the middle
/// of is cancelled first unless the agent only looks.
pub fn serve_on(session: &mut Session, server: &mut Server, message: Value) -> (Option<Value>, Served) {
    let tool = (message.get("method").and_then(Value::as_str) == Some("tools/call"))
        .then(|| {
            message
                .pointer("/params/name")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .flatten();
    if let Some(t) = &tool
        && !LOOKING.contains(&t.as_str())
        && session.active_tool.id() != crate::tools::SELECT_TOOL_ID
    {
        let name = session.active_tool.id().to_string();
        session.set_tool(crate::tools::SELECT_TOOL_ID);
        session.log.warn(format!("The agent's {t} cancelled your {name}"));
    }
    let generation = session.generation();
    let user_log = std::mem::take(&mut session.log);
    std::mem::swap(session, server.session_mut());
    let reply = protocol::handle(message, server);
    std::mem::swap(session, server.session_mut());
    // The agent's output goes back in its reply; the user's log carries on.
    session.log = user_log;
    let content = reply
        .as_ref()
        .and_then(|r| r.pointer("/result/content/0/text"))
        .and_then(Value::as_str);
    let error = reply.as_ref().is_some_and(|r| {
        r.get("error").is_some() || r.pointer("/result/isError").and_then(Value::as_bool) == Some(true)
    });
    let summary = content
        .or_else(|| {
            reply
                .as_ref()
                .and_then(|r| r.pointer("/error/message"))
                .and_then(Value::as_str)
        })
        .map(first_line)
        .unwrap_or_default();
    // Commands the agent typed show as typed, with their output; other
    // tools as one line.
    let transcript = server.take_transcript();
    for line in &transcript {
        session.log.log(line.kind, line.text.clone());
    }
    if let Some(t) = tool.as_ref().filter(|_| transcript.is_empty()) {
        let line = format!("agent › {t}: {summary}");
        if error {
            session.log.warn(line);
        } else {
            session.log.info(line);
        }
    }
    let served = Served {
        tool,
        summary,
        error,
        replaced_project: session.generation() != generation,
    };
    (reply, served)
}

/// A short first line of a result (JSON results give their summary or kind).
fn first_line(text: &str) -> String {
    let t = text.trim_start();
    let line = if t.starts_with('{') {
        serde_json::from_str::<Value>(t)
            .ok()
            .and_then(|v| v.get("summary").and_then(Value::as_str).map(str::to_string))
            .unwrap_or_else(|| "done".into())
    } else {
        t.lines()
            .find(|l| !l.trim().is_empty() && !l.starts_with("> "))
            .unwrap_or(t)
            .to_string()
    };
    let mut s: String = line.chars().take(140).collect();
    if line.chars().count() > 140 {
        s.push('…');
    }
    s
}

// ====================================================================== the relay

/// One POST to the link: the status code and body.
pub fn post(port: u16, token: &str, body: &str) -> io::Result<(u16, String)> {
    let mut stream = TcpStream::connect(SocketAddr::from(([127, 0, 0, 1], port)))?;
    stream.set_read_timeout(Some(ANSWER_TIMEOUT))?;
    write!(
        stream,
        "POST /mcp HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nAuthorization: Bearer {token}\r\n\
         Content-Type: application/json\r\nAccept: application/json, text/event-stream\r\n\
         Connection: close\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    )?;
    stream.flush()?;
    let mut r = BufReader::new(stream);
    let mut status = String::new();
    r.read_line(&mut status)?;
    let code = status
        .split_whitespace()
        .nth(1)
        .and_then(|c| c.parse().ok())
        .unwrap_or(0);
    let mut len = None;
    loop {
        let mut h = String::new();
        if r.read_line(&mut h)? == 0 {
            break;
        }
        let h = h.trim_end();
        if h.is_empty() {
            break;
        }
        if let Some((k, v)) = h.split_once(':')
            && k.trim().eq_ignore_ascii_case("content-length")
        {
            len = v.trim().parse::<usize>().ok();
        }
    }
    let mut body = Vec::new();
    match len {
        Some(n) => {
            body.resize(n, 0);
            r.read_exact(&mut body)?;
        }
        None => {
            r.read_to_end(&mut body)?;
        }
    }
    Ok((code, String::from_utf8_lossy(&body).into_owned()))
}

/// Relays newline-delimited JSON-RPC from `input` to the running app's link
/// (`info`, else the link file) and its replies to `output`. When the app is
/// not running with the link on, requests get an error saying so.
pub fn relay(input: impl BufRead, mut output: impl Write, info: Option<LinkInfo>) -> io::Result<()> {
    for line in input.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let id = serde_json::from_str::<Value>(&line)
            .ok()
            .and_then(|v| v.get("id").cloned());
        let info = info.clone().or_else(read_info);
        let reply = match &info {
            None => Err("survey3d's agent link has never been turned on: open survey3d and switch on Settings → General → Agent link".to_string()),
            Some(i) => match post(i.port, &i.token, &line) {
                Ok((200, body)) => Ok(Some(body)),
                Ok((202, _)) => Ok(None),
                Ok((code, body)) => Err(format!("survey3d's agent link answered {code}: {}", body.trim())),
                Err(e) => Err(format!(
                    "survey3d is not running with the agent link on (port {}: {e}): open survey3d and switch on Settings → General → Agent link",
                    i.port
                )),
            },
        };
        match reply {
            Ok(Some(body)) => writeln!(output, "{}", body.trim())?,
            Ok(None) => {}
            // Requests get the error; notifications have no one to tell.
            Err(e) => {
                if let Some(id) = id {
                    let err =
                        json!({ "jsonrpc": "2.0", "id": id, "error": { "code": -32000, "message": e } });
                    writeln!(output, "{err}")?;
                }
            }
        }
        output.flush()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::server::Options;
    use crate::project::Project;
    use crate::settings::AppSettings;

    #[test]
    fn origins_hosts_and_tokens() {
        assert!(host_ok(Some("127.0.0.1:3850"), 3850) && host_ok(Some("localhost:3850"), 3850));
        assert!(!host_ok(Some("evil.example:3850"), 3850) && !host_ok(None, 3850));
        assert!(
            origin_ok(None) && origin_ok(Some("http://localhost:5173")) && origin_ok(Some("http://[::1]:1"))
        );
        assert!(
            !origin_ok(Some("https://evil.example")) && !origin_ok(Some("http://127.0.0.1.evil.example"))
        );
        assert!(same("abc", "abc") && !same("abc", "abd") && !same("abc", "ab"));
        let (a, b) = (random_token(), random_token());
        assert!(a.len() == 64 && a != b);
    }

    /// Raw HTTP, for requests [`post`] would not make.
    fn raw(port: u16, request: &str) -> String {
        let mut s = TcpStream::connect(("127.0.0.1", port)).expect("connect");
        s.write_all(request.as_bytes()).expect("write");
        let mut out = String::new();
        let _ = s.read_to_string(&mut out);
        out
    }

    /// The link answered by a thread standing in for the app's UI, on a
    /// session of its own that already has a line in its command log.
    #[test]
    fn agents_drive_the_session_on_screen() {
        let token = random_token();
        let link = Link::start(0, token.clone(), Arc::new(|| {})).expect("start");
        let port = link.port;
        let stop = Arc::new(AtomicBool::new(false));
        let ui_stop = stop.clone();
        let ui = std::thread::spawn(move || {
            let mut session = Session::new(Project::new(&AppSettings::default()).expect("project"));
            session.log.info("the user's own line");
            let mut server = Server::new(Options {
                project: None,
                read_only: false,
                user: "tester".into(),
            })
            .expect("server");
            let mut served = Vec::new();
            while !ui_stop.load(Ordering::Relaxed) {
                for req in link.requests() {
                    let (reply, s) = serve_on(&mut session, &mut server, req.message.clone());
                    served.push(s);
                    req.answer(reply);
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            // What the test checks (a session cannot leave its thread).
            let found = session.project.doc.find_point_by_name("A1").is_some();
            let log: Vec<String> = session.log.lines().map(|l| l.text.clone()).collect();
            (found, log, served)
        });

        let init = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"t","version":"0"}}}"#;
        let (code, body) = post(port, &token, init).expect("post");
        assert_eq!(code, 200, "{body}");
        assert!(body.contains("\"protocolVersion\":\"2025-06-18\""), "{body}");
        let (code, _) = post(
            port,
            &token,
            r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
        )
        .expect("post");
        assert_eq!(code, 202);
        let add = r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"add_points","arguments":{"points":[{"name":"A1","e":100,"n":200}]}}}"#;
        let (code, body) = post(port, &token, add).expect("post");
        assert_eq!(code, 200, "{body}");

        // Refused: a wrong token, a foreign Host or Origin, the wrong method or path.
        assert_eq!(post(port, "wrong", init).expect("post").0, 401);
        let bad_host = format!(
            "POST /mcp HTTP/1.1\r\nHost: evil.example:{port}\r\nAuthorization: Bearer {token}\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{{}}"
        );
        assert!(raw(port, &bad_host).starts_with("HTTP/1.1 403"));
        let bad_origin = format!(
            "POST /mcp HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nOrigin: https://evil.example\r\nAuthorization: Bearer {token}\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{{}}"
        );
        assert!(raw(port, &bad_origin).starts_with("HTTP/1.1 403"));
        let get = format!(
            "GET /mcp HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nAuthorization: Bearer {token}\r\nConnection: close\r\n\r\n"
        );
        assert!(raw(port, &get).starts_with("HTTP/1.1 405"));

        // The relay passes stdio through; notifications get nothing back.
        let info = LinkInfo {
            port,
            token: token.clone(),
            url: String::new(),
        };
        let input = format!(
            "{init}\n{}\n{}\n",
            r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
            r#"{"jsonrpc":"2.0","id":3,"method":"tools/list"}"#
        );
        let mut out = Vec::new();
        relay(input.as_bytes(), &mut out, Some(info)).expect("relay");
        let lines: Vec<Value> = String::from_utf8(out)
            .expect("utf8")
            .lines()
            .map(|l| serde_json::from_str(l).expect("json"))
            .collect();
        assert_eq!(lines.len(), 2);
        assert!(
            lines[1]["result"]["tools"]
                .as_array()
                .is_some_and(|t| t.len() > 40)
        );

        stop.store(true, Ordering::Relaxed);
        let (found, log, served) = ui.join().expect("ui");
        // The point is in the session on screen, and the user's log carries on
        // with a line for what the agent did.
        assert!(found);
        assert_eq!(
            log.first().map(String::as_str),
            Some("the user's own line"),
            "{log:?}"
        );
        assert!(log.iter().any(|l| l.starts_with("agent › add_points")), "{log:?}");
        assert!(
            served
                .iter()
                .any(|s| s.tool.as_deref() == Some("add_points") && !s.error)
        );

        // With the app gone, requests are told so.
        let mut out = Vec::new();
        let gone = LinkInfo {
            port,
            token,
            url: String::new(),
        };
        relay(format!("{init}\n").as_bytes(), &mut out, Some(gone)).expect("relay");
        let reply: Value = serde_json::from_slice(&out).expect("json");
        assert!(
            reply["error"]["message"]
                .as_str()
                .is_some_and(|m| m.contains("not running")),
            "{reply}"
        );
    }

    #[test]
    fn a_tool_in_progress_is_cancelled_unless_the_agent_only_looks() {
        let mut session = Session::new(Project::new(&AppSettings::default()).expect("project"));
        let mut server = Server::new(Options {
            project: None,
            read_only: false,
            user: "tester".into(),
        })
        .expect("server");
        session.run("pl");
        session.run("0,0");
        let look = json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"project_info","arguments":{}}});
        serve_on(&mut session, &mut server, look);
        assert_eq!(
            session.active_tool.id(),
            "polyline",
            "looking leaves the user's polyline alone"
        );
        let edit = json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"add_points","arguments":{"points":[{"e":1,"n":1}]}}});
        let (_, served) = serve_on(&mut session, &mut server, edit);
        assert_eq!(session.active_tool.id(), crate::tools::SELECT_TOOL_ID);
        assert!(!served.error && !served.replaced_project);
        let new = json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"new_project","arguments":{"discard_changes":true}}});
        let (_, served) = serve_on(&mut session, &mut server, new);
        assert!(served.replaced_project, "{served:?}");
    }

    #[test]
    fn window_commands_and_view_events_reach_the_window() {
        use crate::session::Event;
        let mut session = Session::new(Project::new(&AppSettings::default()).expect("project"));
        session.in_window = true;
        let mut server = Server::new(Options {
            project: None,
            read_only: false,
            user: "tester".into(),
        })
        .expect("server");
        let call = |name: &str, arguments: Value| json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":name,"arguments":arguments}});
        serve_on(
            &mut session,
            &mut server,
            call(
                "add_points",
                json!({"points":[{"name":"A","e":0,"n":0},{"name":"B","e":30,"n":40}]}),
            ),
        );
        let (reply, _) = serve_on(
            &mut session,
            &mut server,
            call(
                "run_commands",
                json!({"lines":["ze", "bogus", "inverse", "A", "B"]}),
            ),
        );
        let text = reply
            .as_ref()
            .and_then(|r| r.pointer("/result/content/0/text"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        assert!(text.contains("'ze' passed to the window"), "{text}");
        assert!(
            text.contains("50.000"),
            "the agent still gets the inverse: {text}"
        );
        let shown: Vec<String> = session.log.lines().map(|l| l.text.clone()).collect();
        assert!(
            shown.iter().any(|l| l.ends_with("inverse")) && shown.iter().any(|l| l.contains("50.000")),
            "the window's command line shows what the agent typed and its output: {shown:?}"
        );
        assert!(
            !shown.iter().any(|l| l.contains("passed to the window")),
            "{shown:?}"
        );
        let events = session.take_events();
        assert!(
            events.iter().any(|e| matches!(e, Event::Command(c) if c == "ze")),
            "{events:?}"
        );
        assert!(
            events
                .iter()
                .any(|e| matches!(e, Event::Command(c) if c == "bogus")),
            "the window reports it"
        );
        serve_on(
            &mut session,
            &mut server,
            call(
                "setout",
                json!({"station":"A","backsight":"B","points":["B"],"report":true}),
            ),
        );
        let events = session.take_events();
        assert!(
            events
                .iter()
                .any(|e| matches!(e, Event::Report { title, .. } if title == "Stake-out")),
            "the agent's reports show in the window: {events:?}"
        );

        // Headless, nothing is passed on and unknown commands are errors.
        let mut headless = Session::new(Project::new(&AppSettings::default()).expect("project"));
        let (reply, _) = serve_on(
            &mut headless,
            &mut server,
            call("run_commands", json!({"lines":["ze"]})),
        );
        assert!(reply.is_some_and(|r| r.to_string().contains("Unknown command")));
        assert!(headless.take_events().is_empty());
    }
}
