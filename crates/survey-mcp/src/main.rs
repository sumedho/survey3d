//! survey3d MCP server: an agent opens, edits, queries and renders survey3d
//! projects through the same session the application uses.
//!
//! ```text
//! survey3d-mcp [--project FILE.s3d] [--read-only] [--user NAME]
//! survey3d-mcp --attach
//! ```
//! Speaks the Model Context Protocol on stdin/stdout: on a project of its own
//! (headless), or with `--attach` relayed to the survey3d window that is
//! open, so the drawing changes on screen as the agent works.

use std::path::PathBuf;
use std::process::ExitCode;

use survey_app::agent::{Options, Server, link, protocol};

const USAGE: &str = "Usage: survey3d-mcp [--project FILE.s3d] [--read-only] [--user NAME]
       survey3d-mcp --attach

Model Context Protocol server for survey3d, on stdin/stdout.
  --project FILE  open this project at start (otherwise a new, empty one)
  --read-only     never write the project file (edits stay in memory)
  --user NAME     name recorded in the audit trail (default: $USER)
  --attach        drive the survey3d window that is open instead (its agent
                  link must be on: Settings > General > Agent link)";

fn parse_args() -> Result<Options, String> {
    let mut options = Options {
        project: None,
        read_only: false,
        user: std::env::var("USER")
            .or_else(|_| std::env::var("USERNAME"))
            .unwrap_or_else(|_| "agent".into()),
    };
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--project" => {
                options.project = Some(PathBuf::from(args.next().ok_or("--project needs a file")?))
            }
            "--read-only" => options.read_only = true,
            "--user" => options.user = args.next().ok_or("--user needs a name")?,
            "--attach" => return Err("--attach takes no other arguments".into()),
            "-h" | "--help" => return Err(String::new()),
            other => return Err(format!("unknown argument '{other}'")),
        }
    }
    Ok(options)
}

fn main() -> ExitCode {
    if std::env::args().nth(1).as_deref() == Some("--attach") && std::env::args().count() == 2 {
        let stdin = std::io::stdin();
        let stdout = std::io::stdout();
        return match link::relay(stdin.lock(), stdout.lock(), None) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("survey3d-mcp: {e}");
                ExitCode::FAILURE
            }
        };
    }
    let options = match parse_args() {
        Ok(o) => o,
        Err(e) => {
            if !e.is_empty() {
                eprintln!("survey3d-mcp: {e}");
            }
            eprintln!("{USAGE}");
            return if e.is_empty() {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            };
        }
    };
    let mut server = match Server::new(options) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("survey3d-mcp: {e}");
            return ExitCode::FAILURE;
        }
    };
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    match protocol::serve(stdin.lock(), stdout.lock(), &mut server) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("survey3d-mcp: {e}");
            ExitCode::FAILURE
        }
    }
}
