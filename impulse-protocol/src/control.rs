//! The conversation between the `impulse` command-line tool and the running
//! app: one JSON object per line over a Unix socket. Terminals inside
//! Impulse get the socket path and a token naming their pane in the
//! environment, so `impulse` can act on "this pane" (and agent hooks can
//! report on it).
//!
//! The lines are the Swift app's, byte for byte: keys sorted, slashes
//! escaped, absent optional fields left out, and a response's `data` (the
//! answer as JSON) carried as a base64 string, as Swift's `JSONEncoder`
//! writes `Data`. Either app's `impulse` talks to either app.
//!
//! The server's side, as the Swift app keeps it: each connection carries
//! one request line and gets exactly one response line, possibly much later
//! (a request with `wait`), then closes. A client that hangs up before its
//! answer has given up on it (Ctrl-C in `impulse edit`). A connection that
//! sends no line within [`REQUEST_DEADLINE`], a longer one than
//! [`MAX_REQUEST_LINE`], or one that isn't a request is answered
//! [`MALFORMED_REQUEST`].

use std::collections::BTreeMap;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::json::{self, Style};

pub const SOCKET_KEY: &str = "IMPULSE_SOCKET";
pub const TOKEN_KEY: &str = "IMPULSE_PANE_TOKEN";
pub const CLI_KEY: &str = "IMPULSE_CLI";

/// The most a task's brief may hold, in bytes.
pub const BRIEF_LIMIT: usize = 16 * 1024;

/// How long a client has to send its request line.
pub const REQUEST_DEADLINE: Duration = Duration::from_secs(5);
/// The longest request line a server reads, in bytes.
pub const MAX_REQUEST_LINE: usize = 1_048_576;
/// The answer to anything that isn't a request.
pub const MALFORMED_REQUEST: &str = "impulse: malformed request";

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ControlRequest {
    pub command: String,
    /// The calling terminal's pane token (`IMPULSE_PANE_TOKEN`), if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
    /// The caller's working directory, for relative paths.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    pub arguments: BTreeMap<String, String>,
    /// Answer only when the action completes (e.g. the edited file's tab
    /// closes).
    pub wait: bool,
}

impl ControlRequest {
    pub fn new(command: impl Into<String>) -> Self {
        ControlRequest {
            command: command.into(),
            ..Default::default()
        }
    }

    /// An argument's value.
    pub fn argument(&self, key: &str) -> Option<&str> {
        self.arguments.get(key).map(String::as_str)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ControlResponse {
    pub ok: bool,
    /// What `impulse` prints: text, or JSON when `--json` asked for it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    /// The same answer as one JSON object, for `impulse mcp`'s structured
    /// tool results (and `--json`). Older apps don't send it.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "base64_data")]
    pub data: Option<Vec<u8>>,
}

impl ControlResponse {
    pub fn ok() -> Self {
        ControlResponse {
            ok: true,
            ..Default::default()
        }
    }

    pub fn ok_message(message: impl Into<String>) -> Self {
        ControlResponse {
            ok: true,
            message: Some(message.into()),
            data: None,
        }
    }

    pub fn failure(message: impl Into<String>) -> Self {
        ControlResponse {
            ok: false,
            message: Some(message.into()),
            data: None,
        }
    }

    /// An answer for the agents' commands: `text`, or `value` as pretty
    /// JSON when the request asked for `--json`, with `value` as data either
    /// way (the app's `answer`).
    pub fn answer<T: Serialize>(value: &T, text: String, json: bool) -> Self {
        ControlResponse {
            ok: true,
            message: Some(if json {
                json::encode(value, Style::PRETTY)
            } else {
                text
            }),
            data: Some(json::encode(value, Style::COMPACT).into_bytes()),
        }
    }

    /// `data` parsed, when it's JSON.
    pub fn data_json(&self) -> Option<serde_json::Value> {
        serde_json::from_slice(self.data.as_deref()?).ok()
    }
}

/// `Data` as Swift's `JSONEncoder` writes it: a base64 string.
mod base64_data {
    use base64::engine::general_purpose::STANDARD;
    use base64::Engine;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(
        data: &Option<Vec<u8>>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        match data {
            Some(data) => serializer.serialize_str(&STANDARD.encode(data)),
            None => serializer.serialize_none(),
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Option<Vec<u8>>, D::Error> {
        match Option::<String>::deserialize(deserializer)? {
            Some(text) => STANDARD
                .decode(text)
                .map(Some)
                .map_err(|_| serde::de::Error::custom("Encountered Data is not valid Base64.")),
            None => Ok(None),
        }
    }
}

/// One line of JSON, newline included.
pub fn encode_line<T: Serialize>(value: &T) -> Vec<u8> {
    let mut line = json::encode(value, Style::CONTROL).into_bytes();
    line.push(b'\n');
    line
}

/// A request or response from one line (without its newline).
pub fn decode<'a, T: Deserialize<'a>>(line: &'a [u8]) -> serde_json::Result<T> {
    serde_json::from_slice(line)
}

/// Default socket location when the environment doesn't say (the Swift
/// app's, on the Mac).
pub fn default_socket_path(home: &str, dev: bool) -> String {
    format!(
        "{home}/Library/Application Support/{}/impulse.sock",
        if dev { "impulse-dev" } else { "impulse" }
    )
}
