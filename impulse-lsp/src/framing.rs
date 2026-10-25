//! JSON-RPC over stdio with `Content-Length` headers, as the language-server
//! protocol frames it.

use std::io::{self, BufRead, Read, Write};

/// Write one message with its header.
pub fn write_message(out: &mut impl Write, body: &[u8]) -> io::Result<()> {
    write!(out, "Content-Length: {}\r\n\r\n", body.len())?;
    out.write_all(body)?;
    out.flush()
}

/// The largest message read (larger ones are skipped), and the limits on
/// a header line and on headers per message (the Swift `ServerProcess`'s):
/// a broken or hostile server can't make the app allocate without bound.
pub const MAX_MESSAGE_SIZE: usize = 32 * 1024 * 1024;
const MAX_HEADER_LINE: u64 = 8 * 1024;
const MAX_HEADER_COUNT: usize = 32;

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.to_string())
}

/// Read one message body. `Ok(None)` at the end of the stream; headers other
/// than `Content-Length` (such as `Content-Type`) are skipped, and so are
/// messages over `MAX_MESSAGE_SIZE`. A header line or header count past its
/// limit is an error (the connection is given up on).
pub fn read_message(input: &mut impl BufRead) -> io::Result<Option<Vec<u8>>> {
    loop {
        let mut length: Option<usize> = None;
        let mut headers = 0;
        let mut line = String::new();
        loop {
            line.clear();
            let read = input.by_ref().take(MAX_HEADER_LINE).read_line(&mut line)?;
            if read == 0 {
                return Ok(None);
            }
            if !line.ends_with('\n') && read as u64 >= MAX_HEADER_LINE {
                return Err(invalid("header line too long"));
            }
            let header = line.trim_end_matches(['\r', '\n']);
            if header.is_empty() {
                if length.is_some() {
                    break;
                }
                // Blank lines before a header: skip them.
                continue;
            }
            headers += 1;
            if headers > MAX_HEADER_COUNT {
                return Err(invalid("too many headers"));
            }
            if let Some((name, value)) = header.split_once(':') {
                if name.trim().eq_ignore_ascii_case("content-length") {
                    length = value.trim().parse().ok();
                }
            }
        }
        let length = length.unwrap_or(0);
        if length > MAX_MESSAGE_SIZE {
            // Read past it without keeping it.
            let skipped = io::copy(&mut input.by_ref().take(length as u64), &mut io::sink())?;
            if skipped < length as u64 {
                return Err(io::ErrorKind::UnexpectedEof.into());
            }
            continue;
        }
        let mut body = vec![0; length];
        input.read_exact(&mut body)?;
        return Ok(Some(body));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_messages() {
        let mut out = Vec::new();
        write_message(&mut out, br#"{"a":1}"#).unwrap();
        write_message(&mut out, "{\"é\":2}".as_bytes()).unwrap();
        let mut input = io::Cursor::new(out);
        assert_eq!(read_message(&mut input).unwrap().unwrap(), br#"{"a":1}"#);
        assert_eq!(
            read_message(&mut input).unwrap().unwrap(),
            "{\"é\":2}".as_bytes()
        );
        assert_eq!(read_message(&mut input).unwrap(), None);
    }

    #[test]
    fn skips_other_headers_and_reads_by_bytes() {
        let raw = b"Content-Type: application/vscode-jsonrpc; charset=utf-8\r\ncontent-length: 4\r\n\r\n\xc3\xa9{}";
        let mut input = io::Cursor::new(&raw[..]);
        assert_eq!(read_message(&mut input).unwrap().unwrap(), b"\xc3\xa9{}");
    }

    #[test]
    fn truncated_body_is_an_error() {
        let mut input = io::Cursor::new(&b"Content-Length: 10\r\n\r\n{}"[..]);
        assert!(read_message(&mut input).is_err());
    }

    #[test]
    fn oversized_messages_are_skipped() {
        let big = MAX_MESSAGE_SIZE + 1;
        let mut raw = format!("Content-Length: {big}\r\n\r\n").into_bytes();
        raw.resize(raw.len() + big, b' ');
        raw.extend_from_slice(b"Content-Length: 2\r\n\r\n{}");
        let mut input = io::Cursor::new(raw);
        assert_eq!(read_message(&mut input).unwrap().unwrap(), b"{}");
    }

    #[test]
    fn endless_headers_are_an_error() {
        let raw = vec![b'x'; 20_000];
        assert!(read_message(&mut io::Cursor::new(raw)).is_err());
        let many = "X-A: 1\r\n".repeat(40);
        assert!(read_message(&mut io::Cursor::new(many.into_bytes())).is_err());
    }
}
