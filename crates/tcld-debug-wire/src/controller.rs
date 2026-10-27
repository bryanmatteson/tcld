//! Rust controller (frontend) library.
//!
//! Tier 4 Phase 4-C-Rust. Sync-blocking design: one reader thread
//! per connection, condvar-based serial waitlist for responses,
//! FIFO event queue for async events. Tauri / CLI / any Rust binary
//! consumes this directly without going through tclsh.
//!
//! The Tcl controller (Phase 4-C-Tcl) speaks the same wire format
//! defined in `crate::frame` / `crate::op` / `crate::event`; both
//! are first-class v2 deliverables.

use std::collections::{HashMap, VecDeque};
use std::io::{self, BufRead, BufReader, BufWriter, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::codec::{CodecError, decode_frame, encode_frame};
use crate::error::{ErrorCode, WireError};
use crate::event::WireEvent;
use crate::frame::{FrameKind, WireFrame};
use crate::handshake::{Hello, HelloAck, PROTOCOL_VERSION};
use crate::op::{Op, OpResponse};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControllerState {
    Idle,
    Listening,
    Accepting,
    Handshaking,
    Open,
    Closing,
}

#[derive(Debug)]
pub enum ControllerError {
    Io(io::Error),
    Codec(CodecError),
    Wire(WireError),
    HandshakeMismatch { expected: u32, got: u32 },
    HandshakeTimeout,
    ResponseTimeout,
    NotConnected,
    UnexpectedFrame(String),
}

impl std::fmt::Display for ControllerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "io: {e}"),
            Self::Codec(e) => write!(f, "codec: {e}"),
            Self::Wire(e) => write!(f, "wire error: {e}"),
            Self::HandshakeMismatch { expected, got } => {
                write!(
                    f,
                    "handshake protocol mismatch: expected v{expected}, got v{got}"
                )
            }
            Self::HandshakeTimeout => write!(f, "handshake timed out"),
            Self::ResponseTimeout => write!(f, "response timed out"),
            Self::NotConnected => write!(f, "not connected (state != Open)"),
            Self::UnexpectedFrame(s) => write!(f, "unexpected frame: {s}"),
        }
    }
}

impl std::error::Error for ControllerError {}

impl From<io::Error> for ControllerError {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

impl From<CodecError> for ControllerError {
    fn from(e: CodecError) -> Self {
        Self::Codec(e)
    }
}

/// Default timeout for `send` waiting on a response.
pub const DEFAULT_RESPONSE_TIMEOUT: Duration = Duration::from_secs(30);

/// Default timeout for the handshake.
pub const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(5);

struct SharedInbox {
    /// Pending responses keyed by serial. Reader thread fills; main
    /// thread drains via condvar.
    responses: HashMap<u64, Result<OpResponse, WireError>>,
    /// Async events FIFO.
    events: Vec<WireEvent>,
    /// True once the reader thread observes an EOF or error.
    closed: bool,
    /// Set on protocol violations so `send` can surface them.
    fatal: Option<ControllerError>,
}

struct ReaderShared {
    inbox: Mutex<SharedInbox>,
    cond: Condvar,
}

pub struct Controller {
    state: ControllerState,
    listener: Option<TcpListener>,
    bound_port: u16,
    writer: Option<Mutex<BufWriter<TcpStream>>>,
    reader: Option<JoinHandle<()>>,
    shared: Option<Arc<ReaderShared>>,
    next_serial: u64,
    nub_capabilities: Vec<String>,
    nub_child: Option<Child>,
}

/// One line the debugged program wrote to stdout or stderr.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NubOutput {
    pub stderr: bool,
    pub text: String,
}

/// Lines kept for a frontend that is not draining output; older lines are
/// dropped first.
const OUTPUT_BACKLOG: usize = 4096;

/// Startup diagnostics kept for [`NubChild::exited_stderr`].
const STDERR_CAPTURE: usize = 64 * 1024;

/// Handle to the spawned nub process. Drop SIGKILLs to prevent
/// orphaned tclsh procs in tests.
pub struct NubChild {
    child: Option<Child>,
    stderr_reader: Option<JoinHandle<Vec<u8>>>,
    stdout_reader: Option<JoinHandle<()>>,
    output: Arc<Mutex<VecDeque<NubOutput>>>,
}

impl NubChild {
    pub fn pid(&self) -> Option<u32> {
        self.child.as_ref().map(|c| c.id())
    }

    /// Take the lines the debugged program has written since the last call.
    pub fn drain_output(&self) -> Vec<NubOutput> {
        self.output
            .lock()
            .map(|mut output| output.drain(..).collect())
            .unwrap_or_default()
    }

    pub fn try_wait(&mut self) -> io::Result<Option<std::process::ExitStatus>> {
        match self.child.as_mut() {
            Some(c) => c.try_wait(),
            None => Ok(Some(unsafe { std::mem::zeroed() })),
        }
    }

    /// Return captured startup diagnostics after the nub has exited. A running
    /// nub returns an empty string so callers never block a live session.
    pub fn exited_stderr(&mut self) -> io::Result<String> {
        if self
            .child
            .as_mut()
            .is_some_and(|child| child.try_wait().ok().flatten().is_none())
        {
            return Ok(String::new());
        }
        let bytes = self
            .stderr_reader
            .take()
            .and_then(|reader| reader.join().ok())
            .unwrap_or_default();
        Ok(String::from_utf8_lossy(&bytes).trim().to_string())
    }

    /// Best-effort termination: graceful close (caller should already
    /// have sent `bye`), wait up to 5s, then SIGKILL.
    pub fn kill(&mut self) -> io::Result<()> {
        let Some(child) = self.child.as_mut() else {
            return Ok(());
        };
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            match child.try_wait()? {
                Some(_) => return Ok(()),
                None => {
                    if Instant::now() >= deadline {
                        break;
                    }
                    thread::sleep(Duration::from_millis(50));
                }
            }
        }
        let _ = child.kill();
        let _ = child.wait();
        if let Some(reader) = self.stderr_reader.take() {
            let _ = reader.join();
        }
        if let Some(reader) = self.stdout_reader.take() {
            let _ = reader.join();
        }
        Ok(())
    }
}

/// Forward each line of a child's pipe into `output`, returning the first
/// `capture` bytes. The pipe is read to its end either way, so a chatty
/// program never blocks on a full pipe.
fn forward_lines(
    pipe: impl Read,
    stderr: bool,
    output: Arc<Mutex<VecDeque<NubOutput>>>,
    capture: usize,
) -> Vec<u8> {
    let mut reader = BufReader::new(pipe);
    let mut captured = Vec::new();
    let mut line = Vec::new();
    loop {
        line.clear();
        match reader.read_until(b'\n', &mut line) {
            Ok(0) | Err(_) => break,
            Ok(_) => {}
        }
        let room = capture.saturating_sub(captured.len());
        captured.extend_from_slice(&line[..line.len().min(room)]);
        let text = String::from_utf8_lossy(&line)
            .trim_end_matches(['\n', '\r'])
            .to_string();
        if let Ok(mut queue) = output.lock() {
            if queue.len() >= OUTPUT_BACKLOG {
                queue.pop_front();
            }
            queue.push_back(NubOutput { stderr, text });
        }
    }
    captured
}

impl Drop for NubChild {
    fn drop(&mut self) {
        // Best effort; don't panic in Drop.
        let _ = self.kill();
    }
}

impl Controller {
    /// Bind a TCP listener. Address `0.0.0.0:0` lets the OS pick
    /// the port; the bound port is in the returned tuple.
    pub fn listen(addr: SocketAddr) -> Result<(Self, u16), ControllerError> {
        let listener = TcpListener::bind(addr)?;
        let port = listener.local_addr()?.port();
        Ok((
            Self {
                state: ControllerState::Listening,
                listener: Some(listener),
                bound_port: port,
                writer: None,
                reader: None,
                shared: None,
                next_serial: 1,
                nub_capabilities: Vec::new(),
                nub_child: None,
            },
            port,
        ))
    }

    pub fn status(&self) -> ControllerState {
        self.state
    }

    pub fn bound_port(&self) -> u16 {
        self.bound_port
    }

    pub fn nub_capabilities(&self) -> &[String] {
        &self.nub_capabilities
    }

    /// Spawn a nub-bearing tclsh subprocess wired up to connect back
    /// to this controller. Sets `TCLD_DBG_DYLIB`, `TCLD_NUB_HOST`, and
    /// `TCLD_NUB_PORT`. Target execution is requested later with
    /// [`Op::StartTarget`].
    ///
    /// The program's stdout and stderr are captured line by line for
    /// [`NubChild::drain_output`] rather than inherited: a frontend that owns
    /// the terminal must not have the program writing over it. Standard
    /// input is closed for the same reason, so a program reading it sees
    /// end-of-file instead of competing for keystrokes.
    pub fn spawn_nub(
        &mut self,
        tclsh: &Path,
        bootstrap: &Path,
        dylib: &Path,
    ) -> io::Result<NubChild> {
        let mut cmd = Command::new(tclsh);
        cmd.arg(bootstrap)
            .env("TCLD_DBG_DYLIB", dylib)
            .env("TCLD_NUB_HOST", "127.0.0.1")
            .env("TCLD_NUB_PORT", self.bound_port.to_string())
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = cmd.spawn()?;
        let output = Arc::new(Mutex::new(VecDeque::new()));
        let stderr_reader = child.stderr.take().map(|stderr| {
            let output = Arc::clone(&output);
            thread::spawn(move || forward_lines(stderr, true, output, STDERR_CAPTURE))
        });
        let stdout_reader = child.stdout.take().map(|stdout| {
            let output = Arc::clone(&output);
            thread::spawn(move || {
                forward_lines(stdout, false, output, 0);
            })
        });
        Ok(NubChild {
            child: Some(child),
            stderr_reader,
            stdout_reader,
            output,
        })
    }

    /// Block until a nub connects and the handshake completes.
    /// Returns the nub's reported capabilities.
    pub fn accept(&mut self) -> Result<Vec<String>, ControllerError> {
        self.accept_with_timeout(HANDSHAKE_TIMEOUT)
    }

    pub fn accept_with_timeout(
        &mut self,
        handshake_timeout: Duration,
    ) -> Result<Vec<String>, ControllerError> {
        let listener = self
            .listener
            .as_ref()
            .ok_or(ControllerError::NotConnected)?;
        self.state = ControllerState::Accepting;
        listener.set_nonblocking(true)?;
        let deadline = Instant::now() + handshake_timeout;
        let (stream, _peer) = loop {
            match listener.accept() {
                Ok(connection) => break connection,
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    if Instant::now() >= deadline {
                        listener.set_nonblocking(false)?;
                        return Err(ControllerError::HandshakeTimeout);
                    }
                    thread::sleep(Duration::from_millis(10));
                }
                Err(error) => {
                    listener.set_nonblocking(false)?;
                    return Err(error.into());
                }
            }
        };
        listener.set_nonblocking(false)?;
        stream.set_nonblocking(false)?;
        stream.set_read_timeout(Some(handshake_timeout))?;
        stream.set_write_timeout(Some(handshake_timeout))?;
        self.state = ControllerState::Handshaking;

        let mut reader = BufReader::new(stream.try_clone()?);
        let mut writer = BufWriter::new(stream.try_clone()?);

        // Read the nub's hello.
        let hello_frame = decode_frame(&mut reader)?;
        if hello_frame.kind != FrameKind::Hello {
            return Err(ControllerError::UnexpectedFrame(format!(
                "expected hello, got {}",
                hello_frame.kind.as_str()
            )));
        }
        let hello = Hello::from_payload(&hello_frame.payload).map_err(|e| {
            ControllerError::Wire(WireError::new(ErrorCode::BadHandshake, e.to_string()))
        })?;
        if hello.protocol_version != PROTOCOL_VERSION {
            return Err(ControllerError::HandshakeMismatch {
                expected: PROTOCOL_VERSION,
                got: hello.protocol_version,
            });
        }
        self.nub_capabilities = hello.capabilities;

        // Send hello_ack.
        let ack = HelloAck {
            controller_version: env!("CARGO_PKG_VERSION").to_string(),
            protocol_version: PROTOCOL_VERSION,
            capabilities: Vec::new(),
        };
        encode_frame(
            &mut writer,
            &WireFrame::new(FrameKind::HelloAck, 0, ack.to_payload()),
        )?;
        writer.flush()?;

        // Clear timeouts now that we're past the handshake — reader
        // will block on the long-running stream.
        let stream_for_reader = reader.into_inner();
        stream_for_reader.set_read_timeout(None)?;
        stream_for_reader.set_write_timeout(None)?;
        let mut reader = BufReader::new(stream_for_reader);

        // Spawn the reader thread.
        let shared = Arc::new(ReaderShared {
            inbox: Mutex::new(SharedInbox {
                responses: HashMap::new(),
                events: Vec::new(),
                closed: false,
                fatal: None,
            }),
            cond: Condvar::new(),
        });
        let reader_shared = shared.clone();
        let reader_handle = thread::spawn(move || {
            loop {
                let frame = match decode_frame(&mut reader) {
                    Ok(f) => f,
                    Err(CodecError::UnexpectedEof) => {
                        let mut g = reader_shared.inbox.lock().unwrap();
                        g.closed = true;
                        reader_shared.cond.notify_all();
                        return;
                    }
                    Err(e) => {
                        let mut g = reader_shared.inbox.lock().unwrap();
                        g.closed = true;
                        g.fatal = Some(ControllerError::Codec(e));
                        reader_shared.cond.notify_all();
                        return;
                    }
                };
                match frame.kind {
                    FrameKind::Resp => {
                        let mut g = reader_shared.inbox.lock().unwrap();
                        g.responses
                            .insert(frame.serial, Ok(OpResponse::from_payload(&frame.payload)));
                        reader_shared.cond.notify_all();
                    }
                    FrameKind::Err => {
                        let parsed = WireError::from_payload(&frame.payload).unwrap_or_else(|_| {
                            WireError::new(ErrorCode::ProtocolViolation, "malformed err payload")
                        });
                        let mut g = reader_shared.inbox.lock().unwrap();
                        g.responses.insert(frame.serial, Err(parsed));
                        reader_shared.cond.notify_all();
                    }
                    FrameKind::Evt => match WireEvent::from_payload(&frame.payload) {
                        Ok(ev) => {
                            let mut g = reader_shared.inbox.lock().unwrap();
                            g.events.push(ev);
                            reader_shared.cond.notify_all();
                        }
                        Err(error) => {
                            let mut g = reader_shared.inbox.lock().unwrap();
                            g.fatal = Some(ControllerError::Wire(WireError::new(
                                ErrorCode::ProtocolViolation,
                                format!("malformed event payload: {error}"),
                            )));
                            g.closed = true;
                            reader_shared.cond.notify_all();
                            return;
                        }
                    },
                    FrameKind::Bye => {
                        let mut g = reader_shared.inbox.lock().unwrap();
                        g.closed = true;
                        reader_shared.cond.notify_all();
                        return;
                    }
                    other => {
                        let mut g = reader_shared.inbox.lock().unwrap();
                        g.fatal = Some(ControllerError::UnexpectedFrame(format!(
                            "kind {} not valid post-handshake",
                            other.as_str()
                        )));
                        g.closed = true;
                        reader_shared.cond.notify_all();
                        return;
                    }
                }
            }
        });

        self.writer = Some(Mutex::new(writer));
        self.reader = Some(reader_handle);
        self.shared = Some(shared);
        self.state = ControllerState::Open;
        Ok(self.nub_capabilities.clone())
    }

    /// Send an op and block for the response. Returns the response
    /// payload or a wire error.
    pub fn send(&mut self, op: Op) -> Result<OpResponse, ControllerError> {
        self.send_with_timeout(op, DEFAULT_RESPONSE_TIMEOUT)
    }

    pub fn send_with_timeout(
        &mut self,
        op: Op,
        timeout: Duration,
    ) -> Result<OpResponse, ControllerError> {
        if self.state != ControllerState::Open {
            return Err(ControllerError::NotConnected);
        }
        let serial = self.next_serial;
        self.next_serial += 1;
        let frame = WireFrame::new(FrameKind::Req, serial, op.to_payload());
        {
            let writer = self
                .writer
                .as_ref()
                .ok_or(ControllerError::NotConnected)?
                .lock()
                .unwrap();
            // Mutex<BufWriter<TcpStream>> doesn't impl Write directly via guard;
            // need to write through a mut ref.
            let mut guard = writer;
            encode_frame(&mut *guard, &frame)?;
            guard.flush()?;
        }
        // Wait for the response.
        let shared = self.shared.as_ref().ok_or(ControllerError::NotConnected)?;
        let deadline = Instant::now() + timeout;
        let mut g = shared.inbox.lock().unwrap();
        loop {
            if let Some(r) = g.responses.remove(&serial) {
                return r.map_err(ControllerError::Wire);
            }
            if let Some(fatal) = g.fatal.take() {
                return Err(fatal);
            }
            if g.closed {
                return Err(ControllerError::NotConnected);
            }
            let now = Instant::now();
            if now >= deadline {
                return Err(ControllerError::ResponseTimeout);
            }
            let remaining = deadline - now;
            let (new_g, wait_result) = shared.cond.wait_timeout(g, remaining).unwrap();
            g = new_g;
            if wait_result.timed_out() && !g.responses.contains_key(&serial) {
                if g.closed {
                    return Err(ControllerError::NotConnected);
                }
                return Err(ControllerError::ResponseTimeout);
            }
        }
    }

    /// Drain any events received since the last call. Non-blocking.
    pub fn drain_events(&mut self) -> Vec<WireEvent> {
        let Some(shared) = self.shared.as_ref() else {
            return Vec::new();
        };
        let mut g = shared.inbox.lock().unwrap();
        std::mem::take(&mut g.events)
    }

    /// Block until at least one event is available, or `timeout`.
    pub fn next_event(&mut self, timeout: Duration) -> Result<WireEvent, ControllerError> {
        let shared = self.shared.as_ref().ok_or(ControllerError::NotConnected)?;
        let deadline = Instant::now() + timeout;
        let mut g = shared.inbox.lock().unwrap();
        loop {
            if !g.events.is_empty() {
                return Ok(g.events.remove(0));
            }
            if let Some(fatal) = g.fatal.take() {
                return Err(fatal);
            }
            if g.closed {
                return Err(ControllerError::NotConnected);
            }
            let now = Instant::now();
            if now >= deadline {
                return Err(ControllerError::ResponseTimeout);
            }
            let remaining = deadline - now;
            let (new_g, _) = shared.cond.wait_timeout(g, remaining).unwrap();
            g = new_g;
        }
    }

    /// Send a `bye` and tear down the connection. Reader thread joins.
    pub fn close(&mut self) -> Result<(), ControllerError> {
        self.state = ControllerState::Closing;
        if let Some(writer_mtx) = self.writer.take() {
            let mut writer = writer_mtx.into_inner().unwrap();
            let _ = encode_frame(&mut writer, &WireFrame::new(FrameKind::Bye, 0, ""));
            let _ = writer.flush();
            // Drop closes the stream.
            drop(writer);
        }
        if let Some(handle) = self.reader.take() {
            let _ = handle.join();
        }
        if let Some(child) = self.nub_child.as_mut() {
            let _ = child.kill();
            let _ = child.wait();
        }
        self.state = ControllerState::Idle;
        Ok(())
    }
}

impl Drop for Controller {
    fn drop(&mut self) {
        let _ = self.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpStream;

    #[test]
    fn v2_controller_rejects_v1_nub_during_hello() {
        let (mut controller, port) = Controller::listen("127.0.0.1:0".parse().unwrap()).unwrap();
        let peer = thread::spawn(move || {
            let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
            let hello = Hello {
                nub_version: "0.1.0".into(),
                tcl_version: "8.6.17".into(),
                protocol_version: 1,
                capabilities: Vec::new(),
            };
            encode_frame(
                &mut stream,
                &WireFrame::new(FrameKind::Hello, 0, hello.to_payload()),
            )
            .unwrap();
        });
        let error = controller.accept().err().unwrap();
        assert!(matches!(
            error,
            ControllerError::HandshakeMismatch {
                expected: PROTOCOL_VERSION,
                got: 1
            }
        ));
        peer.join().unwrap();
    }

    #[test]
    fn accept_timeout_includes_the_connect_phase() {
        let (mut controller, _) = Controller::listen("127.0.0.1:0".parse().unwrap()).unwrap();
        let started = Instant::now();
        let error = controller
            .accept_with_timeout(Duration::from_millis(30))
            .unwrap_err();
        assert!(matches!(error, ControllerError::HandshakeTimeout));
        assert!(started.elapsed() < Duration::from_secs(1));
    }
}
