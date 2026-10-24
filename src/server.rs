//! Relay server for online matches.
//!
//! The server of the original game is not part of the SWF; this one implements what the client expects:
//! a player list with challenges, forwarding of the in-game messages to the opponent, `pong` for `ping`,
//! chat, surrender, and win/draw bookkeeping (`won/lost/drawn` as the `skill` of a player).

use crate::proto::*;
use crate::grid::{stage_center, TILE};
use crate::world::{World, BOMB_FUSE_MS, DT_MS};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::{self, BufReader, Read, Write};
use std::net::{IpAddr, Shutdown, SocketAddr, TcpListener, TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, SyncSender};
#[cfg(test)]
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};
use tungstenite::protocol::WebSocketConfig;
use tungstenite::Message;

pub const DEFAULT_PORT: u16 = 6900;
pub const MAX_PLAYERS: usize = 100;
/// Clients send `<beat/>` every 15 s when otherwise silent.
pub const IDLE_TIMEOUT: Duration = Duration::from_secs(45);
const AUTH_TIMEOUT: Duration = Duration::from_secs(10);
const WRITE_TIMEOUT: Duration = Duration::from_secs(2);
const MAX_CONNECTIONS: usize = 128;
const MAX_CONNECTIONS_PER_IP: usize = 32;
const OUTGOING_CAPACITY: usize = 256;
const MAX_RECORDS: usize = 10000;
/// Two deaths this close together are a draw (the second player died before the first `die` reached it).
pub const DRAW_WINDOW: Duration = Duration::from_millis(1500);
const MAX_MSGS_PER_SECOND: u32 = 300;
/// How long a WebSocket connection waits for the socket before it looks at its outgoing queue again.
const WS_POLL: Duration = Duration::from_millis(5);

struct Client {
    tx: SyncSender<Vec<u8>>,
    socket: Option<Arc<TcpStream>>,
    challenges: HashSet<String>,
    challenge_all: bool,
    opponent: Option<String>,
    in_game: bool,
    died_at: Option<Instant>,
    result_done: bool,
    rematch_requested: bool,
    match_host: Option<String>,
}

impl Client {
    fn send(&self, frame: Vec<u8>) {
        if self.tx.try_send(frame).is_err() {
            if let Some(socket) = &self.socket {
                let _ = socket.shutdown(Shutdown::Both);
            }
        }
    }
}

#[derive(Default)]
pub struct State {
    clients: BTreeMap<String, Client>,
    matches: HashMap<String, MatchState>,
    /// won, lost, drawn per player name; inactive records are pruned at MAX_RECORDS.
    stats: HashMap<String, (u32, u32, u32)>,
}

struct MatchState {
    names: [String; 2],
    world: Option<World>,
    clock: Instant,
    movement_at: [Instant; 2],
    movement_credit: [f64; 2],
}

impl MatchState {
    fn new(host: &str, guest: &str, now: Instant) -> Self {
        Self { names: [host.into(), guest.into()], world: None, clock: now,
            movement_at: [now; 2], movement_credit: [80.0; 2] }
    }

    fn accept(&mut self, name: &str, message: &Msg, now: Instant) -> bool {
        if !message.valid_gameplay() {
            return false;
        }
        let Some(player) = self.names.iter().position(|candidate| candidate == name) else { return false };
        if let Msg::Seed(seed) = message {
            if player != 0 || self.world.is_some() {
                return false;
            }
            let mut world = World::new(*seed);
            world.local = Some(2);
            self.world = Some(world);
            self.clock = now;
            self.movement_at = [now; 2];
            return true;
        }
        if matches!(message, Msg::Stats { .. }) {
            return true;
        }
        let Some(world) = self.world.as_mut() else { return false };
        let elapsed = now.saturating_duration_since(self.clock).as_millis().min(i64::MAX as u128) as i64;
        let target = elapsed;
        for _ in 0..200 {
            if world.now + DT_MS > target { break; }
            world.now += DT_MS;
            world.update_objects();
        }
        if world.now + DT_MS <= target {
            world.now = target;
            world.update_objects();
        }
        world.events.clear();
        let command = match message {
            Msg::Move(cmd) | Msg::Bomb { cmd, .. } | Msg::Pickup { cmd, .. }
            | Msg::Kick { cmd, .. } | Msg::Stop { cmd, .. } | Msg::Detonate { cmd, .. } => Some(cmd),
            _ => None,
        };
        if let Some(cmd) = command {
            let pawn = &world.players[player];
            let credit = (self.movement_credit[player]
                + now.saturating_duration_since(self.movement_at[player]).as_secs_f64() * pawn.speed * 1000.0).min(1000.0);
            let distance = (pawn.x - cmd.x as f64).hypot(pawn.y - cmd.y as f64);
            if distance > credit || pawn.dead {
                return false;
            }
            let steps = (distance / 8.0).ceil().max(1.0) as usize;
            for step in 1..=steps {
                let fraction = step as f64 / steps as f64;
                let x = pawn.x + (cmd.x as f64 - pawn.x) * fraction;
                let y = pawn.y + (cmd.y as f64 - pawn.y) * fraction;
                let Some((cx, cy, cell)) = world.stone_at_px(x, y) else { return false };
                if !cell.can_enter() && !((cx, cy) == (pawn.cx, pawn.cy)
                    && cell.kind == crate::grid::Kind::ArmedBomb) { return false; }
            }
            self.movement_credit[player] = credit - distance;
            self.movement_at[player] = now;
        }
        let nearby = |cx, cy, cmd: &Cmd, distance: f64| {
            let (x, y) = stage_center(cx, cy);
            (x - cmd.x as f64).hypot(y - cmd.y as f64) <= distance
        };
        let accepted = match message {
            Msg::Move(_) => true,
            Msg::Bomb { cmd, cx, cy } => nearby(*cx, *cy, cmd, TILE)
                && world.remote_place_bomb(*cx, *cy, player),
            Msg::Pickup { cmd, cx, cy, speed, blast } => nearby(*cx, *cy, cmd, TILE)
                && world.remote_pickup(*cx, *cy, player, *speed, *blast),
            Msg::Kick { cmd, cx, cy, id, dx, dy, t } => nearby(*cx, *cy, cmd, TILE * 2.0)
                && world.bomb_index_at(*cx, *cy).is_some_and(|index| {
                    let remaining = (BOMB_FUSE_MS - (world.now - world.bombs[index].start)).clamp(0, BOMB_FUSE_MS);
                    (remaining - *t).abs() <= 750
                })
                && world.remote_kick(*cx, *cy, player, *dx, *dy, *t, *id),
            Msg::Stop { cmd, id, cx, cy, t } => {
                if !nearby(*cx, *cy, cmd, TILE * 2.0) { return false; }
                let Some(index) = world.bombs.iter().position(|bomb| !bomb.exploded && bomb.id == Some(*id)
                    && bomb.moving && bomb.kicked_by != Some(player)) else { return false };
                let remaining = (BOMB_FUSE_MS - (world.now - world.bombs[index].start)).clamp(0, BOMB_FUSE_MS);
                if (remaining - *t).abs() > 750 { return false; }
                world.remote_stop(*id, *cx, *cy, *t);
                if world.bombs[index].moving { return false; }
                world.bombs[index].kicked_by = Some(player);
                true
            }
            Msg::Detonate { id, cx, cy, .. } => {
                let Some(index) = world.bombs.iter().position(|bomb| !bomb.exploded && bomb.id == Some(*id)
                    && bomb.kicked_by == Some(player)) else { return false };
                if world.now + 500 < world.bombs[index].start + BOMB_FUSE_MS && !world.flame_at(*cx, *cy) { return false; }
                world.remote_detonate(*id, *cx, *cy);
                world.bombs[index].exploded
            }
            Msg::Die { x, y } => {
                if world.players[player].dead { return false; }
                world.remote_die(player, *x as f64, *y as f64);
                true
            }
            _ => false,
        };
        if accepted {
            if let Some(cmd) = command {
                let pawn = &mut world.players[player];
                pawn.remote_command(cmd.keys(), cmd.x as f64, cmd.y as f64);
                pawn.cx = crate::grid::stage_x_to_cell(pawn.x);
                pawn.cy = crate::grid::stage_y_to_cell(pawn.y);
            }
        }
        accepted
    }
}

impl State {
    fn skill(&self, name: &str) -> String {
        let (w, l, d) = self.stats.get(name).copied().unwrap_or_default();
        format!("{w}/{l}/{d}")
    }

    fn info(&self, name: &str) -> PlayerInfo {
        let in_game = self.clients.get(name).is_some_and(|c| c.in_game);
        PlayerInfo { name: name.to_string(), skill: self.skill(name), state: if in_game { "3" } else { "0" }.to_string() }
    }

    fn send(&self, to: &str, m: &Msg) {
        if let Some(c) = self.clients.get(to) {
            c.send(m.to_frame());
        }
    }

    fn broadcast(&self, m: &Msg, except: Option<&str>) {
        for (n, c) in &self.clients {
            if Some(n.as_str()) != except {
                c.send(m.to_frame());
            }
        }
    }

    pub fn player_count(&self) -> usize {
        self.clients.len()
    }

    /// Registers an authenticated player. `Err` is the text for the `errorMsg`.
    pub fn join(&mut self, name: &str, tx: SyncSender<Vec<u8>>) -> Result<(), String> {
        if !valid_name(name) {
            return Err("Ungültiger Name.".into());
        }
        if self.clients.contains_key(name) {
            return Err("Der Name ist bereits vergeben.".into());
        }
        if self.clients.len() >= MAX_PLAYERS {
            return Err("Der Server ist voll.".into());
        }
        if self.stats.len() >= MAX_RECORDS {
            self.stats.retain(|name, _| self.clients.contains_key(name));
        }
        self.clients.insert(
            name.to_string(),
            Client {
                tx,
                socket: None,
                challenges: HashSet::new(),
                challenge_all: false,
                opponent: None,
                in_game: false,
                died_at: None,
                result_done: false,
                rematch_requested: false,
                match_host: None,
            },
        );
        let list: Vec<PlayerInfo> = self.clients.keys().map(|n| self.info(n)).collect();
        self.send(name, &Msg::UserList(list));
        self.broadcast(&Msg::NewPlayer(self.info(name)), Some(name));
        // players who challenged everybody also challenge the newcomer
        for (n, c) in &self.clients {
            if c.challenge_all && n != name {
                self.send(name, &Msg::Request(n.clone()));
            }
        }
        Ok(())
    }

    pub fn leave(&mut self, name: &str) {
        self.surrender(name);
        if let Some(host) = self.clients.get(name).and_then(|client| client.match_host.clone()) {
            self.matches.remove(&host);
        }
        if self.clients.remove(name).is_none() {
            return;
        }
        for c in self.clients.values_mut() {
            c.challenges.remove(name);
            if c.opponent.as_deref() == Some(name) {
                c.opponent = None;
            }
        }
        self.broadcast(&Msg::PlayerLeft(name.to_string()), None);
    }

    pub fn handle(&mut self, me: &str, m: Msg, now: Instant) {
        if !self.clients.contains_key(me) {
            return;
        }
        match m {
            Msg::Challenge(t) => {
                if t != me && self.clients.contains_key(&t) {
                    self.clients.get_mut(me).unwrap().challenges.insert(t.clone());
                    self.send(&t, &Msg::Request(me.to_string()));
                }
            }
            Msg::RemChallenge(t) => {
                if self.clients.get_mut(me).unwrap().challenges.remove(&t) {
                    self.send(&t, &Msg::RemRequest(me.to_string()));
                }
            }
            Msg::ChallengeAll => {
                self.clients.get_mut(me).unwrap().challenge_all = true;
                self.broadcast(&Msg::Request(me.to_string()), Some(me));
            }
            Msg::RemChallengeAll => {
                if std::mem::take(&mut self.clients.get_mut(me).unwrap().challenge_all) {
                    self.broadcast(&Msg::RemRequest(me.to_string()), Some(me));
                }
            }
            Msg::StartGame(t) => self.start_game(me, &t),
            Msg::MsgAll { msg, .. } => {
                let msg = clean_chat(&msg);
                if msg.is_empty() {
                    return;
                }
                let out = Msg::MsgAll { name: me.to_string(), msg };
                for (n, c) in &self.clients {
                    if !c.in_game || n == me {
                        c.send(out.to_frame());
                    }
                }
            }
            Msg::MsgPlayer { msg, .. } => {
                let c = &self.clients[me];
                let msg = clean_chat(&msg);
                if msg.is_empty() || !c.in_game {
                    return;
                }
                let out = Msg::MsgPlayer { name: me.to_string(), msg };
                self.send(me, &out);
                if let Some(o) = c.opponent.clone() {
                    self.send(&o, &out);
                }
            }
            Msg::ToRoom => self.leave_match(me),
            Msg::PlayAgain => {
                if let Some(o) = self.opponent_of(me) {
                    if self.clients[me].result_done && self.clients[&o].result_done {
                        self.clients.get_mut(me).unwrap().rematch_requested = true;
                        self.send(&o, &Msg::PlayAgain);
                    }
                }
            }
            Msg::Ping => self.send(me, &Msg::Pong),
            Msg::Surrender(_) => self.surrender(me),
            Msg::WinGame => self.win_reported(me),
            Msg::DrawGame => self.draw_reported(me),
            m if m.is_relay() => {
                let Some(o) = self.opponent_of(me) else { return };
                if self.clients[me].result_done {
                    return;
                }
                let Some(host) = self.clients[me].match_host.clone() else { return };
                if !self.matches.get_mut(&host).is_some_and(|game| game.accept(me, &m, now)) {
                    return;
                }
                if matches!(m, Msg::Die { .. }) {
                    self.clients.get_mut(me).unwrap().died_at = Some(now);
                    let other_died = self.clients[&o].died_at.is_some_and(|t| now.saturating_duration_since(t) <= DRAW_WINDOW);
                    self.send(&o, &m);
                    if other_died {
                        self.send(me, &Msg::Draw);
                        self.send(&o, &Msg::Draw);
                    }
                } else {
                    self.send(&o, &m);
                }
            }
            _ => {}
        }
    }

    /// Opponent of a player who is in a match.
    fn opponent_of(&self, me: &str) -> Option<String> {
        let c = self.clients.get(me)?;
        if c.in_game {
            c.opponent.clone()
        } else {
            None
        }
    }

    /// `startGame name=t` from `me`: accepts the challenge of `t` (then `me` is player 1) or starts a rematch.
    fn start_game(&mut self, me: &str, t: &str) {
        let (Some(a), Some(b)) = (self.clients.get(me), self.clients.get(t)) else { return };
        let rematch = a.opponent.as_deref() == Some(t) && b.opponent.as_deref() == Some(me);
        if rematch && (!a.result_done || !b.result_done || !b.rematch_requested) {
            return;
        }
        if !rematch {
            let challenged = b.challenges.contains(me) || b.challenge_all;
            if me == t || a.in_game || b.in_game || !challenged {
                return;
            }
            for (n, other) in [(me, t), (t, me)] {
                let c = self.clients.get_mut(n).unwrap();
                let pending: Vec<String> = c.challenges.drain().filter(|x| x != other).collect();
                let all = std::mem::take(&mut c.challenge_all);
                for x in pending {
                    self.send(&x, &Msg::RemRequest(n.to_string()));
                }
                if all {
                    for (x, cl) in &self.clients {
                        if x != n && x != other {
                            cl.send(Msg::RemRequest(n.to_string()).to_frame());
                        }
                    }
                }
            }
            for (n, other) in [(me, t), (t, me)] {
                let c = self.clients.get_mut(n).unwrap();
                c.in_game = true;
                c.opponent = Some(other.to_string());
            }
            self.broadcast(&Msg::PlayerUpdate(self.info(me)), None);
            self.broadcast(&Msg::PlayerUpdate(self.info(t)), None);
        }
        for n in [me, t] {
            let c = self.clients.get_mut(n).unwrap();
            c.died_at = None;
            c.result_done = false;
            c.rematch_requested = false;
        }
        if let Some(host) = self.clients[me].match_host.clone() {
            self.matches.remove(&host);
        }
        self.matches.insert(me.into(), MatchState::new(me, t, Instant::now()));
        for name in [me, t] {
            self.clients.get_mut(name).unwrap().match_host = Some(me.into());
        }
        self.send(t, &Msg::StartGame(me.to_string()));
    }

    fn leave_match(&mut self, me: &str) {
        self.surrender(me);
        if let Some(host) = self.clients[me].match_host.clone() {
            self.matches.remove(&host);
        }
        let c = self.clients.get_mut(me).unwrap();
        c.match_host = None;
        c.in_game = false;
        c.died_at = None;
        c.rematch_requested = false;
        let opp = c.opponent.take();
        if let Some(o) = opp {
            if let Some(oc) = self.clients.get_mut(&o) {
                if oc.opponent.as_deref() == Some(me) {
                    oc.opponent = None;
                }
            }
        }
        self.broadcast(&Msg::PlayerUpdate(self.info(me)), None);
    }

    /// Records a finished match once; `winner` is `None` for a draw.
    fn finish(&mut self, a: &str, b: &str, winner: Option<&str>) {
        for n in [a, b] {
            self.clients.get_mut(n).unwrap().result_done = true;
            let s = self.stats.entry(n.to_string()).or_default();
            match winner {
                Some(w) if w == n => s.0 = s.0.saturating_add(1),
                Some(_) => s.1 = s.1.saturating_add(1),
                None => s.2 = s.2.saturating_add(1),
            }
        }
        let end = Msg::EndGame { winner: winner.unwrap_or("").to_string() };
        for n in [a, b] {
            self.send(n, &end);
            self.broadcast(&Msg::PlayerUpdate(self.info(n)), None);
        }
    }

    fn surrender(&mut self, me: &str) {
        let Some(o) = self.opponent_of(me) else { return };
        if self.clients[me].result_done {
            return;
        }
        let m = Msg::Surrender(Some(o.clone()));
        self.send(me, &m);
        self.send(&o, &m);
        self.finish(me, &o, Some(&o));
    }

    /// The winner reports at the end of its 3 s game-over delay; the loser must really have died.
    fn win_reported(&mut self, me: &str) {
        let Some(o) = self.opponent_of(me) else { return };
        let (cm, co) = (&self.clients[me], &self.clients[&o]);
        if cm.result_done || cm.died_at.is_some() || co.died_at.is_none() {
            return;
        }
        self.finish(me, &o, Some(me));
    }

    fn draw_reported(&mut self, me: &str) {
        let Some(o) = self.opponent_of(me) else { return };
        let (cm, co) = (&self.clients[me], &self.clients[&o]);
        if cm.result_done || cm.died_at.is_none() || co.died_at.is_none() {
            return;
        }
        self.finish(me, &o, None);
    }
}

fn lock(state: &Mutex<State>) -> std::sync::MutexGuard<'_, State> {
    state.lock().unwrap_or_else(|e| e.into_inner())
}

/// Native clients speak zero-terminated XML over TCP, browsers the same messages as WebSocket text on the same port.
struct DeadlineStream {
    inner: TcpStream,
    deadline: Instant,
    poll: Option<Duration>,
}

impl Read for DeadlineStream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let remaining = self.deadline.checked_duration_since(Instant::now())
            .filter(|duration| !duration.is_zero())
            .ok_or_else(|| io::Error::from(io::ErrorKind::TimedOut))?;
        if self.poll.is_none() {
            self.inner.set_read_timeout(Some(remaining))?;
        }
        self.inner.read(buf)
    }
}

impl Write for DeadlineStream {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.inner.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

fn serve_connection(stream: TcpStream, state: Arc<Mutex<State>>) {
    let deadline = Instant::now() + AUTH_TIMEOUT;
    let _ = stream.set_nodelay(true);
    let _ = stream.set_read_timeout(Some(AUTH_TIMEOUT));
    let _ = stream.set_write_timeout(Some(WRITE_TIMEOUT));
    let mut first = [0u8; 1];
    // an HTTP upgrade request starts with `GET`, an XML frame with `<`
    match stream.peek(&mut first) {
        Ok(1) if first[0] == b'G' => serve_websocket(DeadlineStream { inner: stream, deadline, poll: None }, state),
        Ok(1) => serve_tcp(DeadlineStream { inner: stream, deadline, poll: None }, state),
        _ => {}
    }
}

fn serve_tcp(stream: DeadlineStream, state: Arc<Mutex<State>>) {
    let Ok(write_half) = stream.inner.try_clone() else { return };
    let Ok(control) = stream.inner.try_clone().map(Arc::new) else { return };
    let (tx, rx) = mpsc::sync_channel::<Vec<u8>>(OUTGOING_CAPACITY);
    let Ok(writer) = thread::Builder::new().spawn(move || {
        let mut w = write_half;
        for frame in rx {
            if w.write_all(&frame).is_err() {
                break;
            }
        }
        let _ = w.shutdown(Shutdown::Both);
    }) else { return };
    let auth_deadline = stream.deadline;
    let mut first = true;
    let mut reader = BufReader::new(stream);
    session(&state, tx, control.clone(), |timeout| {
        reader.get_mut().deadline = if first { auth_deadline } else { Instant::now() + timeout };
        first = false;
        read_frame(&mut reader, MAX_FRAME).ok().flatten()
    });
    let _ = control.shutdown(Shutdown::Read);
    let _ = writer.join();
}

fn serve_websocket(stream: DeadlineStream, state: Arc<Mutex<State>>) {
    let auth_deadline = stream.deadline;
    let Ok(control) = stream.inner.try_clone().map(Arc::new) else { return };
    let config = WebSocketConfig::default().max_message_size(Some(MAX_FRAME)).max_frame_size(Some(MAX_FRAME))
        .write_buffer_size(0).max_write_buffer_size(OUTGOING_CAPACITY * MAX_FRAME);
    let Ok(mut ws) = tungstenite::accept_with_config(stream, Some(config)) else { return };
    if ws.get_mut().inner.set_nonblocking(true).is_err() { return; }
    ws.get_mut().poll = Some(WS_POLL);
    let (tx, rx) = mpsc::sync_channel(OUTGOING_CAPACITY);
    let mut first = true;
    // one thread owns the socket, so outgoing frames are sent between short reads
    let to_text = |frame: Vec<u8>| Message::text(String::from_utf8_lossy(&frame).trim_end_matches(['\n', '\0']).to_string());
    session(&state, tx, control, |timeout| {
        let deadline = if first { auth_deadline } else { Instant::now() + timeout };
        first = false;
        ws.get_mut().deadline = deadline;
        loop {
            for frame in rx.try_iter().take(OUTGOING_CAPACITY) {
                match ws.send(to_text(frame)) {
                    Ok(()) => {}
                    Err(tungstenite::Error::Io(error)) if error.kind() == io::ErrorKind::WouldBlock => {}
                    Err(_) => return None,
                }
            }
            match ws.flush() {
                Ok(()) => {}
                Err(tungstenite::Error::Io(error)) if error.kind() == io::ErrorKind::WouldBlock => {}
                Err(_) => return None,
            }
            if Instant::now() >= deadline {
                return None;
            }
            match ws.read() {
                Ok(Message::Text(t)) => return Some(t.as_str().to_string()),
                Ok(Message::Close(_)) => return None,
                Ok(_) => {}
                Err(tungstenite::Error::Io(e)) if matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut) => thread::sleep(WS_POLL),
                Err(_) => return None,
            }
        }
    });
    for frame in rx.try_iter() {
        let _ = ws.send(to_text(frame));
    }
    let _ = ws.close(None);
    let _ = ws.flush();
}

/// Authenticates, then relays messages until the client is gone. `recv` returns the next message or `None` when the
/// connection ended or nothing arrived within the given time.
fn session(state: &Mutex<State>, tx: SyncSender<Vec<u8>>, socket: Arc<TcpStream>, mut recv: impl FnMut(Duration) -> Option<String>) {
    let auth = recv(AUTH_TIMEOUT).and_then(|f| Msg::from_xml(&f));
    let name = match auth {
        Some(Msg::Auth { name, version, .. }) if version == VERSION => name,
        Some(Msg::Auth { .. }) => {
            let _ = tx.try_send(Msg::ErrorMsg("Falsche Spielversion.".into()).to_frame());
            return;
        }
        _ => return,
    };
    let _ = tx.try_send(Msg::ServerVersion(VERSION.into()).to_frame());
    if let Err(e) = lock(state).join(&name, tx.clone()) {
        let _ = tx.try_send(Msg::ErrorMsg(e).to_frame());
        return;
    }
    lock(state).clients.get_mut(&name).unwrap().socket = Some(socket);
    drop(tx);

    let mut window_start = Instant::now();
    let mut count = 0u32;
    while let Some(frame) = recv(IDLE_TIMEOUT) {
        if window_start.elapsed() >= Duration::from_secs(1) {
            window_start = Instant::now();
            count = 0;
        }
        count += 1;
        if count > MAX_MSGS_PER_SECOND {
            break;
        }
        if let Some(m) = Msg::from_xml(&frame) {
            lock(state).handle(&name, m, Instant::now());
        }
    }
    lock(state).leave(&name);
}

pub struct ServerHandle {
    pub addr: SocketAddr,
    stop: Arc<AtomicBool>,
}

impl ServerHandle {
    /// Stops accepting new connections (running ones end when their clients disconnect).
    pub fn stop(&self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = TcpStream::connect(self.addr);
    }
}

struct ConnectionPermit {
    counts: Arc<Mutex<HashMap<IpAddr, usize>>>,
    ip: IpAddr,
}

impl ConnectionPermit {
    fn acquire(counts: &Arc<Mutex<HashMap<IpAddr, usize>>>, ip: IpAddr) -> Option<Self> {
        let mut active = counts.lock().unwrap_or_else(|error| error.into_inner());
        if active.values().sum::<usize>() >= MAX_CONNECTIONS || active.get(&ip).copied().unwrap_or(0) >= MAX_CONNECTIONS_PER_IP {
            return None;
        }
        *active.entry(ip).or_default() += 1;
        Some(Self { counts: counts.clone(), ip })
    }
}

impl Drop for ConnectionPermit {
    fn drop(&mut self) {
        let mut active = self.counts.lock().unwrap_or_else(|error| error.into_inner());
        if let Some(count) = active.get_mut(&self.ip) {
            *count -= 1;
            if *count == 0 {
                active.remove(&self.ip);
            }
        }
    }
}

fn accept_loop(listener: TcpListener, stop: Arc<AtomicBool>) {
    let state = Arc::new(Mutex::new(State::default()));
    let counts = Arc::new(Mutex::new(HashMap::new()));
    for conn in listener.incoming() {
        if stop.load(Ordering::SeqCst) {
            break;
        }
        if let Ok(stream) = conn {
            let Ok(peer) = stream.peer_addr() else { continue };
            let Some(permit) = ConnectionPermit::acquire(&counts, peer.ip()) else { continue };
            let st = state.clone();
            let _ = thread::Builder::new().spawn(move || {
                let _permit = permit;
                serve_connection(stream, st);
            });
        }
    }
}

/// Starts the server on a background thread; use port 0 to let the OS pick one.
pub fn spawn(addr: impl ToSocketAddrs) -> io::Result<ServerHandle> {
    let listener = TcpListener::bind(addr)?;
    let addr = listener.local_addr()?;
    let stop = Arc::new(AtomicBool::new(false));
    let s = stop.clone();
    thread::spawn(move || accept_loop(listener, s));
    Ok(ServerHandle { addr, stop })
}

/// Runs the server on the calling thread.
pub fn run(addr: impl ToSocketAddrs) -> io::Result<()> {
    let listener = TcpListener::bind(addr)?;
    accept_loop(listener, Arc::new(AtomicBool::new(false)));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Peer {
        name: String,
        rx: Receiver<Vec<u8>>,
    }

    impl Peer {
        fn got(&self) -> Vec<Msg> {
            let mut out = Vec::new();
            while let Ok(f) = self.rx.try_recv() {
                let s = String::from_utf8(f).unwrap();
                assert!(s.ends_with("\n\0"), "frame is terminated: {s:?}");
                out.push(Msg::from_xml(s.trim_end_matches('\0')).expect("server sends valid messages"));
            }
            out
        }
    }

    fn join(st: &mut State, name: &str) -> Peer {
        let (tx, rx) = mpsc::sync_channel(OUTGOING_CAPACITY);
        st.join(name, tx).unwrap();
        Peer { name: name.into(), rx }
    }

    fn say(st: &mut State, p: &Peer, m: Msg) {
        st.handle(&p.name, m, Instant::now());
    }

    fn matched(st: &mut State) -> (Peer, Peer) {
        let a = join(st, "Anna");
        let b = join(st, "Bob");
        say(st, &b, Msg::Challenge("Anna".into()));
        say(st, &a, Msg::StartGame("Bob".into()));
        say(st, &a, Msg::Seed(4242));
        a.got();
        b.got();
        (a, b)
    }

    #[test]
    fn forged_gameplay_is_not_relayed_and_bombs_are_bounded() {
        let mut st = State::default();
        let (anna, bob) = matched(&mut st);
        let cmd = Cmd { x: 120, y: 40, c: "0000".into() };
        for forged in [
            Msg::Seed(42),
            Msg::Move(Cmd { x: 600, y: 440, c: "0000".into() }),
            Msg::Move(Cmd { c: "invalid".into(), ..cmd.clone() }),
            Msg::Bomb { cmd: cmd.clone(), cx: 12, cy: 10 },
            Msg::Pickup { cmd: cmd.clone(), cx: 0, cy: 0, speed: -999.0, blast: 64 },
            Msg::Pickup { cmd: cmd.clone(), cx: 0, cy: 0, speed: 0.15, blast: 2 },
            Msg::Kick { cmd: cmd.clone(), cx: 0, cy: 0, id: 5000, dx: 0.1, dy: 0.0, t: 3500 },
        ] {
            say(&mut st, &anna, forged);
            assert!(bob.got().is_empty());
        }
        say(&mut st, &bob, Msg::Seed(777));
        assert!(anna.got().is_empty());
        let bomb = Msg::Bomb { cmd: cmd.clone(), cx: 0, cy: 0 };
        say(&mut st, &anna, bomb.clone());
        assert_eq!(bob.got(), vec![bomb.clone()]);
        for _ in 0..100 { say(&mut st, &anna, bomb.clone()); }
        say(&mut st, &anna, Msg::Bomb { cmd, cx: 1, cy: 0 });
        assert!(bob.got().is_empty());
        assert_eq!(st.matches["Anna"].world.as_ref().unwrap().bombs.len(), 1);
        say(&mut st, &anna, Msg::Surrender(None));
        bob.got();
        say(&mut st, &anna, bomb);
        assert!(bob.got().is_empty());
    }

    #[test]
    fn ledger_requires_host_seed_and_enforces_elapsed_movement_and_walls() {
        let now = Instant::now();
        let mut game = MatchState::new("Anna", "Bob", now);
        let movement = |x, y| Msg::Move(Cmd { x, y, c: "0000".into() });
        assert!(!game.accept("Anna", &movement(120, 40), now));
        assert!(!game.accept("Bob", &Msg::Seed(1), now));
        assert!(game.accept("Anna", &Msg::Seed(1), now));
        assert!(!game.accept("Anna", &Msg::Seed(2), now));
        let mut world = World::with_cells(crate::grid::base_level());
        world.local = Some(2);
        game.world = Some(world);
        assert!(game.accept("Anna", &movement(160, 40), now));
        assert!(!game.accept("Anna", &movement(280, 40), now));
        let later = now + Duration::from_secs(1);
        assert!(!game.accept("Anna", &movement(160, 120), later));
        assert!(game.accept("Anna", &movement(200, 40), later));
        assert_eq!(game.world.as_ref().unwrap().now, 1000);
        for _ in 0..100 { game.accept("Anna", &movement(200, 40), later); }
        assert_eq!(game.world.as_ref().unwrap().now, 1000);
    }

    #[test]
    fn ledger_checks_real_pickups_fuses_and_bomb_control_ownership() {
        let now = Instant::now();
        let mut game = MatchState::new("Anna", "Bob", now);
        assert!(game.accept("Anna", &Msg::Seed(1), now));
        let mut world = World::with_cells(crate::grid::base_level());
        world.local = Some(2);
        world.cells[0][0] = crate::grid::Cell { kind: crate::grid::Kind::ItemKick, open: true };
        world.players[1].x = 160.0;
        world.players[1].y = 40.0;
        world.players[1].cx = 1;
        world.players[1].cy = 0;
        game.world = Some(world);
        let cmd = Cmd { x: 120, y: 40, c: "0000".into() };
        let pickup = Msg::Pickup { cmd: cmd.clone(), cx: 0, cy: 0, speed: 0.15, blast: 2 };
        assert!(game.accept("Anna", &pickup, now));
        assert!(!game.accept("Anna", &pickup, now));
        assert!(game.accept("Anna", &Msg::Bomb { cmd: cmd.clone(), cx: 0, cy: 0 }, now));
        let kick = |id, t| Msg::Kick { cmd: cmd.clone(), cx: 0, cy: 0, id, t, dx: 0.1, dy: 0.0 };
        assert!(!game.accept("Anna", &kick(5000, 0), now));
        assert!(game.accept("Anna", &kick(5000, 3500), now));
        assert!(!game.accept("Anna", &kick(5001, 3500), now));
        let bob = Cmd { x: 160, y: 40, c: "0000".into() };
        assert!(game.accept("Bob", &Msg::Stop { cmd: bob.clone(), cx: 1, cy: 0, id: 5000, t: 3500 }, now));
        let detonate = Msg::Detonate { cmd: bob, cx: 1, cy: 0, id: 5000 };
        assert!(!game.accept("Anna", &detonate, now));
        assert!(!game.accept("Bob", &detonate, now));
        assert!(game.accept("Bob", &detonate, now + Duration::from_millis(3500)));
    }

    #[test]
    fn retained_records_have_a_fixed_memory_bound() {
        let mut st = State::default();
        let _active = join(&mut st, "Anna");
        st.stats.insert("Anna".into(), (1, 0, 0));
        for index in 1..MAX_RECORDS {
            st.stats.insert(format!("Old{index}"), (0, 1, 0));
        }
        let _new = join(&mut st, "Bob");
        assert_eq!(st.stats.len(), 1);
        assert_eq!(st.skill("Anna"), "1/0/0");
    }

    #[test]
    fn name_rules() {
        for ok in ["Anna", "Gast12345", "a b", "Zoë-1_x.y"] {
            assert!(valid_name(ok), "{ok}");
        }
        for bad in ["", " a", "a ", "a\"b", "<x>", "a&b", &"x".repeat(21), "a\nb"] {
            assert!(!valid_name(bad), "{bad:?}");
        }
    }

    #[test]
    fn chat_is_cleaned_like_the_original_client_does() {
        assert_eq!(clean_chat("a\"b&c<d>e[f]g"), "a'b+c(d)e(f)g");
        assert_eq!(clean_chat("  hi\u{7}\n "), "hi");
        assert_eq!(clean_chat(&"x".repeat(500)).chars().count(), MAX_CHAT_CHARS);
    }

    #[test]
    fn join_sends_the_list_and_announces_the_newcomer() {
        let mut st = State::default();
        let a = join(&mut st, "Anna");
        let list = a.got();
        let Msg::UserList(l) = &list[0] else { panic!("{list:?}") };
        assert_eq!(l.len(), 1);
        assert_eq!((l[0].name.as_str(), l[0].skill.as_str(), l[0].state.as_str()), ("Anna", "0/0/0", "0"));
        let b = join(&mut st, "Bob");
        assert_eq!(a.got(), vec![Msg::NewPlayer(PlayerInfo { name: "Bob".into(), skill: "0/0/0".into(), state: "0".into() })]);
        let Msg::UserList(l) = &b.got()[0] else { panic!() };
        assert_eq!(l.len(), 2);
    }

    #[test]
    fn duplicate_invalid_and_overflow_joins_are_rejected() {
        let mut st = State::default();
        join(&mut st, "Anna");
        assert!(st.join("Anna", mpsc::sync_channel(OUTGOING_CAPACITY).0).is_err());
        assert!(st.join("bad<name", mpsc::sync_channel(OUTGOING_CAPACITY).0).is_err());
        for i in 1..MAX_PLAYERS {
            st.join(&format!("p{i}"), mpsc::sync_channel(OUTGOING_CAPACITY).0).unwrap();
        }
        assert!(st.join("one too many", mpsc::sync_channel(OUTGOING_CAPACITY).0).is_err());
        assert_eq!(st.player_count(), MAX_PLAYERS);
    }

    #[test]
    fn challenge_and_withdraw_reach_only_the_target() {
        let mut st = State::default();
        let (a, b, c) = (join(&mut st, "Anna"), join(&mut st, "Bob"), join(&mut st, "Cy"));
        for p in [&a, &b, &c] {
            p.got();
        }
        say(&mut st, &a, Msg::Challenge("Bob".into()));
        assert_eq!(b.got(), vec![Msg::Request("Anna".into())]);
        assert!(a.got().is_empty() && c.got().is_empty());
        say(&mut st, &a, Msg::RemChallenge("Bob".into()));
        assert_eq!(b.got(), vec![Msg::RemRequest("Anna".into())]);
        say(&mut st, &a, Msg::RemChallenge("Bob".into()));
        assert!(b.got().is_empty(), "nothing to withdraw");
        say(&mut st, &a, Msg::Challenge("Nobody".into()));
        say(&mut st, &a, Msg::Challenge("Anna".into()));
        assert!(a.got().is_empty());
    }

    #[test]
    fn challenge_all_includes_players_who_join_later() {
        let mut st = State::default();
        let (a, b) = (join(&mut st, "Anna"), join(&mut st, "Bob"));
        a.got();
        b.got();
        say(&mut st, &a, Msg::ChallengeAll);
        assert_eq!(b.got(), vec![Msg::Request("Anna".into())]);
        let c = join(&mut st, "Cy");
        assert!(c.got().contains(&Msg::Request("Anna".into())));
        say(&mut st, &a, Msg::RemChallengeAll);
        assert_eq!(c.got(), vec![Msg::RemRequest("Anna".into())]);
    }

    #[test]
    fn accepting_a_challenge_starts_the_game_with_the_acceptor_as_player_one() {
        let mut st = State::default();
        let (a, b, c) = (join(&mut st, "Anna"), join(&mut st, "Bob"), join(&mut st, "Cy"));
        say(&mut st, &b, Msg::Challenge("Anna".into()));
        say(&mut st, &b, Msg::Challenge("Cy".into()));
        for p in [&a, &b, &c] {
            p.got();
        }
        say(&mut st, &a, Msg::StartGame("Bob".into()));
        assert_eq!(b.got().iter().filter(|m| **m == Msg::StartGame("Anna".into())).count(), 1);
        assert!(a.got().iter().all(|m| !matches!(m, Msg::StartGame(_))));
        let seen = c.got();
        assert!(seen.contains(&Msg::RemRequest("Bob".into())), "Bob's other challenge is withdrawn: {seen:?}");
        let upd: Vec<_> = seen.iter().filter_map(|m| if let Msg::PlayerUpdate(p) = m { Some((p.name.clone(), p.state.clone())) } else { None }).collect();
        assert!(upd.contains(&("Anna".into(), "3".into())) && upd.contains(&("Bob".into(), "3".into())));
    }

    #[test]
    fn starting_without_a_challenge_or_while_busy_is_refused() {
        let mut st = State::default();
        let (a, b, c) = (join(&mut st, "Anna"), join(&mut st, "Bob"), join(&mut st, "Cy"));
        say(&mut st, &a, Msg::StartGame("Bob".into()));
        assert!(b.got().iter().all(|m| !matches!(m, Msg::StartGame(_))));
        say(&mut st, &b, Msg::Challenge("Anna".into()));
        say(&mut st, &c, Msg::Challenge("Anna".into()));
        say(&mut st, &a, Msg::StartGame("Bob".into()));
        c.got();
        say(&mut st, &a, Msg::StartGame("Cy".into()));
        assert!(c.got().iter().all(|m| !matches!(m, Msg::StartGame(_))), "Anna is already in a game");
    }

    #[test]
    fn in_game_messages_go_to_the_opponent_only_and_only_in_a_match() {
        let mut st = State::default();
        let c = join(&mut st, "Cy");
        let (a, b) = matched(&mut st);
        c.got();
        let mv = Msg::Move(Cmd { x: 120, y: 40, c: "0010".into() });
        say(&mut st, &a, mv.clone());
        assert_eq!(b.got(), vec![mv.clone()]);
        assert!(a.got().is_empty() && c.got().is_empty());
        say(&mut st, &c, mv);
        assert!(a.got().is_empty() && b.got().is_empty(), "Cy is not in a match");
        say(&mut st, &b, Msg::Ping);
        assert_eq!(b.got(), vec![Msg::Pong]);
        assert!(a.got().is_empty());
    }

    #[test]
    fn win_is_recorded_once_and_updates_the_skill() {
        let mut st = State::default();
        let (a, b) = matched(&mut st);
        say(&mut st, &b, Msg::Die { x: 600, y: 440 });
        assert_eq!(a.got(), vec![Msg::Die { x: 600, y: 440 }]);
        say(&mut st, &a, Msg::WinGame);
        say(&mut st, &a, Msg::WinGame);
        let got = b.got();
        assert_eq!(got.iter().filter(|m| matches!(m, Msg::EndGame { winner } if winner == "Anna")).count(), 1);
        let skills: HashMap<_, _> = got.iter().filter_map(|m| if let Msg::PlayerUpdate(p) = m { Some((p.name.clone(), p.skill.clone())) } else { None }).collect();
        assert_eq!(skills["Anna"], "1/0/0");
        assert_eq!(skills["Bob"], "0/1/0");
    }

    #[test]
    fn a_player_cannot_claim_a_win_without_the_other_dying() {
        let mut st = State::default();
        let (a, b) = matched(&mut st);
        say(&mut st, &a, Msg::WinGame);
        assert!(a.got().is_empty() && b.got().is_empty());
        // the dead one cannot claim it either
        say(&mut st, &a, Msg::Die { x: 120, y: 40 });
        b.got();
        say(&mut st, &a, Msg::WinGame);
        assert!(b.got().is_empty());
    }

    #[test]
    fn two_deaths_close_together_are_a_draw_but_far_apart_are_not() {
        let mut st = State::default();
        let (a, b) = matched(&mut st);
        let t0 = Instant::now();
        st.handle("Anna", Msg::Die { x: 120, y: 40 }, t0);
        st.handle("Bob", Msg::Die { x: 600, y: 440 }, t0 + Duration::from_millis(300));
        assert!(a.got().contains(&Msg::Draw) && b.got().contains(&Msg::Draw));
        st.handle("Anna", Msg::DrawGame, t0);
        let ends = b.got().into_iter().filter(|m| matches!(m, Msg::EndGame { winner } if winner.is_empty())).count();
        assert_eq!(ends, 1);

        let mut st = State::default();
        let (a, b) = matched(&mut st);
        st.handle("Anna", Msg::Die { x: 120, y: 40 }, t0);
        st.handle("Bob", Msg::Die { x: 600, y: 440 }, t0 + DRAW_WINDOW + Duration::from_millis(1));
        assert!(!a.got().contains(&Msg::Draw) && !b.got().contains(&Msg::Draw));
    }

    #[test]
    fn surrender_tells_both_who_won() {
        let mut st = State::default();
        let (a, b) = matched(&mut st);
        say(&mut st, &b, Msg::Surrender(None));
        assert!(a.got().contains(&Msg::Surrender(Some("Anna".into()))));
        assert!(b.got().contains(&Msg::Surrender(Some("Anna".into()))));
        say(&mut st, &b, Msg::Surrender(None));
        assert!(a.got().is_empty(), "only once");
    }

    #[test]
    fn rematch_needs_no_new_challenge_and_keeps_the_pair() {
        let mut st = State::default();
        let (a, b) = matched(&mut st);
        say(&mut st, &b, Msg::Surrender(None));
        a.got();
        b.got();
        say(&mut st, &a, Msg::PlayAgain);
        assert_eq!(b.got(), vec![Msg::PlayAgain]);
        say(&mut st, &b, Msg::StartGame("Anna".into()));
        assert_eq!(a.got(), vec![Msg::StartGame("Bob".into())]);
    }

    #[test]
    fn leaving_to_the_room_frees_both_and_chat_is_scoped() {
        let mut st = State::default();
        let c = join(&mut st, "Cy");
        let (a, b) = matched(&mut st);
        c.got();
        say(&mut st, &a, Msg::MsgPlayer { name: "x".into(), msg: "gl".into() });
        assert_eq!(b.got(), vec![Msg::MsgPlayer { name: "Anna".into(), msg: "gl".into() }]);
        assert!(c.got().is_empty());
        a.got();
        say(&mut st, &c, Msg::MsgAll { name: "spoof".into(), msg: "hi <b>".into() });
        assert_eq!(c.got(), vec![Msg::MsgAll { name: "Cy".into(), msg: "hi (b)".into() }]);
        assert!(a.got().is_empty() && b.got().is_empty(), "players in a match do not get room chat");

        say(&mut st, &a, Msg::ToRoom);
        say(&mut st, &b, Msg::ToRoom);
        say(&mut st, &a, Msg::MsgAll { name: "Anna".into(), msg: "back".into() });
        assert!(c.got().iter().any(|m| matches!(m, Msg::MsgAll { .. })));
        // a relay message after the match goes nowhere
        say(&mut st, &a, Msg::Seed(1));
        assert!(b.got().iter().all(|m| !matches!(m, Msg::Seed(_))));
    }

    #[test]
    fn disconnect_is_announced_and_cleans_challenges() {
        let mut st = State::default();
        let (a, b) = matched(&mut st);
        st.leave("Anna");
        assert!(b.got().contains(&Msg::PlayerLeft("Anna".into())));
        assert_eq!(st.player_count(), 1);
        // Bob is still in_game but has no opponent; messages go nowhere
        say(&mut st, &b, Msg::Move(Cmd { x: 1, y: 1, c: "0000".into() }));
        drop(a);
        st.leave("Anna");
    }

    #[test]
    fn active_and_unrequested_rematches_are_refused() {
        let mut st = State::default();
        let (a, b) = matched(&mut st);
        say(&mut st, &a, Msg::PlayAgain);
        say(&mut st, &a, Msg::StartGame("Bob".into()));
        assert!(b.got().is_empty());
        say(&mut st, &a, Msg::Surrender(None));
        a.got();
        b.got();
        say(&mut st, &a, Msg::StartGame("Bob".into()));
        assert!(b.got().is_empty());
        assert!(st.clients["Anna"].result_done);
    }

    #[test]
    fn leaving_an_active_match_records_a_forfeit_once() {
        for disconnect in [false, true] {
            let mut st = State::default();
            let (a, b) = matched(&mut st);
            if disconnect {
                st.leave("Anna");
            } else {
                say(&mut st, &a, Msg::ToRoom);
            }
            assert!(b.got().contains(&Msg::EndGame { winner: "Bob".into() }));
            assert_eq!(st.skill("Anna"), "0/1/0");
            assert_eq!(st.skill("Bob"), "1/0/0");
            st.leave("Anna");
            assert_eq!(st.skill("Bob"), "1/0/0");
        }
    }

    #[test]
    fn connection_permits_limit_each_ip_and_release_on_drop() {
        let counts = Arc::new(Mutex::new(HashMap::new()));
        let ip = "127.0.0.1".parse().unwrap();
        let mut permits: Vec<_> = (0..MAX_CONNECTIONS_PER_IP)
            .map(|_| ConnectionPermit::acquire(&counts, ip).unwrap()).collect();
        assert!(ConnectionPermit::acquire(&counts, ip).is_none());
        permits.pop();
        assert!(ConnectionPermit::acquire(&counts, ip).is_some());
        drop(permits);
        assert!(counts.lock().unwrap().is_empty());
        counts.lock().unwrap().insert(ip, MAX_CONNECTIONS);
        assert!(ConnectionPermit::acquire(&counts, "127.0.0.2".parse().unwrap()).is_none());
    }

    #[test]
    fn full_outgoing_queue_disconnects_the_slow_recipient() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let mut peer = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        peer.set_read_timeout(Some(Duration::from_secs(1))).unwrap();
        let (socket, _) = listener.accept().unwrap();
        let (tx, rx) = mpsc::sync_channel(1);
        let mut st = State::default();
        st.join("Slow", tx).unwrap();
        st.clients.get_mut("Slow").unwrap().socket = Some(Arc::new(socket));
        st.send("Slow", &Msg::Pong);
        assert_eq!(rx.try_iter().count(), 1);
        assert_eq!(peer.read(&mut [0; 1]).unwrap(), 0);
    }

    #[test]
    fn expired_read_deadline_is_not_extended_by_more_bytes() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let mut peer = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (inner, _) = listener.accept().unwrap();
        peer.write_all(b"partial frame").unwrap();
        let mut stream = DeadlineStream { inner, deadline: Instant::now(), poll: None };
        assert_eq!(stream.read(&mut [0; 1]).unwrap_err().kind(), io::ErrorKind::TimedOut);
    }
}
