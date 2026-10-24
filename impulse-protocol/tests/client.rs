//! The socket client against a real socket: one line each way, a missing
//! socket, and hanging up on a request that waits.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixListener;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use impulse_protocol::control;
use impulse_protocol::{ControlClient, ControlRequest, ControlResponse, Failure};

fn listener() -> (tempfile::TempDir, String, UnixListener) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir
        .path()
        .join("impulse.sock")
        .to_string_lossy()
        .into_owned();
    let listener = UnixListener::bind(&path).unwrap();
    (dir, path, listener)
}

#[test]
fn one_request_one_answer() {
    let (_dir, path, listener) = listener();
    let server = thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut line = Vec::new();
        reader.read_until(b'\n', &mut line).unwrap();
        let request: ControlRequest = control::decode(&line[..line.len() - 1]).unwrap();
        let mut stream = stream;
        let answer = ControlResponse::ok_message(format!("got {}", request.command));
        stream.write_all(&control::encode_line(&answer)).unwrap();
        String::from_utf8(line).unwrap()
    });
    let reply = ControlClient::new(path)
        .send(&ControlRequest::new("ping"))
        .unwrap();
    assert_eq!(reply.message.as_deref(), Some("got ping"));
    assert_eq!(
        server.join().unwrap(),
        "{\"arguments\":{},\"command\":\"ping\",\"wait\":false}\n"
    );
}

#[test]
fn a_missing_socket_is_unreachable_and_a_bad_answer_none() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir
        .path()
        .join("nothing.sock")
        .to_string_lossy()
        .into_owned();
    assert_eq!(
        ControlClient::new(missing).send(&ControlRequest::new("ping")),
        Err(Failure::Unreachable)
    );
    let (_dir, path, listener) = listener();
    thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream.write_all(b"not json\n").unwrap();
    });
    assert_eq!(
        ControlClient::new(path).send(&ControlRequest::new("ping")),
        Err(Failure::NoAnswer)
    );
}

#[test]
fn hanging_up_stops_a_waiting_request() {
    let (_dir, path, listener) = listener();
    let server = thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        // Read the request, never answer, and notice the hang-up.
        let mut reader = BufReader::new(stream);
        let mut line = Vec::new();
        reader.read_until(b'\n', &mut line).unwrap();
        let mut rest = Vec::new();
        reader.read_until(b'\n', &mut rest).unwrap()
    });
    let client = Arc::new(ControlClient::new(path));
    let canceller = Arc::clone(&client);
    thread::spawn(move || {
        thread::sleep(Duration::from_millis(200));
        canceller.cancel();
    });
    let mut request = ControlRequest::new("tasks");
    request.wait = true;
    assert_eq!(client.send(&request), Err(Failure::NoAnswer));
    assert_eq!(
        server.join().unwrap(),
        0,
        "the app sees the connection close"
    );
}

#[test]
fn a_client_cancelled_first_sends_nothing() {
    let (_dir, path, listener) = listener();
    let client = ControlClient::new(path);
    client.cancel();
    assert_eq!(
        client.send(&ControlRequest::new("ping")),
        Err(Failure::NoAnswer)
    );
    listener.set_nonblocking(true).unwrap();
    let (stream, _) = listener.accept().unwrap();
    let mut line = Vec::new();
    BufReader::new(stream).read_until(b'\n', &mut line).unwrap();
    assert!(line.is_empty());
}
