//! Relay server for online matches.
//!
//! The server of the original game is not part of the SWF; this one implements what the client expects:
//! a player list with challenges, forwarding of the in-game messages to the opponent, `pong` for `ping`,
//! chat, surrender, and win/draw bookkeeping (`won/lost/drawn` as the `skill` of a player).

use crate::proto::*;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::{self, BufReader, Write};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

pub const DEFAULT_PORT: u16 = 6900;
pub const MAX_PLAYERS: usize = 100;
/// Clients send `<beat/>` every 15 s when otherwise silent.
pub const IDLE_TIMEOUT: Duration = Duration::from_secs(45);
const AUTH_TIMEOUT: Duration = Duration::from_secs(10);
/// Two deaths this close together are a draw (the second player died before the first `die` reached it).
pub const DRAW_WINDOW: Duration = Duration::from_millis(1500);
pub const MAX_NAME_CHARS: usize = 20;
pub const MAX_CHAT_CHARS: usize = 200;
const MAX_MSGS_PER_SECOND: u32 = 300;

/// Names end up in attributes and are compared by clients; keep them plain.
pub fn valid_name(name: &str) -> bool {
    let n = name.chars().count();
    (1..=MAX_NAME_CHARS).contains(&n)
        && name == name.trim()
        && name.chars().all(|c| c.is_alphanumeric() || matches!(c, ' ' | '_' | '-' | '.'))
}

/// `Util.cleanMsg` plus a length limit.
pub fn clean_chat(msg: &str) -> String {
    let s: String = msg
        .chars()
        .filter(|c| !c.is_control())
        .map(|c| match c {
            '"' => '\'',
            '&' => '+',
            '<' | '[' => '(',
            '>' | ']' => ')',
            c => c,
        })
        .take(MAX_CHAT_CHARS)
        .collect();
    s.trim().to_string()
}

struct Client {
    tx: Sender<Vec<u8>>,
    challenges: HashSet<String>,
    challenge_all: bool,
    opponent: Option<String>,
    in_game: bool,
    died_at: Option<Instant>,
    result_done: bool,
}

#[derive(Default)]
pub struct State {
    clients: BTreeMap<String, Client>,
    /// won, lost, drawn per player name; kept while the server runs
    stats: HashMap<String, (u32, u32, u32)>,
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
            let _ = c.tx.send(m.to_frame());
        }
    }

    fn broadcast(&self, m: &Msg, except: Option<&str>) {
        for (n, c) in &self.clients {
            if Some(n.as_str()) != except {
                let _ = c.tx.send(m.to_frame());
            }
        }
    }

    pub fn player_count(&self) -> usize {
        self.clients.len()
    }

    /// Registers an authenticated player. `Err` is the text for the `errorMsg`.
    pub fn join(&mut self, name: &str, tx: Sender<Vec<u8>>) -> Result<(), String> {
        if !valid_name(name) {
            return Err("Ungültiger Name.".into());
        }
        if self.clients.contains_key(name) {
            return Err("Der Name ist bereits vergeben.".into());
        }
        if self.clients.len() >= MAX_PLAYERS {
            return Err("Der Server ist voll.".into());
        }
        self.clients.insert(
            name.to_string(),
            Client {
                tx,
                challenges: HashSet::new(),
                challenge_all: false,
                opponent: None,
                in_game: false,
                died_at: None,
                result_done: false,
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
                        let _ = c.tx.send(out.to_frame());
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
                    self.send(&o, &Msg::PlayAgain);
                }
            }
            Msg::Ping => self.send(me, &Msg::Pong),
            Msg::Surrender(_) => self.surrender(me),
            Msg::WinGame => self.win_reported(me),
            Msg::DrawGame => self.draw_reported(me),
            m if m.is_relay() => {
                let Some(o) = self.opponent_of(me) else { return };
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
                            let _ = cl.tx.send(Msg::RemRequest(n.to_string()).to_frame());
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
        }
        self.send(t, &Msg::StartGame(me.to_string()));
    }

    fn leave_match(&mut self, me: &str) {
        let c = self.clients.get_mut(me).unwrap();
        c.in_game = false;
        c.died_at = None;
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
                Some(w) if w == n => s.0 += 1,
                Some(_) => s.1 += 1,
                None => s.2 += 1,
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

fn serve_connection(stream: TcpStream, state: Arc<Mutex<State>>) {
    let _ = stream.set_nodelay(true);
    let _ = stream.set_read_timeout(Some(AUTH_TIMEOUT));
    let Ok(write_half) = stream.try_clone() else { return };
    let (tx, rx): (Sender<Vec<u8>>, Receiver<Vec<u8>>) = mpsc::channel();
    thread::spawn(move || {
        let mut w = write_half;
        for frame in rx {
            if w.write_all(&frame).is_err() {
                break;
            }
        }
        let _ = w.shutdown(Shutdown::Both);
    });
    let mut reader = BufReader::new(stream);

    let auth = read_frame(&mut reader, MAX_FRAME).ok().flatten().and_then(|f| Msg::from_xml(&f));
    let name = match auth {
        Some(Msg::Auth { name, version, .. }) if version == VERSION => name,
        Some(Msg::Auth { .. }) => {
            let _ = tx.send(Msg::ErrorMsg("Falsche Spielversion.".into()).to_frame());
            return;
        }
        _ => return,
    };
    if let Err(e) = lock(&state).join(&name, tx.clone()) {
        let _ = tx.send(Msg::ErrorMsg(e).to_frame());
        return;
    }
    drop(tx);
    let _ = reader.get_ref().set_read_timeout(Some(IDLE_TIMEOUT));

    let mut window_start = Instant::now();
    let mut count = 0u32;
    while let Ok(Some(frame)) = read_frame(&mut reader, MAX_FRAME) {
        if window_start.elapsed() >= Duration::from_secs(1) {
            window_start = Instant::now();
            count = 0;
        }
        count += 1;
        if count > MAX_MSGS_PER_SECOND {
            break;
        }
        if let Some(m) = Msg::from_xml(&frame) {
            lock(&state).handle(&name, m, Instant::now());
        }
    }
    lock(&state).leave(&name);
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

fn accept_loop(listener: TcpListener, stop: Arc<AtomicBool>) {
    let state = Arc::new(Mutex::new(State::default()));
    for conn in listener.incoming() {
        if stop.load(Ordering::SeqCst) {
            break;
        }
        if let Ok(stream) = conn {
            let st = state.clone();
            thread::spawn(move || serve_connection(stream, st));
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
        let (tx, rx) = mpsc::channel();
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
        a.got();
        b.got();
        (a, b)
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
        assert!(st.join("Anna", mpsc::channel().0).is_err());
        assert!(st.join("bad<name", mpsc::channel().0).is_err());
        for i in 1..MAX_PLAYERS {
            st.join(&format!("p{i}"), mpsc::channel().0).unwrap();
        }
        assert!(st.join("one too many", mpsc::channel().0).is_err());
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
        say(&mut st, &a, Msg::Die { x: 1, y: 1 });
        b.got();
        say(&mut st, &a, Msg::WinGame);
        assert!(b.got().is_empty());
    }

    #[test]
    fn two_deaths_close_together_are_a_draw_but_far_apart_are_not() {
        let mut st = State::default();
        let (a, b) = matched(&mut st);
        let t0 = Instant::now();
        st.handle("Anna", Msg::Die { x: 1, y: 1 }, t0);
        st.handle("Bob", Msg::Die { x: 2, y: 2 }, t0 + Duration::from_millis(300));
        assert!(a.got().contains(&Msg::Draw) && b.got().contains(&Msg::Draw));
        st.handle("Anna", Msg::DrawGame, t0);
        let ends = b.got().into_iter().filter(|m| matches!(m, Msg::EndGame { winner } if winner.is_empty())).count();
        assert_eq!(ends, 1);

        let mut st = State::default();
        let (a, b) = matched(&mut st);
        st.handle("Anna", Msg::Die { x: 1, y: 1 }, t0);
        st.handle("Bob", Msg::Die { x: 2, y: 2 }, t0 + DRAW_WINDOW + Duration::from_millis(1));
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
}
