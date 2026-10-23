// The WebSocket side of the Ayatsuri API: a listener thread and one thread
// per connection. Connections answer from the snapshot the editor refreshes
// every frame and write parameter overrides into a shared buffer the editor
// applies; nothing here touches the model directly, so a slow or hostile
// client can never stall a frame.

use std::collections::{HashMap, VecDeque};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::protocol::{self, Client, Conn, Snapshot, World};

/// How many messages the traffic log keeps.
const TRAFFIC_KEEP: usize = 500;
/// Longest message text the log stores; the rest is cut.
const TRAFFIC_TEXT: usize = 4000;

/// One message in the traffic log.
#[derive(Clone, Debug)]
pub(crate) struct Traffic {
    pub(crate) at: Instant,
    /// True for what a client sent, false for what we sent.
    pub(crate) incoming: bool,
    pub(crate) peer: String,
    pub(crate) method: String,
    /// Response, Event, Error or Request.
    pub(crate) kind: String,
    pub(crate) text: String,
}

/// More connections than any real setup needs; past this, new ones close.
const MAX_CONNS: usize = 16;
/// A frame larger than this is not a real request.
const MAX_MESSAGE: usize = 4 << 20;

/// One live connection as the editor's client list shows it.
#[derive(Clone, Debug)]
pub(crate) struct LiveConn {
    pub(crate) id: u64,
    pub(crate) peer: String,
    pub(crate) token: Option<String>,
}

/// An event on its way to one connection: (method, message text).
type EventMsg = (String, String);

/// State shared between the editor and the connection threads.
#[derive(Default)]
pub(crate) struct Shared {
    pub(crate) snap: Mutex<Snapshot>,
    pub(crate) clients: Mutex<Vec<Client>>,
    /// Set when a connection registered someone, so the editor saves them.
    pub(crate) clients_changed: AtomicBool,
    pub(crate) overrides: Mutex<HashMap<String, (f32, Instant)>>,
    /// Tokens waiting for the user's Allow / Deny.
    pub(crate) asks: Mutex<Vec<String>>,
    pub(crate) live: Mutex<Vec<LiveConn>>,
    /// Connections the user asked to drop.
    pub(crate) kick: Mutex<Vec<u64>>,
    /// The last messages both ways, for the API window.
    pub(crate) traffic: Mutex<VecDeque<Traffic>>,
    pub(crate) count_in: AtomicU64,
    pub(crate) count_out: AtomicU64,
    /// Per-connection event queues: (connection, sender).
    events: Mutex<Vec<(u64, Sender<EventMsg>)>>,
    stop: AtomicBool,
    next_id: AtomicU64,
}

pub(crate) struct Server {
    pub(crate) shared: Arc<Shared>,
    pub(crate) addr: SocketAddr,
    pub(crate) started: Instant,
}

/// Note a message in the traffic log, cheaply: the method and type come
/// out of a quick parse, the text is kept only up to a limit.
fn record(shared: &Shared, peer: &str, incoming: bool, text: &str) {
    let (method, kind) = match serde_json::from_str::<serde_json::Value>(text) {
        Ok(v) => (
            v.get("Method").and_then(|m| m.as_str()).unwrap_or("?").to_string(),
            v.get("Type").and_then(|m| m.as_str()).unwrap_or("?").to_string(),
        ),
        Err(_) => ("?".into(), "unparsed".into()),
    };
    // the log file gets every exchange but the every-frame parameter stream,
    // which would bury the rest
    if !matches!(method.as_str(), "SetParameterValues" | "GetParameterValues") {
        let dir = if incoming { "<-" } else { "->" };
        log::debug!("api {peer} {dir} {method} {kind}");
    }
    let text: String = text.chars().take(TRAFFIC_TEXT).collect();
    let counter = if incoming { &shared.count_in } else { &shared.count_out };
    counter.fetch_add(1, Ordering::Relaxed);
    let mut log = lock(&shared.traffic);
    if log.len() >= TRAFFIC_KEEP {
        log.pop_front();
    }
    log.push_back(Traffic {
        at: Instant::now(),
        incoming,
        peer: peer.to_string(),
        method,
        kind,
        text,
    });
}

/// A lock that a panicking thread poisoned is still usable data.
fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

impl Server {
    /// Listen on `port`, on localhost unless `remote` opens it to the network.
    pub(crate) fn start(port: u16, remote: bool, clients: Vec<Client>) -> std::io::Result<Self> {
        let host = if remote { [0, 0, 0, 0] } else { [127, 0, 0, 1] };
        let listener = TcpListener::bind(SocketAddr::from((host, port)))?;
        // accept blocks, so an idle server costs nothing; stop wakes it
        let addr = listener.local_addr()?;
        let shared = Arc::new(Shared { clients: Mutex::new(clients), ..Shared::default() });
        let s = shared.clone();
        std::thread::Builder::new().name("api-listen".into()).spawn(move || listen(listener, s))?;
        log::info!("Ayatsuri API listening on ws://{addr}");
        Ok(Server { shared, addr, started: Instant::now() })
    }

    /// Stop listening and drop every connection (they notice within ~0.1 s).
    pub(crate) fn stop(&self) {
        if self.shared.stop.swap(true, Ordering::Relaxed) {
            return;
        }
        // the listener sleeps in accept: one connection of our own wakes it
        // to see the flag
        let wake = SocketAddr::from(([127, 0, 0, 1], self.addr.port()));
        let _ = TcpStream::connect_timeout(&wake, Duration::from_millis(200));
        log::info!("Ayatsuri API stopped");
    }

    /// Send an event to every connection that subscribed to it.
    pub(crate) fn notify(&self, method: &str, data: serde_json::Value) {
        let text = protocol::event(method, data);
        let mut events = lock(&self.shared.events);
        // a closed connection's queue is gone; forget it
        events.retain(|(_, tx)| tx.send((method.to_string(), text.clone())).is_ok());
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop();
    }
}

fn listen(listener: TcpListener, shared: Arc<Shared>) {
    while !shared.stop.load(Ordering::Relaxed) {
        match listener.accept() {
            Ok(_) if shared.stop.load(Ordering::Relaxed) => break,
            Ok((stream, peer)) => {
                if lock(&shared.live).len() >= MAX_CONNS {
                    log::warn!("api: refused {peer}, {MAX_CONNS} connections already open");
                    continue;
                }
                let s = shared.clone();
                let spawned = std::thread::Builder::new()
                    .name("api-conn".into())
                    .spawn(move || serve(stream, peer, s));
                if let Err(e) = spawned {
                    log::error!("api: could not start a connection thread: {e}");
                }
            }
            Err(e) => {
                log::warn!("api: accept failed: {e}");
                std::thread::sleep(Duration::from_millis(200));
            }
        }
    }
}

fn serve(stream: TcpStream, peer: SocketAddr, shared: Arc<Shared>) {
    let id = shared.next_id.fetch_add(1, Ordering::Relaxed);
    // the handshake blocks; give it a bounded time
    let _ = stream.set_nonblocking(false);
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let config = tungstenite::protocol::WebSocketConfig::default()
        .max_message_size(Some(MAX_MESSAGE))
        .max_frame_size(Some(MAX_MESSAGE));
    let mut ws = match tungstenite::accept_with_config(stream, Some(config)) {
        Ok(ws) => ws,
        Err(e) => {
            log::debug!("api: handshake with {peer} failed: {e}");
            return;
        }
    };
    // from here, short read timeouts so events, stop and kick get a turn
    if let Err(e) = ws.get_ref().set_read_timeout(Some(Duration::from_millis(50))) {
        // without it the thread waits on reads and misses stop and kick
        log::warn!("api: {peer}: could not set the read timeout: {e}");
    }
    let (tx, rx): (Sender<EventMsg>, Receiver<EventMsg>) = channel();
    lock(&shared.events).push((id, tx));
    lock(&shared.live).push(LiveConn { id, peer: peer.to_string(), token: None });
    log::info!("api: {peer} connected");
    let mut conn = Conn::default();

    loop {
        if shared.stop.load(Ordering::Relaxed) || lock(&shared.kick).contains(&id) {
            let _ = ws.close(None);
            let _ = ws.flush();
            break;
        }
        while let Ok((method, text)) = rx.try_recv() {
            let allowed = conn
                .token
                .as_ref()
                .is_some_and(|t| lock(&shared.clients).iter().any(|c| &c.token == t && c.allowed));
            if allowed && conn.subscribed.contains(&method) {
                record(&shared, &peer.to_string(), false, &text);
                if ws.send(text.into()).is_err() {
                    break;
                }
            }
        }
        match ws.read() {
            Ok(tungstenite::Message::Text(text)) => {
                record(&shared, &peer.to_string(), true, text.as_str());
                let reply = {
                    let snap = lock(&shared.snap);
                    let mut clients = lock(&shared.clients);
                    let mut overrides = lock(&shared.overrides);
                    let mut asks = lock(&shared.asks);
                    let before = clients.len();
                    let mut world = World {
                        snap: &snap,
                        clients: &mut clients,
                        overrides: &mut overrides,
                        wants_approval: &mut asks,
                    };
                    let reply = protocol::handle(text.as_str(), &mut conn, &mut world);
                    if clients.len() != before || !asks.is_empty() {
                        shared.clients_changed.store(true, Ordering::Relaxed);
                    }
                    reply
                };
                if let Some(l) = lock(&shared.live).iter_mut().find(|l| l.id == id) {
                    l.token = conn.token.clone();
                }
                if let Some(r) = reply {
                    record(&shared, &peer.to_string(), false, &r);
                    if ws.send(r.into()).is_err() {
                        break;
                    }
                }
            }
            Ok(tungstenite::Message::Close(_)) => break,
            // pings are answered inside tungstenite; binary is not ours
            Ok(_) => {}
            Err(tungstenite::Error::Io(e))
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                // nothing arrived; flush anything queued (pong replies)
                let _ = ws.flush();
            }
            Err(e) => {
                log::debug!("api: {peer}: {e}");
                break;
            }
        }
    }
    lock(&shared.events).retain(|(c, _)| *c != id);
    lock(&shared.live).retain(|l| l.id != id);
    lock(&shared.kick).retain(|k| *k != id);
    log::info!("api: {peer} disconnected");
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A real socket round trip: register, get allowed, read the model.
    #[test]
    fn talks_websocket_on_localhost() {
        let server = Server::start(0, false, Vec::new()).unwrap();
        lock(&server.shared.snap).model_uid = Some("42".into());
        let url = format!("ws://{}", server.addr);
        let (mut ws, _) = tungstenite::connect(url).unwrap();
        let mut ask = |m: &str, data: &str| -> serde_json::Value {
            let msg = format!(
                r#"{{"Version":"1.0.0","RequestId":"1","Type":"Request","Method":"{m}","Data":{data}}}"#
            );
            ws.send(msg.into()).unwrap();
            loop {
                if let Ok(tungstenite::Message::Text(t)) = ws.read() {
                    return serde_json::from_str(t.as_str()).unwrap();
                }
            }
        };
        let reg = ask("RegisterPlugin", r#"{"Name":"test"}"#);
        assert!(reg["Data"]["Token"].is_string());
        assert_eq!(ask("GetIsApproval", "{}")["Data"]["Result"], false);
        lock(&server.shared.clients)[0].allowed = true;
        assert_eq!(ask("GetCurrentModelUID", "{}")["Data"]["ModelUID"], "42");
        assert_eq!(lock(&server.shared.live).len(), 1);
        // both directions are in the traffic log
        let log = lock(&server.shared.traffic);
        assert!(log.iter().any(|t| t.incoming && t.method == "RegisterPlugin"));
        assert!(log.iter().any(|t| !t.incoming && t.kind == "Response"));
        drop(log);
        server.stop();
    }
}
