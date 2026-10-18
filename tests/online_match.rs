//! Whole stack without a window: two clients (lobby + match logic) play through a real server over TCP.

use bomberpengu::lobby::{Lobby, LobbyEvent};
use bomberpengu::net::{Connection, NetEvent};
use bomberpengu::netgame::{NetMatch, NetOutcome, NetPhase};
use bomberpengu::proto::Msg;
use bomberpengu::server;
use bomberpengu::world::DT_MS;
use std::time::Duration;

struct Client {
    conn: Connection,
    lobby: Lobby,
    nm: Option<NetMatch>,
    ticks: i64,
}

impl Client {
    fn new(addr: &str, name: &str) -> Self {
        Client { conn: Connection::connect(addr, name), lobby: Lobby::new(name), nm: None, ticks: 0 }
    }

    fn wall(&self) -> i64 {
        self.ticks * DT_MS
    }

    /// What the UI does each frame: deliver network events, advance the match by one tick, send what it produced.
    fn step(&mut self, seed: i64) {
        for e in self.conn.poll() {
            match e {
                NetEvent::Closed(why) => panic!("connection closed: {why}"),
                NetEvent::Msg(m) => {
                    let wall = self.wall();
                    if let Some(nm) = &mut self.nm {
                        nm.handle(m.clone(), wall);
                    }
                    let r = self.lobby.handle(&m);
                    for s in r.send {
                        self.conn.send(s);
                    }
                    if let Some(LobbyEvent::StartMatch { opponent, number }) = r.event {
                        if self.nm.is_none() {
                            let me = usize::from(number) - 1;
                            let mine = self.lobby.my_name.clone();
                            let names = if me == 0 { [mine, opponent] } else { [opponent, mine] };
                            self.nm = Some(NetMatch::new(me, names, seed, wall));
                        }
                    }
                }
            }
        }
        self.ticks += 1;
        let wall = self.wall();
        if let Some(nm) = &mut self.nm {
            nm.tick(wall);
            for m in nm.take_outbox() {
                self.conn.send(m);
            }
        }
    }
}

fn run_until(a: &mut Client, b: &mut Client, what: &str, mut done: impl FnMut(&mut Client, &mut Client) -> bool) {
    for _ in 0..4000 {
        a.step(321);
        b.step(654);
        if done(a, b) {
            return;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    panic!("timeout waiting for: {what}");
}

#[test]
fn a_full_match_over_the_server_decides_a_winner_and_updates_the_record() {
    let srv = server::spawn("127.0.0.1:0").unwrap();
    let addr = srv.addr.to_string();
    let mut anna = Client::new(&addr, "Anna");
    let mut bob = Client::new(&addr, "Bob");
    run_until(&mut anna, &mut bob, "both in the room", |a, b| a.lobby.players.len() == 2 && b.lobby.players.len() == 2);

    // Bob challenges, Anna accepts and becomes player 1
    let r = bob.lobby.click("Anna");
    for m in r.send {
        bob.conn.send(m);
    }
    run_until(&mut anna, &mut bob, "request arrives", |a, _| a.lobby.players.iter().any(|p| p.name == "Bob" && p.code() == "1"));
    let r = anna.lobby.click("Bob");
    for m in r.send {
        anna.conn.send(m);
    }
    anna.nm = Some(NetMatch::new(0, ["Anna".into(), "Bob".into()], 321, 0));
    run_until(&mut anna, &mut bob, "both playing", |a, b| {
        a.nm.as_ref().is_some_and(|n| n.phase == NetPhase::Playing) && b.nm.as_ref().is_some_and(|n| n.phase == NetPhase::Playing)
    });
    assert_eq!(anna.nm.as_ref().unwrap().world.cells, bob.nm.as_ref().unwrap().world.cells, "same level on both sides");

    // Anna drops a bomb and stands on it
    anna.nm.as_mut().unwrap().local_player_mut().key_space_down();
    run_until(&mut anna, &mut bob, "match result", |a, b| a.nm.as_ref().unwrap().outcome.is_some() && b.nm.as_ref().unwrap().outcome.is_some());
    assert_eq!(anna.nm.as_ref().unwrap().outcome, Some(NetOutcome::Lose));
    assert_eq!(bob.nm.as_ref().unwrap().outcome, Some(NetOutcome::Win));

    // the server counted it once and told everybody
    run_until(&mut anna, &mut bob, "records", |a, b| {
        let skill = |c: &Client, n: &str| c.lobby.players.iter().find(|p| p.name == n).map(|p| p.skill.clone());
        skill(a, "Bob").as_deref() == Some("1/0/0") && skill(b, "Anna").as_deref() == Some("0/1/0")
    });

    // rematch: Anna asks, Bob accepts and becomes player 1
    let r = anna.lobby.play_again();
    for m in r.send {
        anna.conn.send(m);
    }
    run_until(&mut anna, &mut bob, "rematch request", |_, b| b.lobby.rematch_requested);
    let r = bob.lobby.play_again();
    assert!(matches!(r.event, Some(LobbyEvent::StartMatch { number: 1, .. })));
    for m in r.send {
        bob.conn.send(m);
    }
    anna.nm = None;
    bob.nm = Some(NetMatch::new(0, ["Bob".into(), "Anna".into()], 654, 0));
    run_until(&mut anna, &mut bob, "rematch running", |a, b| {
        a.nm.as_ref().is_some_and(|n| n.phase == NetPhase::Playing) && b.nm.as_ref().is_some_and(|n| n.phase == NetPhase::Playing)
    });
    assert_eq!(anna.nm.as_ref().unwrap().me, 1);
    assert_eq!(anna.nm.as_ref().unwrap().world.cells, bob.nm.as_ref().unwrap().world.cells);

    // Bob surrenders: Anna wins at once
    bob.conn.send(Msg::Surrender(None));
    run_until(&mut anna, &mut bob, "surrender result", |a, b| a.nm.as_ref().unwrap().outcome.is_some() && b.nm.as_ref().unwrap().outcome.is_some());
    assert_eq!(anna.nm.as_ref().unwrap().outcome, Some(NetOutcome::Win));
    assert_eq!(bob.nm.as_ref().unwrap().outcome, Some(NetOutcome::Lose));
    srv.stop();
}

#[test]
fn leaving_mid_match_makes_the_other_player_win() {
    let srv = server::spawn("127.0.0.1:0").unwrap();
    let addr = srv.addr.to_string();
    let mut anna = Client::new(&addr, "Anna");
    let mut bob = Client::new(&addr, "Bob");
    run_until(&mut anna, &mut bob, "both in the room", |a, b| a.lobby.players.len() == 2 && b.lobby.players.len() == 2);
    for m in bob.lobby.click("Anna").send {
        bob.conn.send(m);
    }
    run_until(&mut anna, &mut bob, "request arrives", |a, _| a.lobby.players.iter().any(|p| p.code() == "1"));
    for m in anna.lobby.click("Bob").send {
        anna.conn.send(m);
    }
    anna.nm = Some(NetMatch::new(0, ["Anna".into(), "Bob".into()], 5, 0));
    run_until(&mut anna, &mut bob, "both playing", |a, b| {
        a.nm.as_ref().is_some_and(|n| n.phase == NetPhase::Playing) && b.nm.as_ref().is_some_and(|n| n.phase == NetPhase::Playing)
    });
    let Client { conn, .. } = bob;
    drop(conn);
    for _ in 0..2000 {
        // Bob's side is gone; only Anna keeps running
        for e in anna.conn.poll() {
            if let NetEvent::Msg(m) = e {
                let w = anna.wall();
                anna.nm.as_mut().unwrap().handle(m, w);
            }
        }
        if anna.nm.as_ref().unwrap().outcome.is_some() {
            break;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    assert_eq!(anna.nm.as_ref().unwrap().outcome, Some(NetOutcome::Win));
    srv.stop();
}
