//! The relay WebSocket client (`RelayClient.swift`): one connection to
//! `wss://<base>/m/<room>` on its own thread, pings, half-open detection,
//! and reconnection. It only moves frames; the `watch` thread owns state.

use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use base64::Engine as _;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use tungstenite::client::IntoClientRequest;
use tungstenite::handshake::HandshakeError;
use tungstenite::protocol::WebSocketConfig;
use tungstenite::stream::MaybeTlsStream;
use tungstenite::{Connector, Message, WebSocket};

use super::frame::{Assembler, Inbound};

pub(crate) const DEFAULT_BASE: &str = "https://relay.wristper.dev";
pub(crate) const PING_INTERVAL: Duration = Duration::from_secs(30);
pub(crate) const MAX_BACKOFF: Duration = Duration::from_secs(30);
pub(crate) const MAX_MESSAGE_BYTES: usize = 1024 * 1024;
pub(crate) const SECRET_HEADER: &str = "X-Wristper-Mac-Secret";

/// As `net`'s connect timeout; also bounds the proxy CONNECT and the
/// WebSocket handshake.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const ROOM_FILE: &str = "relay-room";
const SECRET_FILE: &str = "relay-secret";
const MAX_CONNECT_RESPONSE: usize = 8 * 1024;

type Socket = WebSocket<MaybeTlsStream<TcpStream>>;

/// `relay-room` and `relay-secret` beside Hex's other state; the files
/// Wristper wrote are reused unchanged.
#[derive(Clone)]
pub(crate) struct Room {
    pub(crate) room: String,
    pub(crate) secret: String,
}

/// Reads or creates both files in `support_dir` (`readOrCreate` in
/// `EmbeddedReceiver.swift`). The room is 16 random bytes and the secret 32,
/// each as unpadded base64url.
pub(crate) fn read_or_create_room(support_dir: &Path) -> io::Result<Room> {
    Ok(Room {
        room: read_or_create(&support_dir.join(ROOM_FILE), 22, 16)?,
        secret: read_or_create(&support_dir.join(SECRET_FILE), 32, 32)?,
    })
}

fn read_or_create(path: &Path, minimum_chars: usize, byte_count: usize) -> io::Result<String> {
    if let Ok(text) = std::fs::read_to_string(path) {
        let trimmed = text.trim();
        if trimmed.chars().count() >= minimum_chars {
            return Ok(trimmed.to_owned());
        }
    }
    let mut bytes = vec![0u8; byte_count];
    getrandom::fill(&mut bytes).map_err(io::Error::other)?;
    let value = URL_SAFE_NO_PAD.encode(&bytes);
    super::write_owner_only(path, format!("{value}\n").as_bytes())?;
    Ok(value)
}

/// `resolveBase`: an override needs a scheme and a host; release builds
/// accept only https and wss so frames and the room secret never travel in
/// the clear.
pub(crate) fn resolve_base(custom: &str, allow_insecure: bool) -> String {
    let trimmed = custom.trim();
    let Ok(url) = url::Url::parse(trimmed) else {
        return DEFAULT_BASE.to_owned();
    };
    if url.host_str().is_none_or(str::is_empty) {
        return DEFAULT_BASE.to_owned();
    }
    let allowed: &[&str] = if allow_insecure {
        &["https", "wss", "http", "ws"]
    } else {
        &["https", "wss"]
    };
    if allowed.contains(&url.scheme()) {
        trimmed.to_owned()
    } else {
        DEFAULT_BASE.to_owned()
    }
}

/// `socketURL`: `https://host[/path]` → `wss://host[/path]/m/<room>`.
pub(crate) fn socket_url(base: &str, room: &str) -> Option<String> {
    let mut url = url::Url::parse(base).ok()?;
    let scheme = match url.scheme() {
        "https" | "wss" => "wss",
        "http" | "ws" => "ws",
        _ => return None,
    };
    url.set_scheme(scheme).ok()?;
    let path = url
        .path()
        .strip_suffix('/')
        .unwrap_or(url.path())
        .to_owned();
    url.set_path(&format!("{path}/m/{room}"));
    Some(url.to_string())
}

/// 1, 2, 4 … seconds, at most `MAX_BACKOFF`.
pub(crate) fn backoff(attempt: u32) -> Duration {
    Duration::from_secs(1u64 << attempt.min(10)).min(MAX_BACKOFF)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RelayState {
    Connecting,
    Connected,
    Offline,
}

impl RelayState {
    /// As `/health` reports it.
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Connecting => "connecting",
            Self::Connected => "connected",
            Self::Offline => "offline",
        }
    }

    fn from_u8(value: u8) -> Self {
        match value {
            0 => Self::Connecting,
            1 => Self::Connected,
            _ => Self::Offline,
        }
    }

    fn as_u8(self) -> u8 {
        match self {
            Self::Connecting => 0,
            Self::Connected => 1,
            Self::Offline => 2,
        }
    }
}

/// What the relay thread tells the watch thread.
#[derive(Debug)]
pub(crate) enum RelayEvent {
    State(RelayState),
    /// Every frame the assembler produced except pongs.
    Inbound(Inbound),
    /// The relay closed us with 1000 `replaced`: another Mac took the room.
    /// `State(Offline)` follows, and the client keeps reconnecting.
    Replaced,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct RelayTiming {
    pub(crate) ping_interval: Duration,
    pub(crate) max_backoff: Duration,
    pub(crate) allow_insecure: bool,
}

impl Default for RelayTiming {
    fn default() -> Self {
        Self {
            ping_interval: PING_INTERVAL,
            max_backoff: MAX_BACKOFF,
            allow_insecure: crate::dev_build::is_dev_build(),
        }
    }
}

pub(crate) struct RelayConfig {
    pub(crate) base: String,
    pub(crate) room: Room,
    pub(crate) timing: RelayTiming,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct NotConnected;

impl std::fmt::Display for NotConnected {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("the relay is not connected")
    }
}

impl std::error::Error for NotConnected {}

/// The handle the watch thread keeps. Dropping it ends the thread without
/// waiting; `stop` also waits for it.
pub(crate) struct Relay {
    outgoing: Option<Sender<String>>,
    state: Arc<AtomicU8>,
    thread: Option<JoinHandle<()>>,
}

impl Relay {
    pub(crate) fn start(config: RelayConfig, events: Sender<RelayEvent>) -> Relay {
        let (outgoing, commands) = mpsc::channel();
        let state = Arc::new(AtomicU8::new(RelayState::Offline.as_u8()));
        let worker = Worker {
            config,
            events,
            commands,
            state: Arc::clone(&state),
        };
        let thread = std::thread::Builder::new()
            .name("watch-relay".to_owned())
            .spawn(move || worker.run())
            .map_err(|error| tracing::warn!(%error, "relay thread failed to start"))
            .ok();
        Relay {
            outgoing: Some(outgoing),
            state,
            thread,
        }
    }

    /// Queues a text frame for the socket.
    pub(crate) fn send(&self, text: String) -> Result<(), NotConnected> {
        if self.state() != RelayState::Connected {
            return Err(NotConnected);
        }
        self.outgoing
            .as_ref()
            .ok_or(NotConnected)?
            .send(text)
            .map_err(|_| NotConnected)
    }

    pub(crate) fn state(&self) -> RelayState {
        RelayState::from_u8(self.state.load(Ordering::Acquire))
    }

    /// Ends the thread and waits for it: up to half a second for the close
    /// handshake, or `CONNECT_TIMEOUT` while a connection attempt is under way.
    pub(crate) fn stop(mut self) {
        self.outgoing = None;
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for Relay {
    fn drop(&mut self) {
        // Closing the channel is the stop signal the thread polls for.
        self.outgoing = None;
    }
}

enum Ending {
    Lost,
    Stopped,
}

struct Worker {
    config: RelayConfig,
    events: Sender<RelayEvent>,
    commands: Receiver<String>,
    state: Arc<AtomicU8>,
}

impl Worker {
    fn run(self) {
        let Some(url) = socket_url(&self.config.base, &self.config.room.room) else {
            tracing::warn!(base = %self.config.base, "relay address is not valid");
            return;
        };
        let mut attempts = 0u32;
        loop {
            if !self.set_state(RelayState::Connecting) {
                return;
            }
            match connect(&url, &self.config.room.secret) {
                Ok((socket, raw)) => {
                    attempts = 0;
                    tracing::info!("relay connected");
                    if !self.set_state(RelayState::Connected) {
                        return;
                    }
                    if let Ending::Stopped = self.serve(socket, &raw) {
                        return;
                    }
                }
                Err(tungstenite::Error::Http(response)) if response.status() == 403 => {
                    tracing::warn!(
                        "relay refused this Mac's secret (403): another secret owns the room"
                    );
                }
                Err(error) => tracing::warn!(%error, "relay connection failed"),
            }
            if !self.set_state(RelayState::Offline) {
                return;
            }
            let delay = backoff(attempts).min(self.config.timing.max_backoff);
            attempts = attempts.saturating_add(1);
            if !self.wait(delay) {
                return;
            }
        }
    }

    /// False once the watch thread is gone.
    fn set_state(&self, state: RelayState) -> bool {
        let previous = self.state.swap(state.as_u8(), Ordering::AcqRel);
        if previous != state.as_u8() {
            tracing::info!(state = state.name(), "relay state");
        }
        self.events.send(RelayEvent::State(state)).is_ok()
    }

    /// Sleeps out a backoff; false when the handle asked to stop. Frames
    /// queued meanwhile were meant for the lost connection.
    fn wait(&self, delay: Duration) -> bool {
        let deadline = Instant::now() + delay;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            match self.commands.recv_timeout(left) {
                Ok(_) => self.drop_stale(1),
                Err(RecvTimeoutError::Timeout) => return true,
                Err(RecvTimeoutError::Disconnected) => return false,
            }
        }
    }

    fn drop_stale(&self, count: usize) {
        if count > 0 {
            tracing::warn!(
                count,
                "relay connection lost before replies went out; the watch re-queries /status"
            );
        }
    }

    fn serve(&self, mut socket: Socket, raw: &TcpStream) -> Ending {
        let interval = self.config.timing.ping_interval;
        let poll = (interval / 5).clamp(Duration::from_millis(5), Duration::from_millis(250));
        if let Err(error) = raw.set_read_timeout(Some(poll)) {
            tracing::warn!(%error, "relay socket rejected its read timeout");
            return Ending::Lost;
        }
        let mut assembler = Assembler::new();
        let mut last_heard = Instant::now();
        let mut next_tick = last_heard + interval;
        let ending = loop {
            if let Some(ending) = self.send_queued(&mut socket) {
                break ending;
            }
            let now = Instant::now();
            if now >= next_tick {
                next_tick = now + interval;
                let silent = now.duration_since(last_heard);
                if silent >= 2 * interval {
                    tracing::warn!(
                        silent_ms = silent.as_millis() as u64,
                        "relay heard nothing; reconnecting"
                    );
                    break Ending::Lost;
                }
                if let Err(error) = socket.send(Message::text("ping")) {
                    tracing::warn!(%error, "relay ping failed");
                    break Ending::Lost;
                }
            }
            match socket.read() {
                Ok(message) => {
                    last_heard = Instant::now();
                    let inbound = match message {
                        Message::Text(text) => assembler.receive_text(text.as_str()),
                        Message::Binary(bytes) => assembler.receive_binary(&bytes),
                        Message::Close(frame) => {
                            let replaced = frame.as_ref().is_some_and(|frame| {
                                u16::from(frame.code) == 1000 && frame.reason.as_str() == "replaced"
                            });
                            if replaced {
                                tracing::warn!("relay closed: another Mac took this room");
                                if self.events.send(RelayEvent::Replaced).is_err() {
                                    break Ending::Stopped;
                                }
                            } else {
                                tracing::info!(?frame, "relay closed the connection");
                            }
                            break Ending::Lost;
                        }
                        _ => Vec::new(),
                    };
                    if !self.forward(inbound) {
                        break Ending::Stopped;
                    }
                }
                Err(tungstenite::Error::Io(error)) if is_idle(&error) => {}
                Err(error) => {
                    tracing::warn!(%error, "relay connection lost");
                    break Ending::Lost;
                }
            }
        };
        match ending {
            Ending::Stopped => close(&mut socket),
            Ending::Lost => {
                let mut stale = 0;
                while self.commands.try_recv().is_ok() {
                    stale += 1;
                }
                self.drop_stale(stale);
            }
        }
        ending
    }

    /// Sends every queued frame; `Some` ends the connection.
    fn send_queued(&self, socket: &mut Socket) -> Option<Ending> {
        loop {
            match self.commands.try_recv() {
                Ok(text) => {
                    if let Err(error) = socket.send(Message::text(text)) {
                        tracing::warn!(%error, "relay send failed");
                        return Some(Ending::Lost);
                    }
                }
                Err(TryRecvError::Empty) => return None,
                Err(TryRecvError::Disconnected) => return Some(Ending::Stopped),
            }
        }
    }

    /// False once the watch thread is gone.
    fn forward(&self, inbound: Vec<Inbound>) -> bool {
        inbound
            .into_iter()
            .filter(|frame| !matches!(frame, Inbound::Pong))
            .all(|frame| self.events.send(RelayEvent::Inbound(frame)).is_ok())
    }
}

fn is_idle(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
    )
}

/// Says goodbye so the relay frees the room at once, waiting briefly for
/// its reply.
fn close(socket: &mut Socket) {
    if socket.close(None).is_err() {
        return;
    }
    let deadline = Instant::now() + Duration::from_millis(500);
    while Instant::now() < deadline {
        match socket.read() {
            Ok(_) => {}
            Err(tungstenite::Error::Io(error)) if is_idle(&error) => {}
            Err(_) => return,
        }
    }
}

/// Opens the socket, returning it with a handle on its TCP stream for
/// changing timeouts later.
fn connect(url: &str, secret: &str) -> Result<(Socket, TcpStream), tungstenite::Error> {
    let no_host = || tungstenite::Error::Url(tungstenite::error::UrlError::NoHostName);
    let parsed = url::Url::parse(url).map_err(|_| no_host())?;
    let secure = parsed.scheme() == "wss";
    let host = parsed.host_str().ok_or_else(no_host)?.to_owned();
    let port = parsed
        .port_or_known_default()
        .unwrap_or(if secure { 443 } else { 80 });
    let stream = if secure {
        let mut https = parsed.clone();
        let _ = https.set_scheme("https");
        match crate::net::proxy_for(https.as_str()) {
            Some(proxy) if proxy.protocol() == ureq::ProxyProtocol::Http => {
                tunnel(&proxy, &format!("{host}:{port}"))?
            }
            Some(proxy) => {
                tracing::warn!(
                    protocol = ?proxy.protocol(),
                    "relay can't use this proxy kind; connecting directly"
                );
                direct(&parsed, port)?
            }
            None => direct(&parsed, port)?,
        }
    } else {
        direct(&parsed, port)?
    };
    stream.set_read_timeout(Some(CONNECT_TIMEOUT))?;
    stream.set_write_timeout(Some(CONNECT_TIMEOUT))?;
    let raw = stream.try_clone()?;

    let mut request = url.into_client_request()?;
    let value = secret
        .parse()
        .map_err(|_| io::Error::other("relay secret is not a valid header value"))?;
    request.headers_mut().insert(SECRET_HEADER, value);
    let config = WebSocketConfig::default()
        .max_message_size(Some(MAX_MESSAGE_BYTES))
        .max_frame_size(Some(MAX_MESSAGE_BYTES));
    let connector = if secure {
        Some(Connector::NativeTls(
            native_tls::TlsConnector::new().map_err(|error| io::Error::other(error.to_string()))?,
        ))
    } else {
        None
    };
    match tungstenite::client_tls_with_config(request, stream, Some(config), connector) {
        Ok((socket, _)) => Ok((socket, raw)),
        Err(HandshakeError::Failure(error)) => Err(error),
        Err(HandshakeError::Interrupted(_)) => Err(tungstenite::Error::Io(io::Error::new(
            io::ErrorKind::TimedOut,
            "relay handshake timed out",
        ))),
    }
}

fn direct(url: &url::Url, port: u16) -> io::Result<TcpStream> {
    let addresses = url.socket_addrs(|| Some(port))?;
    connect_any(&addresses)
}

fn connect_any(addresses: &[SocketAddr]) -> io::Result<TcpStream> {
    let mut last = io::Error::new(io::ErrorKind::NotFound, "no address");
    for address in addresses {
        match TcpStream::connect_timeout(address, CONNECT_TIMEOUT) {
            Ok(stream) => return Ok(stream),
            Err(error) => last = error,
        }
    }
    Err(last)
}

/// An HTTP CONNECT tunnel through `proxy` to `authority` (`host:port`).
fn tunnel(proxy: &ureq::Proxy, authority: &str) -> io::Result<TcpStream> {
    use std::net::ToSocketAddrs;
    let addresses: Vec<SocketAddr> = (proxy.host(), proxy.port()).to_socket_addrs()?.collect();
    let mut stream = connect_any(&addresses)?;
    stream.set_read_timeout(Some(CONNECT_TIMEOUT))?;
    stream.set_write_timeout(Some(CONNECT_TIMEOUT))?;
    let mut request = format!("CONNECT {authority} HTTP/1.1\r\nHost: {authority}\r\n");
    if let Some(user) = proxy.username() {
        let credentials = STANDARD.encode(format!("{user}:{}", proxy.password().unwrap_or("")));
        request.push_str(&format!("Proxy-Authorization: Basic {credentials}\r\n"));
    }
    request.push_str("\r\n");
    stream.write_all(request.as_bytes())?;
    stream.flush()?;

    // Byte by byte, so nothing past the header (the TLS server hello) is
    // read away from the handshake.
    let mut response = Vec::new();
    let mut byte = [0u8; 1];
    while !response.ends_with(b"\r\n\r\n") {
        if response.len() >= MAX_CONNECT_RESPONSE {
            return Err(io::Error::other("proxy response header too long"));
        }
        if stream.read(&mut byte)? == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "proxy closed the connection",
            ));
        }
        response.push(byte[0]);
    }
    let status_line = String::from_utf8_lossy(&response);
    let status = status_line
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|code| code.parse::<u16>().ok());
    match status {
        Some(200..=299) => Ok(stream),
        Some(code) => Err(io::Error::other(format!(
            "proxy refused the tunnel ({code})"
        ))),
        None => Err(io::Error::other("proxy sent no status")),
    }
}

#[cfg(test)]
mod tests {
    use std::net::TcpListener;
    use std::path::PathBuf;

    use tungstenite::handshake::server::{Request, Response};
    use tungstenite::protocol::CloseFrame;
    use tungstenite::protocol::frame::coding::CloseCode;

    use super::*;
    use crate::watch::frame::response_frame;

    const WAIT: Duration = Duration::from_secs(3);
    const ROOM: &str = "AAAAAAAAAAAAAAAAAAAAAA";
    const SECRET: &str = "BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB";

    fn temp_dir(name: &str) -> PathBuf {
        let mut nonce = [0u8; 8];
        getrandom::fill(&mut nonce).unwrap();
        let dir = std::env::temp_dir().join(format!(
            "hex-watch-relay-{name}-{}-{:016x}",
            std::process::id(),
            u64::from_le_bytes(nonce)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn timing() -> RelayTiming {
        RelayTiming {
            ping_interval: Duration::from_millis(50),
            max_backoff: Duration::from_millis(20),
            allow_insecure: true,
        }
    }

    fn start(port: u16) -> (Relay, Receiver<RelayEvent>) {
        let (events, received) = mpsc::channel();
        let relay = Relay::start(
            RelayConfig {
                base: format!("ws://127.0.0.1:{port}"),
                room: Room {
                    room: ROOM.to_owned(),
                    secret: SECRET.to_owned(),
                },
                timing: timing(),
            },
            events,
        );
        (relay, received)
    }

    fn listener() -> (TcpListener, u16) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        (listener, port)
    }

    struct Accepted {
        socket: WebSocket<TcpStream>,
        path: String,
        secret: Option<String>,
    }

    #[expect(
        clippy::result_large_err,
        reason = "tungstenite's Callback fixes the error type"
    )]
    fn accept(listener: &TcpListener) -> Accepted {
        let (stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_millis(20)))
            .unwrap();
        let mut path = String::new();
        let mut secret = None;
        let socket = tungstenite::accept_hdr(stream, |request: &Request, response: Response| {
            path = request.uri().path().to_owned();
            secret = request
                .headers()
                .get(SECRET_HEADER)
                .and_then(|value| value.to_str().ok())
                .map(str::to_owned);
            Ok(response)
        })
        .unwrap();
        Accepted {
            socket,
            path,
            secret,
        }
    }

    /// The next message the fake relay reads, `None` once `within` passes.
    fn server_read(socket: &mut WebSocket<TcpStream>, within: Duration) -> Option<Message> {
        let deadline = Instant::now() + within;
        while Instant::now() < deadline {
            match socket.read() {
                Ok(message) => return Some(message),
                Err(tungstenite::Error::Io(error)) if is_idle(&error) => {}
                Err(_) => return None,
            }
        }
        None
    }

    fn next_event(events: &Receiver<RelayEvent>) -> RelayEvent {
        events.recv_timeout(WAIT).expect("relay event")
    }

    fn expect_state(events: &Receiver<RelayEvent>, state: RelayState) {
        match next_event(events) {
            RelayEvent::State(got) => assert_eq!(got, state),
            other => panic!("expected {state:?}, got {other:?}"),
        }
    }

    fn connected(listener: &TcpListener, events: &Receiver<RelayEvent>) -> Accepted {
        expect_state(events, RelayState::Connecting);
        let accepted = accept(listener);
        expect_state(events, RelayState::Connected);
        accepted
    }

    fn next_request(events: &Receiver<RelayEvent>) -> (super::super::frame::RequestHead, Vec<u8>) {
        match next_event(events) {
            RelayEvent::Inbound(Inbound::Request(head, body)) => (head, body),
            other => panic!("expected a request, got {other:?}"),
        }
    }

    #[test]
    fn resolve_base_accepts_only_secure_overrides_in_release() {
        assert_eq!(resolve_base("", false), DEFAULT_BASE);
        assert_eq!(resolve_base("   ", true), DEFAULT_BASE);
        assert_eq!(resolve_base("http://x", false), DEFAULT_BASE);
        assert_eq!(resolve_base("ws://x", false), DEFAULT_BASE);
        assert_eq!(resolve_base("http://x", true), "http://x");
        assert_eq!(resolve_base("ws://x:8787", true), "ws://x:8787");
        assert_eq!(resolve_base("wss://x", false), "wss://x");
        assert_eq!(
            resolve_base(" https://relay.example/base \n", false),
            "https://relay.example/base"
        );
        assert_eq!(resolve_base("relay.example", false), DEFAULT_BASE);
        assert_eq!(resolve_base("https://", false), DEFAULT_BASE);
        assert_eq!(resolve_base("ftp://x", true), DEFAULT_BASE);
    }

    #[test]
    fn socket_url_switches_scheme_and_appends_the_room() {
        assert_eq!(
            socket_url("https://relay.example", "room").as_deref(),
            Some("wss://relay.example/m/room")
        );
        assert_eq!(
            socket_url("https://relay.example/", "room").as_deref(),
            Some("wss://relay.example/m/room")
        );
        assert_eq!(
            socket_url("https://relay.example/base/", "room").as_deref(),
            Some("wss://relay.example/base/m/room")
        );
        assert_eq!(
            socket_url("http://127.0.0.1:8787", "room").as_deref(),
            Some("ws://127.0.0.1:8787/m/room")
        );
        assert_eq!(
            socket_url("wss://relay.example/x", "room").as_deref(),
            Some("wss://relay.example/x/m/room")
        );
        assert_eq!(
            socket_url("ws://relay.example", "room").as_deref(),
            Some("ws://relay.example/m/room")
        );
        assert_eq!(socket_url("ftp://relay.example", "room"), None);
        assert_eq!(socket_url("not a url", "room"), None);
    }

    #[test]
    fn backoff_doubles_up_to_thirty_seconds() {
        let seconds = |attempt| backoff(attempt).as_secs();
        assert_eq!(seconds(0), 1);
        assert_eq!(seconds(1), 2);
        assert_eq!(seconds(4), 16);
        assert_eq!(seconds(5), 30);
        assert_eq!(seconds(10), 30);
        assert_eq!(seconds(100), 30);
        assert_eq!(seconds(u32::MAX), 30);
    }

    #[test]
    fn state_names_match_health() {
        assert_eq!(RelayState::Connecting.name(), "connecting");
        assert_eq!(RelayState::Connected.name(), "connected");
        assert_eq!(RelayState::Offline.name(), "offline");
    }

    #[test]
    fn room_files_are_created_owner_only() {
        let dir = temp_dir("create");
        let room = read_or_create_room(&dir).unwrap();
        assert_eq!(room.room.len(), 22);
        assert_eq!(room.secret.len(), 43);
        for value in [&room.room, &room.secret] {
            assert!(
                value
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_'),
                "{value}"
            );
        }
        assert_eq!(
            std::fs::read_to_string(dir.join(ROOM_FILE)).unwrap(),
            format!("{}\n", room.room)
        );
        assert_eq!(
            std::fs::read_to_string(dir.join(SECRET_FILE)).unwrap(),
            format!("{}\n", room.secret)
        );
        #[cfg(unix)]
        for name in [ROOM_FILE, SECRET_FILE] {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(dir.join(name))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600, "{name}");
        }
        let again = read_or_create_room(&dir).unwrap();
        assert_eq!((again.room, again.secret), (room.room, room.secret));
    }

    #[test]
    fn room_files_written_by_wristper_are_reused_unchanged() {
        let dir = temp_dir("reuse");
        let room_text = "  abcdefghijklmnopqrstuv\n";
        let secret_text = "0123456789abcdefghijklmnopqrstuv\n";
        std::fs::write(dir.join(ROOM_FILE), room_text).unwrap();
        std::fs::write(dir.join(SECRET_FILE), secret_text).unwrap();
        let room = read_or_create_room(&dir).unwrap();
        assert_eq!(room.room, "abcdefghijklmnopqrstuv");
        assert_eq!(room.secret, "0123456789abcdefghijklmnopqrstuv");
        assert_eq!(
            std::fs::read_to_string(dir.join(ROOM_FILE)).unwrap(),
            room_text
        );
        assert_eq!(
            std::fs::read_to_string(dir.join(SECRET_FILE)).unwrap(),
            secret_text
        );
    }

    #[test]
    fn a_too_short_room_file_is_regenerated() {
        let dir = temp_dir("short");
        std::fs::write(dir.join(ROOM_FILE), "abcdefghijklmnopqrstu\n").unwrap();
        std::fs::write(dir.join(SECRET_FILE), "short\n").unwrap();
        let room = read_or_create_room(&dir).unwrap();
        assert_eq!(room.room.len(), 22);
        assert_ne!(room.room, "abcdefghijklmnopqrstu");
        assert_eq!(room.secret.len(), 43);
        assert_eq!(
            std::fs::read_to_string(dir.join(ROOM_FILE)).unwrap(),
            format!("{}\n", room.room)
        );
    }

    #[test]
    fn handshake_names_the_room_and_carries_the_secret() {
        let (listener, port) = listener();
        let (relay, events) = start(port);
        let accepted = connected(&listener, &events);
        assert_eq!(accepted.path, format!("/m/{ROOM}"));
        assert_eq!(accepted.secret.as_deref(), Some(SECRET));
        assert_eq!(relay.state(), RelayState::Connected);
        relay.stop();
    }

    #[test]
    fn a_request_arrives_and_its_response_goes_back() {
        let (listener, port) = listener();
        let (relay, events) = start(port);
        let mut server = connected(&listener, &events).socket;
        server
            .send(Message::text(
                r#"{"type":"req","id":"r1","method":"GET","path":"/health","query":{},"headers":{},"body_len":0}"#,
            ))
            .unwrap();
        let (head, body) = next_request(&events);
        assert_eq!((head.id.as_str(), head.path.as_str()), ("r1", "/health"));
        assert!(body.is_empty());

        let reply = response_frame("r1", 200, br#"{"ok":true}"#);
        relay.send(reply.clone()).unwrap();
        loop {
            match server_read(&mut server, WAIT) {
                Some(Message::Text(text)) if text.as_str() == "ping" => {}
                Some(Message::Text(text)) => {
                    assert_eq!(text.as_str(), reply);
                    break;
                }
                other => panic!("expected the response, got {other:?}"),
            }
        }
        relay.stop();
    }

    #[test]
    fn a_body_in_two_binary_chunks_is_one_request() {
        let (listener, port) = listener();
        let (relay, events) = start(port);
        let mut server = connected(&listener, &events).socket;
        server
            .send(Message::text(
                r#"{"type":"req","id":"r2","method":"POST","path":"/e2ee","query":{},"headers":{},"body_len":7}"#,
            ))
            .unwrap();
        server.send(Message::binary(b"abc".to_vec())).unwrap();
        server.send(Message::binary(b"defg".to_vec())).unwrap();
        let (head, body) = next_request(&events);
        assert_eq!(head.id, "r2");
        assert_eq!(body, b"abcdefg");
        relay.stop();
    }

    #[test]
    fn pings_answered_with_pong_keep_the_connection() {
        let (listener, port) = listener();
        listener.set_nonblocking(true).unwrap();
        let (relay, events) = start(port);
        expect_state(&events, RelayState::Connecting);
        let deadline = Instant::now() + WAIT;
        let stream = loop {
            match listener.accept() {
                Ok((stream, _)) => break stream,
                Err(_) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(5)),
                Err(error) => panic!("no connection: {error}"),
            }
        };
        stream.set_nonblocking(false).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_millis(20)))
            .unwrap();
        let mut server = tungstenite::accept(stream).unwrap();
        expect_state(&events, RelayState::Connected);

        let mut pings = 0;
        let until = Instant::now() + Duration::from_millis(300);
        while Instant::now() < until {
            if let Some(Message::Text(text)) = server_read(&mut server, Duration::from_millis(20))
                && text.as_str() == "ping"
            {
                pings += 1;
                server.send(Message::text("pong")).unwrap();
            }
        }
        assert!(pings >= 2, "pings: {pings}");
        assert!(events.try_recv().is_err(), "no state change while answered");
        assert!(listener.accept().is_err(), "no second connection");
        assert_eq!(relay.state(), RelayState::Connected);
        relay.stop();
    }

    #[test]
    fn a_silent_relay_is_dropped_and_reconnected() {
        let (listener, port) = listener();
        let (relay, events) = start(port);
        let _silent = connected(&listener, &events);
        let lost_after = Instant::now();
        expect_state(&events, RelayState::Offline);
        assert!(lost_after.elapsed() >= Duration::from_millis(50));
        let _second = connected(&listener, &events);
        relay.stop();
    }

    #[test]
    fn replaced_is_surfaced_and_the_client_reconnects() {
        let (listener, port) = listener();
        let (relay, events) = start(port);
        let mut server = connected(&listener, &events).socket;
        server
            .close(Some(CloseFrame {
                code: CloseCode::Normal,
                reason: "replaced".into(),
            }))
            .unwrap();
        assert!(matches!(next_event(&events), RelayEvent::Replaced));
        expect_state(&events, RelayState::Offline);
        assert!(relay.send("late".to_owned()).is_err());
        let _second = connected(&listener, &events);
        relay.stop();
    }

    #[test]
    fn send_while_offline_is_refused() {
        let (listener, port) = listener();
        drop(listener);
        let (relay, events) = start(port);
        assert_eq!(relay.send("x".to_owned()), Err(NotConnected));
        expect_state(&events, RelayState::Connecting);
        expect_state(&events, RelayState::Offline);
        assert_eq!(relay.send("x".to_owned()), Err(NotConnected));
        relay.stop();
    }

    #[test]
    fn stop_closes_the_socket_and_ends_the_thread() {
        let (listener, port) = listener();
        let (relay, events) = start(port);
        let mut server = connected(&listener, &events).socket;
        let (done, finished) = mpsc::channel();
        std::thread::spawn(move || {
            relay.stop();
            let _ = done.send(());
        });
        let saw_close = loop {
            match server_read(&mut server, WAIT) {
                Some(Message::Close(_)) => break true,
                Some(_) => {}
                None => break false,
            }
        };
        assert!(saw_close, "the relay saw the close");
        finished.recv_timeout(WAIT).expect("thread ended");
    }

    #[test]
    fn a_proxy_tunnel_sends_connect_with_credentials() {
        let (listener, port) = listener();
        let proxy = ureq::Proxy::new(&format!("http://user:pw@127.0.0.1:{port}")).unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = Vec::new();
            let mut byte = [0u8; 1];
            while !request.ends_with(b"\r\n\r\n") {
                stream.read_exact(&mut byte).unwrap();
                request.push(byte[0]);
            }
            stream
                .write_all(b"HTTP/1.1 200 Connection established\r\n\r\nhello")
                .unwrap();
            String::from_utf8(request).unwrap()
        });
        let mut stream = tunnel(&proxy, "relay.example:443").unwrap();
        let mut rest = [0u8; 5];
        stream.read_exact(&mut rest).unwrap();
        assert_eq!(&rest, b"hello", "bytes after the header stay on the stream");
        let request = server.join().unwrap();
        assert!(
            request.starts_with("CONNECT relay.example:443 HTTP/1.1\r\n"),
            "{request}"
        );
        assert!(request.contains("Host: relay.example:443\r\n"), "{request}");
        assert!(
            request.contains(&format!(
                "Proxy-Authorization: Basic {}\r\n",
                STANDARD.encode("user:pw")
            )),
            "{request}"
        );
    }

    #[test]
    fn a_proxy_refusal_fails_the_tunnel() {
        let (listener, port) = listener();
        let proxy = ureq::Proxy::new(&format!("http://127.0.0.1:{port}")).unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = Vec::new();
            let mut byte = [0u8; 1];
            while !request.ends_with(b"\r\n\r\n") {
                stream.read_exact(&mut byte).unwrap();
                request.push(byte[0]);
            }
            stream
                .write_all(b"HTTP/1.1 407 Proxy Authentication Required\r\n\r\n")
                .unwrap();
            String::from_utf8(request).unwrap()
        });
        let error = tunnel(&proxy, "relay.example:443").unwrap_err();
        assert!(error.to_string().contains("407"), "{error}");
        assert!(!server.join().unwrap().contains("Proxy-Authorization"));
    }
}
