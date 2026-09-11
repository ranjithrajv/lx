use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

/// Spins up a tiny local HTTP/1.1 server that returns `statuses` in order,
/// one per connection, then closes. Returns the base URL and a hit counter.
fn spawn_server(statuses: Vec<u16>) -> (String, Arc<AtomicUsize>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let hits = Arc::new(AtomicUsize::new(0));
    let hits_clone = Arc::clone(&hits);
    std::thread::spawn(move || {
        for status in statuses {
            let Ok((mut stream, _)) = listener.accept() else {
                return;
            };
            let mut buf = [0u8; 1024];
            let _ = stream.read(&mut buf);
            hits_clone.fetch_add(1, Ordering::SeqCst);
            let reason = match status {
                200 => "OK",
                404 => "Not Found",
                503 => "Service Unavailable",
                _ => "Error",
            };
            let body = "{}";
            let resp = format!(
                "HTTP/1.1 {status} {reason}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(resp.as_bytes());
        }
    });
    (format!("http://{addr}"), hits)
}

#[test]
fn retries_on_5xx_then_succeeds() {
    let (base, hits) = spawn_server(vec![503, 503, 200]);
    let client = lx_lib::http::new_client().unwrap();
    let resp = lx_lib::http::send_get_with_retry(&client, &base, None).unwrap();
    assert_eq!(resp.status(), 200);
    assert_eq!(hits.load(Ordering::SeqCst), 3);
}

#[test]
fn does_not_retry_4xx() {
    let (base, hits) = spawn_server(vec![404]);
    let client = lx_lib::http::new_client().unwrap();
    let resp = lx_lib::http::send_get_with_retry(&client, &base, None).unwrap();
    assert_eq!(resp.status(), 404);
    assert_eq!(hits.load(Ordering::SeqCst), 1);
}
