//! The client side of the control socket: one request line out, one
//! response line back, on a connection of its own. `impulse` sends one
//! request and exits; `impulse mcp` sends one per tool call, possibly
//! several at once, and hangs up on a call the agent cancels (which stops a
//! waiting request in the app, as Ctrl-C does for `impulse tasks wait`).

use std::io::{Read, Write};
use std::net::Shutdown;
use std::os::unix::net::UnixStream;
use std::sync::Mutex;

use crate::control::{self, ControlRequest, ControlResponse};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Failure {
    /// The socket couldn't be reached: Impulse quit, or this isn't its
    /// socket.
    Unreachable,
    /// The connection closed (or the client hung up) before an answer.
    NoAnswer,
}

/// One connection to the app, for one request: [`ControlClient`], or a
/// stand-in in tests.
pub trait ControlChannel: Send + Sync {
    /// Send `request` and wait for its answer.
    fn send(&self, request: &ControlRequest) -> Result<ControlResponse, Failure>;
    /// Hang up: a request waiting for its answer gets `NoAnswer`. Safe from
    /// any thread, before or during `send`.
    fn cancel(&self);
}

pub struct ControlClient {
    socket_path: String,
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    stream: Option<UnixStream>,
    cancelled: bool,
}

impl ControlClient {
    pub fn new(socket_path: impl Into<String>) -> Self {
        ControlClient {
            socket_path: socket_path.into(),
            state: Mutex::new(State::default()),
        }
    }

    /// Send `request` and wait for its answer (an `edit` or a `tasks wait`
    /// waits here until it completes). Call once per client.
    pub fn send(&self, request: &ControlRequest) -> Result<ControlResponse, Failure> {
        // A path too long for a socket address fails here too.
        let mut stream =
            UnixStream::connect(&self.socket_path).map_err(|_| Failure::Unreachable)?;
        {
            let mut state = self.state.lock().unwrap();
            if state.cancelled {
                return Err(Failure::NoAnswer);
            }
            state.stream = stream.try_clone().ok();
        }
        let result = Self::exchange(&mut stream, request);
        self.state.lock().unwrap().stream = None;
        result
    }

    fn exchange(
        stream: &mut UnixStream,
        request: &ControlRequest,
    ) -> Result<ControlResponse, Failure> {
        // A write to a socket the app closed fails with EPIPE: Rust ignores
        // SIGPIPE, so it doesn't kill the tool.
        stream
            .write_all(&control::encode_line(request))
            .map_err(|_| Failure::NoAnswer)?;
        let mut response = Vec::new();
        let mut buffer = [0u8; 4096];
        'reading: loop {
            let count = match stream.read(&mut buffer) {
                Ok(0) => break,
                Ok(count) => count,
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(_) => break,
            };
            for &byte in &buffer[..count] {
                if byte == b'\n' {
                    break 'reading;
                }
                response.push(byte);
            }
        }
        control::decode(&response).map_err(|_| Failure::NoAnswer)
    }

    /// Hang up: a request waiting for its answer gets `NoAnswer`, and the
    /// app stops what it was waiting for. Safe from any thread, before or
    /// during `send`.
    pub fn cancel(&self) {
        let mut state = self.state.lock().unwrap();
        state.cancelled = true;
        if let Some(stream) = &state.stream {
            let _ = stream.shutdown(Shutdown::Both);
        }
    }
}

impl ControlChannel for ControlClient {
    fn send(&self, request: &ControlRequest) -> Result<ControlResponse, Failure> {
        ControlClient::send(self, request)
    }

    fn cancel(&self) {
        ControlClient::cancel(self)
    }
}
