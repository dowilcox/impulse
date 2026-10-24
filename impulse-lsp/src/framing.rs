//! JSON-RPC over stdio with `Content-Length` headers, as the language-server
//! protocol frames it.

use std::io::{self, BufRead, Write};

/// Write one message with its header.
pub fn write_message(out: &mut impl Write, body: &[u8]) -> io::Result<()> {
    write!(out, "Content-Length: {}\r\n\r\n", body.len())?;
    out.write_all(body)?;
    out.flush()
}

/// Read one message body. `Ok(None)` at the end of the stream; headers other
/// than `Content-Length` (such as `Content-Type`) are skipped.
pub fn read_message(input: &mut impl BufRead) -> io::Result<Option<Vec<u8>>> {
    let mut length: Option<usize> = None;
    let mut line = String::new();
    loop {
        line.clear();
        if input.read_line(&mut line)? == 0 {
            return Ok(None);
        }
        let header = line.trim_end_matches(['\r', '\n']);
        if header.is_empty() {
            if length.is_some() {
                break;
            }
            // Blank lines before a header: skip them.
            continue;
        }
        if let Some((name, value)) = header.split_once(':') {
            if name.trim().eq_ignore_ascii_case("content-length") {
                length = value.trim().parse().ok();
            }
        }
    }
    let length = length.unwrap_or(0);
    let mut body = vec![0; length];
    input.read_exact(&mut body)?;
    Ok(Some(body))
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
}
