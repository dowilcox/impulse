//! `impulse`: drives the Impulse window this terminal belongs to (see
//! `impulse_protocol::control`). Bundled with the app and put on PATH
//! inside Impulse terminals. The Rust port of the Swift `ImpulseCLI`, with
//! the same arguments, output and exit codes: 0 when Impulse did it (or the
//! usage was asked for), 1 when it couldn't, 2 for a usage mistake.
//!
//! Agent hooks are configured for an agent everywhere but only mean
//! something inside Impulse, so `impulse hook` is silent and successful
//! outside it, or when Impulse doesn't answer.

use std::collections::HashMap;
use std::fs::File;
use std::io::{self, BufRead, Read, Write};
use std::path::{Path, PathBuf};
use std::process::exit;
use std::sync::Arc;
use std::time::Duration;

use impulse_protocol::args::{self, UsageError};
use impulse_protocol::control::{BRIEF_LIMIT, SOCKET_KEY};
use impulse_protocol::mcp::McpServer;
use impulse_protocol::{ControlClient, Failure};

/// The most `impulse open` copies of something that can only be read here
/// (the editor's own limit).
const OPEN_LIMIT: u64 = 10 * 1024 * 1024;

fn main() {
    let arguments: Vec<String> = std::env::args_os()
        .skip(1)
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect();
    let environment: HashMap<String, String> = std::env::vars_os()
        .map(|(key, value)| {
            (
                key.to_string_lossy().into_owned(),
                value.to_string_lossy().into_owned(),
            )
        })
        .collect();
    let cwd = std::env::current_dir()
        .map(|dir| dir.to_string_lossy().into_owned())
        .unwrap_or_else(|_| "/".to_string());
    let is_hook = arguments.first().map(String::as_str) == Some("hook");

    // `impulse mcp`: serve MCP on stdin/stdout until the agent closes
    // stdin. Outside Impulse it still answers (with no tools), so a server
    // registered for every project never shows up as broken.
    if arguments.first().map(String::as_str) == Some("mcp") {
        if matches!(arguments.get(1).map(String::as_str), Some("-h" | "--help")) {
            say("impulse mcp: an MCP server on standard input and output, for agents in Impulse terminals.");
            exit(0);
        }
        serve_mcp(&environment, &cwd);
        exit(0);
    }

    let mut stdin = || {
        let mut data = Vec::new();
        let _ = io::stdin().read_to_end(&mut data);
        Some(data)
    };
    let mut request = match args::request(&arguments, &environment, &cwd, &mut stdin) {
        Ok(request) => request,
        Err(UsageError { message, is_help }) => {
            let wants_help = is_help
                || arguments
                    .first()
                    .is_none_or(|first| ["-h", "--help", "help"].contains(&first.as_str()));
            if wants_help {
                say(&message);
                exit(0);
            }
            fail(&message, 2);
        }
    };

    let Some(socket_path) = environment.get(SOCKET_KEY) else {
        if is_hook {
            exit(0);
        }
        fail(
            "impulse: not running inside an Impulse terminal (IMPULSE_SOCKET isn't set)",
            1,
        );
    };

    // A pipe (`impulse open <(git show HEAD:x)`, fish's `psub`), a device,
    // or anything in /dev can only be read here: in the app, /dev/fd/63
    // would be one of its own files, whatever this one is. What it holds
    // opens from a temporary file instead.
    if request.command == "open" {
        if let Some(path) = request.arguments.get("path").cloned() {
            // A path that can't be looked at is left to the app to report.
            let regular_or_folder = std::fs::metadata(&path)
                .map(|meta| meta.is_file() || meta.is_dir())
                .unwrap_or(true);
            let resolved = std::fs::canonicalize(&path)
                .map(|real| real.to_string_lossy().into_owned())
                .unwrap_or_else(|_| path.clone());
            let in_dev = [&path, &resolved]
                .iter()
                .any(|p| p.as_str() == "/dev" || p.starts_with("/dev/"));
            if in_dev || !regular_or_folder {
                if request.wait {
                    fail(
                        &format!(
                            "impulse edit: {path} can't be edited in place; impulse open shows a copy"
                        ),
                        1,
                    );
                }
                let copy = temporary_copy(&path);
                request
                    .arguments
                    .insert("path".into(), copy.to_string_lossy().into_owned());
            }
        }
    }

    // `impulse tasks new --brief-file notes.md`: the brief travels in the
    // request (the app may not be able to read the file, and it shouldn't
    // be read twice).
    if let Some(path) = request.arguments.remove("briefFile") {
        let unreadable = || -> ! { fail(&format!("impulse tasks new: couldn't read {path}"), 1) };
        let file = File::open(&path).unwrap_or_else(|_| unreadable());
        let mut data = Vec::new();
        if file
            .take(BRIEF_LIMIT as u64 + 1)
            .read_to_end(&mut data)
            .is_err()
        {
            unreadable();
        }
        if data.len() > BRIEF_LIMIT {
            fail(
                &format!("impulse tasks new: {path} holds more than 16 KB"),
                1,
            );
        }
        request
            .arguments
            .insert("brief".into(), String::from_utf8_lossy(&data).into_owned());
    }

    // An `edit` (or a `tasks wait`) waits here until it completes.
    let reply = match ControlClient::new(socket_path.clone()).send(&request) {
        Ok(reply) => reply,
        Err(failure) => {
            if is_hook {
                exit(0);
            }
            match failure {
                Failure::Unreachable => {
                    fail("impulse: couldn't reach Impulse (is it still running?)", 1)
                }
                Failure::NoAnswer => fail("impulse: no answer from Impulse", 1),
            }
        }
    };
    if let Some(message) = reply.message.as_deref().filter(|m| !m.is_empty()) {
        if reply.ok {
            say(message);
        } else {
            let _ = writeln!(io::stderr(), "{message}");
        }
    }
    exit(if reply.ok { 0 } else { 1 });
}

/// A line on standard output.
fn say(text: &str) {
    let mut out = io::stdout().lock();
    let _ = writeln!(out, "{text}");
    let _ = out.flush();
}

fn fail(message: &str, code: i32) -> ! {
    let _ = writeln!(io::stderr(), "{message}");
    exit(code);
}

fn serve_mcp(environment: &HashMap<String, String>, cwd: &str) {
    let server = McpServer::standard(
        environment,
        cwd,
        &app_version(),
        // Lines come from several threads; the lock keeps each whole.
        Arc::new(|line: &[u8]| {
            let mut out = io::stdout().lock();
            let _ = out.write_all(line);
            let _ = out.flush();
        }),
    );
    let mut input = io::stdin().lock();
    let mut line = Vec::new();
    loop {
        line.clear();
        match input.read_until(b'\n', &mut line) {
            Ok(0) | Err(_) => break,
            Ok(_) => server.handle(&line),
        }
    }
    server.finish(Duration::from_secs(2));
}

/// The app's version, from the bundle this tool is in
/// (`Contents/Resources/bin/impulse`); outside one, this build's.
fn app_version() -> String {
    let bundled = std::env::current_exe()
        .and_then(std::fs::canonicalize)
        .ok()
        .and_then(|exe| {
            let contents = exe.parent()?.parent()?.parent()?.to_path_buf();
            std::fs::read_to_string(contents.join("Info.plist")).ok()
        })
        .and_then(|plist| plist_string(&plist, "CFBundleShortVersionString"));
    bundled.unwrap_or_else(|| env!("CARGO_PKG_VERSION").to_string())
}

/// A string value from an XML property list.
fn plist_string(plist: &str, key: &str) -> Option<String> {
    let after_key = &plist[plist.find(&format!("<key>{key}</key>"))?..];
    let start = after_key.find("<string>")? + "<string>".len();
    let end = after_key[start..].find("</string>")?;
    Some(after_key[start..start + end].trim().to_string())
}

/// The contents of `path` in a new temporary file (same name), for the
/// editor; up to its 10 MB limit.
fn temporary_copy(path: &str) -> PathBuf {
    let unreadable = || -> ! { fail(&format!("impulse open: couldn't read {path}"), 1) };
    let file = File::open(path).unwrap_or_else(|_| unreadable());
    let mut data = Vec::new();
    if file.take(OPEN_LIMIT + 1).read_to_end(&mut data).is_err() {
        unreadable();
    }
    if data.len() as u64 > OPEN_LIMIT {
        fail(&format!("impulse open: {path} holds more than 10 MB"), 1);
    }
    let no_file = || -> ! { fail("impulse open: couldn't make a temporary file", 1) };
    let directory = tempfile::Builder::new()
        .prefix("impulse-open-")
        .rand_bytes(6)
        .tempdir()
        .unwrap_or_else(|_| no_file())
        .keep();
    let name = Path::new(path)
        .file_name()
        .map(|name| name.to_os_string())
        .unwrap_or_else(|| "file".into());
    let copy = directory.join(name);
    if std::fs::write(&copy, &data).is_err() {
        no_file();
    }
    copy
}
