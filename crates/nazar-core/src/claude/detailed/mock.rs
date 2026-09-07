//! A one-file HTTP server, so that no test ever touches the real endpoint.
//!
//! Hand-rolled on `std::net::TcpListener` rather than pulled in as a development
//! dependency, for the same reason `testutil::TempDir` is hand-rolled: this crate ships in
//! a product whose whole claim is a short dependency list, and eighty lines of `read` and
//! `write` is cheaper to audit than a crate.
//!
//! It binds `127.0.0.1:0`, so the operating system picks a free port and two tests can
//! never collide. It answers a fixed script — one canned response per request, in order —
//! and records what it was asked, which is how the leak test proves the token really did
//! travel (and the header test proves it travelled with the right company).

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

/// One canned answer.
#[derive(Debug, Clone)]
pub struct Canned {
    /// The status line's code.
    pub status: u16,
    /// Extra headers, as `(name, value)`.
    pub headers: Vec<(String, String)>,
    /// The body.
    pub body: String,
}

impl Canned {
    /// `200 OK` with a JSON body.
    pub fn ok(body: impl Into<String>) -> Self {
        Canned {
            status: 200,
            headers: vec![("Content-Type".to_owned(), "application/json".to_owned())],
            body: body.into(),
        }
    }

    /// A status with an empty body.
    #[must_use]
    pub fn status(status: u16) -> Self {
        Canned {
            status,
            headers: Vec::new(),
            body: String::new(),
        }
    }

    /// A status with a body — used to prove the body never reaches an error message.
    pub fn status_with_body(status: u16, body: impl Into<String>) -> Self {
        Canned {
            status,
            headers: Vec::new(),
            body: body.into(),
        }
    }

    /// Add a header.
    #[must_use]
    pub fn header(mut self, name: &str, value: &str) -> Self {
        self.headers.push((name.to_owned(), value.to_owned()));
        self
    }
}

/// One request as the server saw it: the request line and the headers, lower-cased names.
#[derive(Debug, Clone, Default)]
pub struct Seen {
    /// `GET /api/oauth/usage HTTP/1.1`.
    pub request_line: String,
    /// Headers, in the order they arrived, names lower-cased.
    pub headers: Vec<(String, String)>,
}

impl Seen {
    /// The value of one header, if it was sent.
    #[must_use]
    pub fn header(&self, name: &str) -> Option<&str> {
        let name = name.to_ascii_lowercase();
        self.headers
            .iter()
            .find(|(key, _)| *key == name)
            .map(|(_, value)| value.as_str())
    }
}

/// A server that answers a fixed script and then stops.
pub struct MockServer {
    url: String,
    seen: Arc<Mutex<Vec<Seen>>>,
    worker: Option<JoinHandle<()>>,
}

impl MockServer {
    /// Start a server that answers `script` in order, then closes.
    ///
    /// A request beyond the end of the script gets `503`, which is a failure a test can
    /// notice rather than a hang it has to time out.
    pub fn start(script: Vec<Canned>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("a loopback port must be bindable");
        let port = listener
            .local_addr()
            .expect("the socket must have an address")
            .port();
        let seen = Arc::new(Mutex::new(Vec::new()));

        let worker = {
            let seen = Arc::clone(&seen);
            std::thread::spawn(move || {
                for (index, connection) in listener.incoming().enumerate() {
                    let Ok(stream) = connection else { break };
                    let answer = script.get(index).cloned().unwrap_or_else(|| {
                        Canned::status_with_body(503, "the mock server ran out of script")
                    });
                    if let Some(request) = serve(stream, &answer) {
                        seen.lock()
                            .expect("the record must not be poisoned")
                            .push(request);
                    }
                    if index + 1 >= script.len() {
                        break;
                    }
                }
            })
        };

        MockServer {
            url: format!("http://127.0.0.1:{port}/api/oauth/usage"),
            seen,
            worker: Some(worker),
        }
    }

    /// A server that answers one thing.
    pub fn once(answer: Canned) -> Self {
        MockServer::start(vec![answer])
    }

    /// The URL a client should be pointed at.
    #[must_use]
    pub fn url(&self) -> &str {
        &self.url
    }

    /// Every request the server has answered so far, in order.
    #[must_use]
    pub fn requests(&self) -> Vec<Seen> {
        self.seen
            .lock()
            .expect("the record must not be poisoned")
            .clone()
    }
}

impl Drop for MockServer {
    fn drop(&mut self) {
        // The worker stops on its own once the script is exhausted. A test that never made
        // the last request would otherwise leave a thread blocked on `accept`, so the
        // handle is detached rather than joined: a leaked loopback listener lives until
        // the test binary exits, and joining could hang the suite.
        drop(self.worker.take());
    }
}

/// Read one request and write one answer.
fn serve(mut stream: TcpStream, answer: &Canned) -> Option<Seen> {
    let mut reader = BufReader::new(stream.try_clone().ok()?);

    let mut request_line = String::new();
    reader.read_line(&mut request_line).ok()?;

    let mut headers = Vec::new();
    let mut content_length = 0usize;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).ok()? == 0 {
            break;
        }
        let line = line.trim_end_matches(['\r', '\n']);
        if line.is_empty() {
            break;
        }
        if let Some((name, value)) = line.split_once(':') {
            let name = name.trim().to_ascii_lowercase();
            let value = value.trim().to_owned();
            if name == "content-length" {
                content_length = value.parse().unwrap_or(0);
            }
            headers.push((name, value));
        }
    }
    // A GET has no body, but draining one keeps the connection well-behaved if it ever
    // grows one.
    if content_length > 0 {
        let mut body = vec![0u8; content_length.min(64 * 1024)];
        let _ = reader.read_exact(&mut body);
    }

    let reason = match answer.status {
        200 => "OK",
        401 => "Unauthorized",
        403 => "Forbidden",
        429 => "Too Many Requests",
        500 => "Internal Server Error",
        503 => "Service Unavailable",
        _ => "Status",
    };
    let mut response = format!("HTTP/1.1 {} {reason}\r\n", answer.status);
    for (name, value) in &answer.headers {
        response.push_str(&format!("{name}: {value}\r\n"));
    }
    response.push_str(&format!("Content-Length: {}\r\n", answer.body.len()));
    response.push_str("Connection: close\r\n\r\n");
    response.push_str(&answer.body);

    stream.write_all(response.as_bytes()).ok()?;
    stream.flush().ok()?;

    Some(Seen {
        request_line: request_line.trim_end().to_owned(),
        headers,
    })
}
