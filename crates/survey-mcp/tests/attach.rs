//! `survey3d-mcp --attach` relays an MCP client on stdio to a running
//! survey3d's agent link (here, a link answered by a thread standing in for
//! the window), and headless mode still serves a project of its own.

use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use serde_json::Value;
use survey_app::agent::link::{self, Link, LinkInfo};
use survey_app::agent::{Options, Server};
use survey_app::settings::AppSettings;
use survey_app::{Project, Session};

const INIT: &str = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"t","version":"0"}}}"#;

/// Sends `lines` to the binary with `args` and returns its replies.
fn exchange(args: &[&str], env: &[(&str, &std::path::Path)], lines: &[&str]) -> Vec<Value> {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_survey3d-mcp"));
    cmd.args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    for (k, v) in env {
        cmd.env(k, v);
    }
    let mut child = cmd.spawn().expect("spawn");
    let mut stdin = child.stdin.take().expect("stdin");
    for l in lines {
        writeln!(stdin, "{l}").expect("write");
    }
    drop(stdin);
    let out = BufReader::new(child.stdout.take().expect("stdout"));
    let replies = out
        .lines()
        .map(|l| serde_json::from_str(&l.expect("line")).expect("json"))
        .collect();
    child.wait().expect("wait");
    replies
}

#[test]
fn attach_relays_to_the_window_and_headless_still_works() {
    let dir = std::env::temp_dir().join(format!("survey3d-attach-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("dir");
    let info_file = dir.join("agent-link.json");

    // No window yet: a request is told how to turn the link on.
    let replies = exchange(&["--attach"], &[("SURVEY3D_AGENT_LINK", &info_file)], &[INIT]);
    assert!(
        replies[0]["error"]["message"]
            .as_str()
            .is_some_and(|m| m.contains("Agent link")),
        "{replies:?}"
    );

    // A "window" with its link on.
    let token = "a".repeat(64);
    let link = Link::start(0, token.clone(), Arc::new(|| {})).expect("link");
    let port = link.port;
    std::fs::write(
        &info_file,
        serde_json::to_string(&LinkInfo {
            port,
            token,
            url: link.url(),
        })
        .expect("json"),
    )
    .expect("info");
    let stop = Arc::new(AtomicBool::new(false));
    let ui_stop = stop.clone();
    let ui = std::thread::spawn(move || {
        let mut session = Session::new(Project::new(&AppSettings::default()).expect("project"));
        let mut server = Server::new(Options {
            project: None,
            read_only: false,
            user: "t".into(),
        })
        .expect("server");
        while !ui_stop.load(Ordering::Relaxed) {
            for req in link.requests() {
                let (reply, _) = link::serve_on(&mut session, &mut server, req.message.clone());
                req.answer(reply);
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        session.project.doc.find_point_by_name("W1").is_some()
    });
    let add = r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"add_points","arguments":{"points":[{"name":"W1","e":5,"n":5}]}}}"#;
    let replies = exchange(
        &["--attach"],
        &[("SURVEY3D_AGENT_LINK", &info_file)],
        &[
            INIT,
            r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
            add,
        ],
    );
    assert_eq!(replies.len(), 2, "{replies:?}");
    assert_eq!(replies[0]["result"]["serverInfo"]["name"], "survey3d");
    assert_eq!(replies[1]["result"]["isError"], false, "{replies:?}");
    stop.store(true, Ordering::Relaxed);
    assert!(ui.join().expect("ui"), "the point went to the window's session");

    // Headless: its own project.
    let list = r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#;
    let replies = exchange(&[], &[], &[INIT, list]);
    assert!(
        replies[1]["result"]["tools"]
            .as_array()
            .is_some_and(|t| t.len() > 40)
    );
    let _ = std::fs::remove_dir_all(&dir);
}
