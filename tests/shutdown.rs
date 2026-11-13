//! Finding R1-1: stopping the server while a page holds the event stream open. Real time.
//! The client is a plain blocking socket on its own thread, like a browser tab.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

use routing_console::app::App;
use routing_console::clock::Clock;
use routing_console::http;
use routing_console::store::Store;
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use tokio::time::timeout;

/// Connects, asks for the event stream, and returns the socket once the headers are in.
fn open_stream(addr: std::net::SocketAddr) -> (TcpStream, String) {
    let mut conn = TcpStream::connect(addr).unwrap();
    conn.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    conn.write_all(
        b"GET /api/events HTTP/1.1\r\nHost: localhost\r\nAccept: text/event-stream\r\n\r\n",
    )
    .unwrap();
    let mut got = Vec::new();
    let mut buf = [0u8; 1024];
    while !String::from_utf8_lossy(&got).contains("\r\n\r\n") {
        let n = conn.read(&mut buf).unwrap();
        assert!(n > 0, "closed before the headers");
        got.extend_from_slice(&buf[..n]);
    }
    (conn, String::from_utf8_lossy(&got).into_owned())
}

/// Reads until end of file; `Err` if the read timed out first.
fn read_to_end(mut conn: TcpStream) -> Result<String, String> {
    let mut rest = Vec::new();
    match conn.read_to_end(&mut rest) {
        Ok(_) => Ok(String::from_utf8_lossy(&rest).into_owned()),
        Err(e) => Err(format!(
            "{e}; read so far: {:?}",
            String::from_utf8_lossy(&rest)
        )),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn shutdown_ends_open_event_streams_and_the_server_stops() {
    let app = App::start_blank(Store::open_in_memory().unwrap(), Clock::system()).unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (stop, stopped) = oneshot::channel::<()>();
    let server = tokio::spawn(http::serve(listener, app, async {
        let _ = stopped.await;
    }));

    let (conn, head) = tokio::task::spawn_blocking(move || open_stream(addr))
        .await
        .unwrap();
    assert!(head.starts_with("HTTP/1.1 200"), "{head}");
    assert!(head.contains("text/event-stream"), "{head}");
    let client = tokio::task::spawn_blocking(move || read_to_end(conn));

    stop.send(()).unwrap();
    let done = timeout(Duration::from_secs(3), server).await;
    assert!(
        done.is_ok(),
        "the server was still running 3 s after shutdown began"
    );
    done.unwrap().unwrap().unwrap();

    // The client sees its stream end: end of file, after the chunked terminator.
    let tail = client.await.unwrap().expect("the client's stream ended");
    assert!(tail.ends_with("0\r\n\r\n"), "{tail:?}");
}
