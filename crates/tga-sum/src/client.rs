//! The model server, over loopback HTTP.
//!
//! **A port, never a host.** [`Client`] connects to `127.0.0.1:<port>` and
//! nothing else, so chat text cannot leave the machine through it whatever a
//! setting says. That is the whole privacy argument for this feature, and it is
//! the type's shape rather than a check.
//!
//! llama-server speaks plain HTTP/1.1: `Content-Length` on an ordinary answer,
//! `Transfer-Encoding: chunked` on a streamed one, server-sent events inside
//! that. Every request says `Connection: close`, so a response ends where the
//! socket does and there is no pool to keep honest.
//!
//! **Every read can be cancelled.** The socket's read timeout is short and
//! [`Patient`] retries it, checking the cancel flag and the overall deadline in
//! between. Dropping the socket is how llama-server learns to stop generating.

use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use serde_json::Value;

use crate::Error;

/// Long enough for a filtered port to fail, short enough that "is anything
/// there" does not hang the caller. A loopback connect either succeeds or is
/// refused at once; this only matters if something is wedged.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(3);

/// How often a blocked read wakes to look at the cancel flag.
const TICK: Duration = Duration::from_millis(250);

/// The longest one request may take, start to last byte. A part's prefill on
/// a card that has spilled into RAM, plus 3,000 tokens of answer at a few
/// tokens a second, fits inside it; a server that has stopped answering does
/// not wait forever.
const REQUEST_DEADLINE: Duration = Duration::from_secs(30 * 60);

/// The longest "are you there" and "what are you" may take. Both are answered
/// at once by a server that is up, loading or not; something on the port that
/// accepts and never answers would otherwise hold the caller for the whole
/// [`REQUEST_DEADLINE`], with Stop pressed and nothing reading it.
const PROBE_DEADLINE: Duration = Duration::from_secs(5);

/// What the server says it is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Props {
    /// The model file's name, `gemma-4-26B-A4B-it-qat-UD-Q4_K_XL.gguf`.
    pub model: String,
    /// The context of one slot, in tokens.
    pub n_ctx: usize,
}

/// The server's own count of one call.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Timings {
    pub prompt_tokens: usize,
    pub gen_tokens: usize,
    pub prefill_tps: f64,
    pub gen_tps: f64,
}

impl Timings {
    fn of(v: &Value) -> Option<Timings> {
        let t = v.get("timings")?;
        let n = |k: &str| t.get(k).and_then(Value::as_u64).unwrap_or(0) as usize;
        let f = |k: &str| t.get(k).and_then(Value::as_f64).unwrap_or(0.0);
        Some(Timings {
            prompt_tokens: n("prompt_n"),
            gen_tokens: n("predicted_n"),
            prefill_tps: f("prompt_per_second"),
            gen_tps: f("predicted_per_second"),
        })
    }
}

/// How a streamed answer ended.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Streamed {
    pub timings: Timings,
    /// The answer reached its `max_tokens` and was stopped there: whole as
    /// far as it goes, and not finished.
    pub cut: bool,
}

/// A llama-server on this machine.
#[derive(Debug, Clone)]
pub struct Client {
    port: u16,
}

impl Client {
    pub fn new(port: u16) -> Self {
        Client { port }
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    /// Where every request goes. Loopback, always.
    pub fn address(&self) -> SocketAddr {
        SocketAddr::from((Ipv4Addr::LOCALHOST, self.port))
    }

    /// Whether anything accepts a connection on the port at all — a server
    /// still loading its model does, and answers `/health` with 503.
    pub fn listening(&self) -> bool {
        TcpStream::connect_timeout(&self.address(), CONNECT_TIMEOUT).is_ok()
    }

    /// Whether a server is up and has finished loading its model. Bounded by
    /// [`PROBE_DEADLINE`].
    pub fn healthy(&self) -> bool {
        self.healthy_unless(&AtomicBool::new(false))
    }

    /// [`Client::healthy`], given up as soon as `cancel` is set: what a wait
    /// that the user can stop asks.
    pub fn healthy_unless(&self, cancel: &AtomicBool) -> bool {
        matches!(self.probe("/health", cancel), Ok(v) if v.get("status").and_then(Value::as_str) == Some("ok"))
    }

    /// What the server has loaded. Bounded by [`PROBE_DEADLINE`].
    pub fn props(&self) -> Result<Props, Error> {
        let never = AtomicBool::new(false);
        let v = self.probe("/props", &never)?;
        // `model_path` is the file it loaded; older builds name it only in
        // `/v1/models`, which is asked as a fallback.
        let path = v
            .get("model_path")
            .and_then(Value::as_str)
            .map(str::to_string)
            .or_else(|| {
                self.probe("/v1/models", &never).ok().and_then(|m| {
                    m.pointer("/data/0/id")
                        .and_then(Value::as_str)
                        .map(str::to_string)
                })
            })
            .unwrap_or_default();
        let model = path
            .rsplit(['/', '\\'])
            .next()
            .unwrap_or_default()
            .to_string();
        let n_ctx = v
            .pointer("/default_generation_settings/n_ctx")
            .or_else(|| v.get("n_ctx"))
            .and_then(Value::as_u64)
            .unwrap_or(0) as usize;
        Ok(Props { model, n_ctx })
    }

    /// How many tokens the model's own tokenizer makes of `text`.
    pub fn tokens(&self, text: &str, cancel: &AtomicBool) -> Result<usize, Error> {
        let body = serde_json::json!({ "content": text });
        let v = self.post("/tokenize", &body, cancel)?;
        v.get("tokens")
            .and_then(Value::as_array)
            .map(Vec::len)
            .ok_or_else(|| Error::Bad("/tokenize answered without tokens".into()))
    }

    /// One chat completion, whole. The answer's text and the server's timings.
    pub fn complete(&self, body: &Value, cancel: &AtomicBool) -> Result<(String, Timings), Error> {
        let v = self.post("/v1/chat/completions", body, cancel)?;
        let text = v
            .pointer("/choices/0/message/content")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        Ok((text, Timings::of(&v).unwrap_or_default()))
    }

    /// One chat completion, streamed: `on` gets each piece of text as the
    /// model writes it. `body` gets `"stream": true` added.
    ///
    /// **An answer is finished when the server says so** — a `finish_reason`,
    /// or `[DONE]` — and not when the socket goes quiet. A server stopped,
    /// crashed or out of memory mid-answer closes the connection exactly as a
    /// finished one does, and the text so far would otherwise be kept as the
    /// whole summary.
    pub fn stream(
        &self,
        body: &Value,
        cancel: &AtomicBool,
        on: &mut dyn FnMut(&str),
    ) -> Result<Streamed, Error> {
        let mut body = body.clone();
        body["stream"] = Value::Bool(true);
        let mut reader = self.request(
            "POST",
            "/v1/chat/completions",
            Some(&body),
            REQUEST_DEADLINE,
            cancel,
        )?;
        let mut out = Streamed::default();
        let mut finished = false;
        let mut line = String::new();
        loop {
            // A server that never pauses never lets `Patient` look, so the
            // flag is also read here, between events.
            if cancel.load(Ordering::Relaxed) {
                return Err(Error::Cancelled);
            }
            line.clear();
            match reader.read_line(&mut line) {
                Ok(0) => break,
                Ok(_) => {}
                // Said it was done, then dropped the socket without the
                // closing chunk: the answer is whole all the same.
                Err(_) if finished && !cancel.load(Ordering::Relaxed) => break,
                Err(e) => return Err(io_error(e, cancel)),
            }
            let Some(data) = line.trim_end().strip_prefix("data:") else {
                continue;
            };
            let data = data.trim();
            if data == "[DONE]" {
                finished = true;
                break;
            }
            let Ok(v) = serde_json::from_str::<Value>(data) else {
                continue;
            };
            if let Some(err) = v.get("error") {
                return Err(Error::Server(err.to_string()));
            }
            if let Some(piece) = v
                .pointer("/choices/0/delta/content")
                .and_then(Value::as_str)
            {
                if !piece.is_empty() {
                    on(piece);
                }
            }
            if let Some(reason) = v
                .pointer("/choices/0/finish_reason")
                .and_then(Value::as_str)
            {
                finished = true;
                out.cut |= reason == "length";
            }
            if let Some(t) = Timings::of(&v) {
                out.timings = t;
            }
        }
        if !finished {
            return Err(Error::Bad(
                "the model server stopped before the answer was finished".into(),
            ));
        }
        Ok(out)
    }

    /// A short question about the server itself, under [`PROBE_DEADLINE`].
    fn probe(&self, path: &str, cancel: &AtomicBool) -> Result<Value, Error> {
        self.read_json(
            self.request("GET", path, None, PROBE_DEADLINE, cancel)?,
            cancel,
        )
    }

    fn post(&self, path: &str, body: &Value, cancel: &AtomicBool) -> Result<Value, Error> {
        self.read_json(
            self.request("POST", path, Some(body), REQUEST_DEADLINE, cancel)?,
            cancel,
        )
    }

    fn read_json(
        &self,
        mut body: Box<dyn BufRead + '_>,
        cancel: &AtomicBool,
    ) -> Result<Value, Error> {
        let mut text = String::new();
        body.read_to_string(&mut text)
            .map_err(|e| io_error(e, cancel))?;
        serde_json::from_str(&text)
            .map_err(|e| Error::Bad(format!("not JSON from the server: {e}")))
    }

    /// Send one request; on a 2xx, the body to read. Anything else is an
    /// error carrying what the server said. `deadline` bounds the whole of
    /// it, start to last byte.
    fn request<'a>(
        &self,
        method: &str,
        path: &str,
        body: Option<&Value>,
        deadline: Duration,
        cancel: &'a AtomicBool,
    ) -> Result<Box<dyn BufRead + 'a>, Error> {
        if cancel.load(Ordering::Relaxed) {
            return Err(Error::Cancelled);
        }
        let mut socket = TcpStream::connect_timeout(&self.address(), CONNECT_TIMEOUT)
            .map_err(|_| Error::NoServer(self.port))?;
        socket.set_read_timeout(Some(TICK))?;
        socket.set_nodelay(true)?;
        let payload = body.map(|b| b.to_string()).unwrap_or_default();
        let head = format!(
            "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nAccept: */*\r\nConnection: close\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n",
            self.port,
            payload.len()
        );
        socket.write_all(head.as_bytes())?;
        socket.write_all(payload.as_bytes())?;
        socket.flush()?;

        let mut reader = BufReader::new(Patient {
            socket,
            cancel,
            deadline: Instant::now() + deadline,
        });
        let mut status_line = String::new();
        reader
            .read_line(&mut status_line)
            .map_err(|e| io_error(e, cancel))?;
        let status: u16 = status_line
            .split_whitespace()
            .nth(1)
            .and_then(|s| s.parse().ok())
            .ok_or_else(|| Error::Bad(format!("not HTTP: {}", status_line.trim())))?;
        let mut chunked = false;
        let mut length: Option<u64> = None;
        loop {
            let mut header = String::new();
            if reader
                .read_line(&mut header)
                .map_err(|e| io_error(e, cancel))?
                == 0
            {
                break;
            }
            let header = header.trim_end();
            if header.is_empty() {
                break;
            }
            if let Some((name, value)) = header.split_once(':') {
                let name = name.trim().to_ascii_lowercase();
                let value = value.trim();
                if name == "transfer-encoding" && value.eq_ignore_ascii_case("chunked") {
                    chunked = true;
                } else if name == "content-length" {
                    length = value.parse().ok();
                }
            }
        }
        let body: Box<dyn BufRead + 'a> = if chunked {
            Box::new(BufReader::new(Chunked::new(reader)))
        } else if let Some(n) = length {
            Box::new(reader.take(n))
        } else {
            Box::new(reader)
        };
        if !(200..300).contains(&status) {
            let mut said = String::new();
            let mut body = body;
            let _ = body.read_to_string(&mut said);
            // llama-server answers a refusal with `{"error": {"message": …}}`.
            let message = serde_json::from_str::<Value>(&said)
                .ok()
                .and_then(|v| {
                    v.pointer("/error/message")
                        .and_then(Value::as_str)
                        .map(str::to_string)
                })
                .unwrap_or(said);
            return Err(Error::Server(format!("{status}: {}", message.trim())));
        }
        Ok(body)
    }
}

/// A socket whose reads wake every [`TICK`] to look at the cancel flag and
/// the deadline, so neither a long prefill nor a dead server can hold the
/// caller past either.
struct Patient<'a> {
    socket: TcpStream,
    cancel: &'a AtomicBool,
    deadline: Instant,
}

impl Read for Patient<'_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        loop {
            match self.socket.read(buf) {
                Err(e)
                    if matches!(
                        e.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                    ) =>
                {
                    // **Not `Interrupted`.** `read_line` and `read_to_string`
                    // retry that kind by contract, so a Stop pressed while the
                    // server was still thinking spun here for good. The
                    // caller's `io_error` names it `Cancelled` by the flag.
                    if self.cancel.load(Ordering::Relaxed) {
                        return Err(io::Error::other("cancelled"));
                    }
                    if Instant::now() > self.deadline {
                        return Err(io::Error::new(
                            io::ErrorKind::TimedOut,
                            "the model server stopped answering",
                        ));
                    }
                }
                other => return other,
            }
        }
    }
}

fn io_error(e: io::Error, cancel: &AtomicBool) -> Error {
    if cancel.load(Ordering::Relaxed) {
        Error::Cancelled
    } else {
        Error::Io(e)
    }
}

/// `Transfer-Encoding: chunked`, decoded as it arrives — which is what lets a
/// streamed answer be shown while it is still being written.
pub(crate) struct Chunked<R> {
    inner: R,
    /// Bytes left in the chunk being read.
    left: u64,
    done: bool,
}

impl<R: BufRead> Chunked<R> {
    pub(crate) fn new(inner: R) -> Self {
        Chunked {
            inner,
            left: 0,
            done: false,
        }
    }
}

impl<R: BufRead> Read for Chunked<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.done || buf.is_empty() {
            return Ok(0);
        }
        if self.left == 0 {
            let mut size = String::new();
            // The body ends on a zero-size chunk. A socket closed before it
            // is a server that went away between two chunks, and the text so
            // far is not the whole answer.
            if self.inner.read_line(&mut size)? == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "the stream ended before its last chunk",
                ));
            }
            // A chunk size may carry `;extensions`; nobody here sends them.
            let hex = size.trim().split(';').next().unwrap_or_default();
            self.left = u64::from_str_radix(hex, 16)
                .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "bad chunk size"))?;
            if self.left == 0 {
                // The trailer: header lines up to an empty one.
                loop {
                    let mut line = String::new();
                    if self.inner.read_line(&mut line)? == 0 || line.trim().is_empty() {
                        break;
                    }
                }
                self.done = true;
                return Ok(0);
            }
        }
        let want = buf.len().min(self.left as usize);
        let n = self.inner.read(&mut buf[..want])?;
        if n == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "the stream ended inside a chunk",
            ));
        }
        self.left -= n as u64;
        if self.left == 0 {
            let mut crlf = String::new();
            self.inner.read_line(&mut crlf)?;
        }
        Ok(n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_request_goes_to_loopback() {
        assert!(Client::new(8080).address().ip().is_loopback());
        assert!(Client::new(1).address().ip().is_loopback());
    }

    #[test]
    fn a_chunked_body_decodes_across_chunks_and_ends_on_the_trailer() {
        let wire = b"5\r\nhello\r\n7;ext=1\r\n, world\r\n0\r\nX-Trailer: 1\r\n\r\n";
        let mut out = String::new();
        Chunked::new(&wire[..]).read_to_string(&mut out).unwrap();
        assert_eq!(out, "hello, world");
    }

    #[test]
    fn a_body_cut_inside_a_chunk_is_an_error_not_a_short_answer() {
        let wire = b"a\r\nhello";
        let mut out = String::new();
        assert!(Chunked::new(&wire[..]).read_to_string(&mut out).is_err());
    }

    #[test]
    fn a_body_cut_between_chunks_is_an_error_not_a_short_answer() {
        let wire = b"5\r\nhello\r\n";
        let mut out = String::new();
        let err = Chunked::new(&wire[..])
            .read_to_string(&mut out)
            .unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::UnexpectedEof);
    }

    #[test]
    fn a_port_that_accepts_and_never_answers_does_not_hold_a_probe() {
        // Bound and never served: the system accepts the connection for it,
        // and the request then waits on a reply that is not coming.
        let silent = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let client = Client::new(silent.local_addr().unwrap().port());

        let cancel = AtomicBool::new(false);
        let started = Instant::now();
        std::thread::scope(|s| {
            s.spawn(|| {
                std::thread::sleep(Duration::from_millis(300));
                cancel.store(true, Ordering::Relaxed);
            });
            assert!(!client.healthy_unless(&cancel));
        });
        assert!(started.elapsed() < Duration::from_secs(3), "Stop was heard");

        let started = Instant::now();
        assert!(!client.healthy());
        assert!(
            started.elapsed() < PROBE_DEADLINE + Duration::from_secs(2),
            "{:?}",
            started.elapsed()
        );
    }

    #[test]
    fn stop_is_heard_while_the_server_is_still_thinking() {
        // A long prefill: connected, sent, and nothing back yet. This spun
        // for good while a cancelled read said `Interrupted`.
        let silent = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let client = Client::new(silent.local_addr().unwrap().port());
        let cancel = AtomicBool::new(false);
        std::thread::scope(|s| {
            s.spawn(|| {
                std::thread::sleep(Duration::from_millis(300));
                cancel.store(true, Ordering::Relaxed);
            });
            assert!(matches!(client.tokens("x", &cancel), Err(Error::Cancelled)));
        });
    }

    #[test]
    fn nothing_listening_is_named_by_its_port() {
        // Port 1 is tcpmux; nothing on a workstation listens there.
        let never = AtomicBool::new(false);
        match Client::new(1).tokens("x", &never) {
            Err(Error::NoServer(1)) => {}
            other => panic!("{other:?}"),
        }
    }
}
