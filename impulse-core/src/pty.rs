use std::collections::HashMap;
use std::io::{Read, Write};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use portable_pty::{native_pty_system, Child, MasterPty, PtySize};
use serde::Serialize;
use uuid::Uuid;

use crate::shell::{self, ShellType};

/// Trait for sending PTY events to the frontend.
/// Implement this for your UI framework's event channel.
pub trait PtyEventSender: Send + 'static {
    fn send(&self, msg: PtyMessage);
}

/// Messages sent from the PTY backend to the frontend.
#[derive(Serialize, Clone, Debug)]
#[serde(tag = "type")]
pub enum PtyMessage {
    Output {
        data: Vec<u8>,
    },
    CommandStart {
        block_id: String,
        command: String,
    },
    CommandEnd {
        block_id: String,
        exit_code: i32,
        duration_ms: u64,
    },
    CwdChanged {
        path: String,
    },
    ShellReady,
}

/// A single PTY session.
pub struct PtySession {
    pub id: String,
    pub writer: Box<dyn Write + Send>,
    pub child: Box<dyn Child + Send + Sync>,
    pub master: Box<dyn MasterPty + Send>,
    pub shell_type: ShellType,
}

/// Manages all PTY sessions.
pub struct PtyManager {
    sessions: Arc<Mutex<HashMap<String, PtySession>>>,
}

impl Default for PtyManager {
    fn default() -> Self {
        Self::new()
    }
}

impl PtyManager {
    pub fn new() -> Self {
        Self {
            sessions: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Create a new PTY session, spawning the user's shell with integration hooks.
    pub fn create_session(
        &self,
        sender: impl PtyEventSender,
        cols: u16,
        rows: u16,
    ) -> Result<String, String> {
        let id = Uuid::new_v4().to_string();

        let spawn_config = shell::prepare_shell_spawn()
            .map_err(|e| format!("Failed to set up shell integration: {}", e))?;

        let temp_files = spawn_config.temp_files;
        let cmd = spawn_config.command;
        let shell_type = spawn_config.shell_type;

        let pty_system = native_pty_system();
        let pair = pty_system
            .openpty(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| format!("Failed to open PTY: {}", e))?;

        let child = pair
            .slave
            .spawn_command(cmd)
            .map_err(|e| format!("Failed to spawn shell: {}", e))?;

        let writer = pair
            .master
            .take_writer()
            .map_err(|e| format!("Failed to get PTY writer: {}", e))?;

        let mut reader = pair
            .master
            .try_clone_reader()
            .map_err(|e| format!("Failed to get PTY reader: {}", e))?;

        sender.send(PtyMessage::ShellReady);

        let session_id = id.clone();
        std::thread::spawn(move || {
            let mut buf = [0u8; 8192];
            let mut parser = OscParser::new();

            loop {
                match reader.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        let chunk = &buf[..n];
                        let events = parser.parse(chunk);

                        for event in events {
                            match event {
                                OscEvent::Output(data) => {
                                    if !data.is_empty() {
                                        sender.send(PtyMessage::Output { data });
                                    }
                                }
                                OscEvent::CommandStart => {
                                    let block_id = Uuid::new_v4().to_string();
                                    parser.current_block_id = Some(block_id.clone());
                                    parser.command_start_time = Some(Instant::now());
                                    sender.send(PtyMessage::CommandStart {
                                        block_id,
                                        command: String::new(),
                                    });
                                }
                                OscEvent::CommandEnd(exit_code) => {
                                    let duration_ms = parser
                                        .command_start_time
                                        .take()
                                        .map(|t| t.elapsed().as_millis() as u64)
                                        .unwrap_or(0);
                                    let block_id = parser
                                        .current_block_id
                                        .take()
                                        .unwrap_or_else(|| "unknown".to_string());
                                    sender.send(PtyMessage::CommandEnd {
                                        block_id,
                                        exit_code,
                                        duration_ms,
                                    });
                                }
                                OscEvent::CwdChanged(path) => {
                                    sender.send(PtyMessage::CwdChanged { path });
                                }
                            }
                        }
                    }
                    Err(e) => {
                        log::warn!("PTY reader error for session {}: {}", session_id, e);
                        break;
                    }
                }
            }

            for path in temp_files {
                let _ = std::fs::remove_file(&path);
                if let Some(parent) = path.parent() {
                    let _ = std::fs::remove_dir(parent);
                }
            }
        });

        let session = PtySession {
            id: id.clone(),
            writer,
            child,
            master: pair.master,
            shell_type,
        };

        self.sessions
            .lock()
            .map_err(|e| format!("Lock poisoned: {}", e))?
            .insert(id.clone(), session);

        Ok(id)
    }

    /// Write data to a PTY session's stdin.
    pub fn write_to(&self, id: &str, data: &[u8]) -> Result<(), String> {
        let mut sessions = self
            .sessions
            .lock()
            .map_err(|e| format!("Lock poisoned: {}", e))?;

        let session = sessions
            .get_mut(id)
            .ok_or_else(|| format!("Session not found: {}", id))?;

        session
            .writer
            .write_all(data)
            .map_err(|e| format!("Failed to write to PTY: {}", e))?;

        session
            .writer
            .flush()
            .map_err(|e| format!("Failed to flush PTY: {}", e))?;

        Ok(())
    }

    /// Resize a PTY session.
    pub fn resize(&self, id: &str, cols: u16, rows: u16) -> Result<(), String> {
        let sessions = self
            .sessions
            .lock()
            .map_err(|e| format!("Lock poisoned: {}", e))?;

        let session = sessions
            .get(id)
            .ok_or_else(|| format!("Session not found: {}", id))?;

        session
            .master
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| format!("Failed to resize PTY: {}", e))?;

        Ok(())
    }

    /// Close and clean up a PTY session.
    pub fn close_session(&self, id: &str) -> Result<(), String> {
        let mut sessions = self
            .sessions
            .lock()
            .map_err(|e| format!("Lock poisoned: {}", e))?;

        if let Some(mut session) = sessions.remove(id) {
            let _ = session.child.kill();
            let _ = session.child.wait();
            Ok(())
        } else {
            Err(format!("Session not found: {}", id))
        }
    }
}

impl Drop for PtyManager {
    fn drop(&mut self) {
        if let Ok(mut sessions) = self.sessions.lock() {
            let ids: Vec<String> = sessions.keys().cloned().collect();
            for id in ids {
                if let Some(mut session) = sessions.remove(&id) {
                    let _ = session.child.kill();
                    let _ = session.child.wait();
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// OSC Escape Sequence Parser
// ---------------------------------------------------------------------------

/// Events produced by the OSC parser.
pub enum OscEvent {
    Output(Vec<u8>),
    CommandStart,
    CommandEnd(i32),
    CwdChanged(String),
}

/// Byte-by-byte parser that scans for OSC sequences and strips them from output.
pub struct OscParser {
    output_buf: Vec<u8>,
    esc_buf: Vec<u8>,
    in_escape: bool,
    pub current_block_id: Option<String>,
    pub command_start_time: Option<Instant>,
}

impl Default for OscParser {
    fn default() -> Self {
        Self::new()
    }
}

impl OscParser {
    pub fn new() -> Self {
        Self {
            output_buf: Vec::with_capacity(8192),
            esc_buf: Vec::with_capacity(256),
            in_escape: false,
            current_block_id: None,
            command_start_time: None,
        }
    }

    /// Parse a chunk of bytes and return a list of events.
    pub fn parse(&mut self, data: &[u8]) -> Vec<OscEvent> {
        let mut events = Vec::new();

        for &byte in data {
            if self.in_escape {
                if self.esc_buf.is_empty() {
                    if byte == b']' {
                        self.esc_buf.push(byte);
                    } else {
                        self.output_buf.push(0x1b);
                        self.output_buf.push(byte);
                        self.in_escape = false;
                    }
                } else {
                    self.esc_buf.push(byte);

                    let terminated = byte == 0x07
                        || (self.esc_buf.len() >= 2
                            && self.esc_buf[self.esc_buf.len() - 2] == 0x1b
                            && byte == b'\\');

                    if terminated {
                        if let Some(event) = self.parse_osc_sequence() {
                            if !self.output_buf.is_empty() {
                                events.push(OscEvent::Output(std::mem::take(&mut self.output_buf)));
                            }
                            events.push(event);
                        } else {
                            self.output_buf.push(0x1b);
                            self.output_buf.extend_from_slice(&self.esc_buf);
                        }
                        self.esc_buf.clear();
                        self.in_escape = false;
                    } else if self.esc_buf.len() > 1024 {
                        self.output_buf.push(0x1b);
                        self.output_buf.extend_from_slice(&self.esc_buf);
                        self.esc_buf.clear();
                        self.in_escape = false;
                    }
                }
            } else if byte == 0x1b {
                self.in_escape = true;
                self.esc_buf.clear();
            } else {
                self.output_buf.push(byte);
            }
        }

        if !self.output_buf.is_empty() {
            events.push(OscEvent::Output(std::mem::take(&mut self.output_buf)));
        }

        events
    }

    fn parse_osc_sequence(&self) -> Option<OscEvent> {
        let buf = &self.esc_buf;

        if buf.is_empty() || buf[0] != b']' {
            return None;
        }

        let payload_end = if buf[buf.len() - 1] == 0x07 {
            buf.len() - 1
        } else if buf.len() >= 2 && buf[buf.len() - 2] == 0x1b && buf[buf.len() - 1] == b'\\' {
            buf.len() - 2
        } else {
            return None;
        };

        let payload = &buf[1..payload_end];
        let payload_str = std::str::from_utf8(payload).ok()?;

        if let Some(rest) = payload_str.strip_prefix("133;") {
            return self.parse_osc_133(rest);
        }

        if let Some(rest) = payload_str.strip_prefix("7;") {
            return self.parse_osc_7(rest);
        }

        None
    }

    fn parse_osc_133(&self, payload: &str) -> Option<OscEvent> {
        match payload.chars().next()? {
            'A' => Some(OscEvent::Output(Vec::new())),
            'B' => Some(OscEvent::CommandStart),
            'C' => Some(OscEvent::Output(Vec::new())),
            'D' => {
                let exit_code = if payload.len() > 2 {
                    payload[2..].parse::<i32>().unwrap_or(0)
                } else {
                    0
                };
                Some(OscEvent::CommandEnd(exit_code))
            }
            _ => None,
        }
    }

    fn parse_osc_7(&self, payload: &str) -> Option<OscEvent> {
        if let Some(rest) = payload.strip_prefix("file://") {
            if let Some(slash_idx) = rest.find('/') {
                let path = &rest[slash_idx..];
                let decoded = url_decode(path);
                return Some(OscEvent::CwdChanged(decoded));
            }
        }
        None
    }
}

/// Decode percent-encoded URL strings, properly handling multi-byte UTF-8.
pub fn url_decode(input: &str) -> String {
    let mut bytes = Vec::with_capacity(input.len());
    let mut chars = input.as_bytes().iter();

    while let Some(&b) = chars.next() {
        if b == b'%' {
            let hex: Vec<u8> = chars.by_ref().take(2).copied().collect();
            if hex.len() == 2 {
                if let Ok(decoded) =
                    u8::from_str_radix(&String::from_utf8_lossy(&hex), 16)
                {
                    bytes.push(decoded);
                } else {
                    bytes.push(b'%');
                    bytes.extend_from_slice(&hex);
                }
            } else {
                bytes.push(b'%');
                bytes.extend_from_slice(&hex);
            }
        } else {
            bytes.push(b);
        }
    }

    String::from_utf8_lossy(&bytes).into_owned()
}
