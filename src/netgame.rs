//! Online match: port of the networking parts of `Game`, `MyPlayer`, `Bomb` and `BombManager`.
//!
//! Like the original, every client simulates the whole board. The level comes from a seed chosen by player 1;
//! afterwards each client only reports what its own penguin does (key changes with position, bombs, pick-ups,
//! kicks, death) and applies the same reports of the other client to the remote penguin.

use crate::game::{GAME_OVER_FRAMES, INIT_FRAMES};
use crate::player::Player;
use crate::proto::{Cmd, Msg};
use crate::world::{Outgoing, World, DT_MS};

/// `Game.pingIntervall`
pub const PING_INTERVAL_MS: i64 = 10_000;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum NetPhase {
    Init,
    /// Player 2 waits for the seed of player 1.
    Waiting,
    Playing,
    GameOver,
    Finished,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum NetOutcome {
    Win,
    Lose,
    Draw,
}

#[derive(Clone, Debug)]
pub struct NetMatch {
    /// Index of the penguin this client controls: 0 = player 1 (host of the seed), 1 = player 2.
    pub me: usize,
    pub names: [String; 2],
    pub world: World,
    /// False until the level exists (player 2 before the seed arrived).
    pub ready: bool,
    pub phase: NetPhase,
    pub frame: u32,
    pub over_counter: u32,
    pub outcome: Option<NetOutcome>,
    pub my_ping: i64,
    pub my_fps: i64,
    pub remote_ping: i64,
    pub remote_fps: i64,
    seed: i64,
    drawn: bool,
    has_won: bool,
    old_keys: [bool; 4],
    prio1: bool,
    ping_sent: i64,
    ping_result_to_other: bool,
    start_wall: i64,
    outbox: Vec<Msg>,
}

impl NetMatch {
    /// `seed` is only used by player 1 (`Util.randomMinMax(0, 20000)` in the original); `wall_ms` is a monotonic clock.
    pub fn new(me: usize, names: [String; 2], seed: i64, wall_ms: i64) -> Self {
        Self {
            me,
            names,
            world: World::new(0),
            ready: false,
            phase: NetPhase::Init,
            frame: 0,
            over_counter: 0,
            outcome: None,
            my_ping: 100,
            my_fps: 0,
            remote_ping: 100,
            remote_fps: 0,
            seed,
            drawn: false,
            has_won: false,
            old_keys: [false; 4],
            prio1: false,
            ping_sent: wall_ms - PING_INTERVAL_MS + 1000,
            ping_result_to_other: false,
            start_wall: wall_ms,
            outbox: Vec::new(),
        }
    }

    pub fn remote(&self) -> usize {
        1 - self.me
    }

    pub fn local_player(&self) -> &Player {
        &self.world.players[self.me]
    }

    pub fn local_player_mut(&mut self) -> &mut Player {
        &mut self.world.players[self.me]
    }

    /// Keys only act while playing (`Game.onKeyDown` ignores `GAME_OVER`).
    pub fn accepts_input(&self) -> bool {
        self.phase == NetPhase::Playing
    }

    pub fn take_outbox(&mut self) -> Vec<Msg> {
        std::mem::take(&mut self.outbox)
    }

    /// `Game.reset` + `grid.init`: build the level and the world for this client.
    fn start_level(&mut self, seed: i64) {
        self.world = World::new(seed);
        self.world.local = Some(self.me);
        self.world.next_bomb_id = if self.me == 0 { 5000 } else { 10000 };
        self.ready = true;
        self.phase = NetPhase::Playing;
    }

    fn send_prio1(&mut self, m: Msg) {
        self.outbox.push(m);
        self.prio1 = true;
    }

    /// One 25 ms frame.
    pub fn tick(&mut self, wall_ms: i64) {
        self.frame += 1;
        self.prio1 = false;
        match self.phase {
            NetPhase::Init => {
                if self.frame == INIT_FRAMES {
                    if self.me == 0 {
                        self.start_level(self.seed);
                        self.send_prio1(Msg::Seed(self.seed));
                    } else {
                        self.phase = NetPhase::Waiting;
                    }
                }
            }
            NetPhase::Playing => {
                self.world.update_players();
                self.world.update_objects();
                self.world.now += DT_MS;
                self.flush_outgoing();
                self.send_key_msg(wall_ms);
            }
            NetPhase::GameOver => {
                self.world.advance_death_anims();
                self.world.update_objects();
                self.world.now += DT_MS;
                self.flush_outgoing();
                self.over_counter += 1;
                if self.over_counter > GAME_OVER_FRAMES {
                    self.finish();
                }
            }
            NetPhase::Waiting | NetPhase::Finished => {}
        }
    }

    /// Turns what the local penguin did this frame into messages; each carries the current position and key state.
    fn flush_outgoing(&mut self) {
        let events: Vec<Outgoing> = self.world.outgoing.drain(..).collect();
        if events.is_empty() {
            return;
        }
        let p = self.local_player();
        let keys = p.key_state();
        let cmd = Cmd::new(p.x, p.y, keys);
        let (speed, blast) = (p.speed, p.num_blast);
        for o in events {
            let c = cmd.clone();
            let m = match o {
                Outgoing::Pickup { cx, cy } => Msg::Pickup { cmd: c, cx, cy, speed, blast },
                Outgoing::BombPlaced { cx, cy } => Msg::Bomb { cmd: c, cx, cy },
                Outgoing::Kick { cx, cy, id, dx, dy, t } => Msg::Kick { cmd: c, cx, cy, id, dx, dy, t },
                Outgoing::Stop { cx, cy, id, t } => Msg::Stop { cmd: c, cx, cy, id, t },
                Outgoing::Detonate { cx, cy, id } => Msg::Detonate { cmd: c, cx, cy, id },
                Outgoing::Die => {
                    self.has_won = false;
                    if self.phase == NetPhase::Playing {
                        self.phase = NetPhase::GameOver;
                    }
                    Msg::Die { x: c.x, y: c.y }
                }
            };
            if !matches!(m, Msg::Die { .. }) {
                self.old_keys = keys;
            }
            self.send_prio1(m);
        }
    }

    /// `Game.sendKeyMsg`: at most one low-priority message per frame.
    fn send_key_msg(&mut self, wall_ms: i64) {
        if self.prio1 {
            return;
        }
        let p = self.local_player();
        let keys = p.key_state();
        if keys != self.old_keys {
            let cmd = Cmd::new(p.x, p.y, keys);
            self.outbox.push(Msg::Move(cmd));
            self.old_keys = keys;
        } else if self.ping_sent < wall_ms - PING_INTERVAL_MS {
            self.ping_sent = wall_ms;
            self.send_prio1(Msg::Ping);
        } else if self.ping_result_to_other {
            self.outbox.push(Msg::Stats { ping: self.my_ping, fps: self.my_fps });
            self.ping_result_to_other = false;
        }
    }

    /// End of the 3 s game-over delay: the winner reports the result (`winGame` / `drawGame`).
    fn finish(&mut self) {
        if self.drawn {
            if self.has_won {
                self.outbox.push(Msg::DrawGame);
            }
            self.outcome = Some(NetOutcome::Draw);
        } else if self.has_won {
            self.outbox.push(Msg::WinGame);
            self.outcome = Some(NetOutcome::Win);
        } else {
            self.outcome = Some(NetOutcome::Lose);
        }
        self.phase = NetPhase::Finished;
    }

    /// The match ends right away (`Game.win/loose`: opponent surrendered or left).
    fn end_now(&mut self, outcome: NetOutcome) {
        if self.phase != NetPhase::Finished {
            self.outcome = Some(outcome);
            self.phase = NetPhase::Finished;
        }
    }

    fn remote_cmd(&mut self, c: &Cmd) {
        let r = self.remote();
        if !self.world.players[r].dead {
            self.world.players[r].remote_command(c.keys(), c.x as f64, c.y as f64);
        }
    }

    /// `Connector.onSockXML` for the in-game messages. Other messages are ignored.
    pub fn handle(&mut self, m: Msg, wall_ms: i64) {
        let r = self.remote();
        match m {
            Msg::Seed(s) => {
                if self.me == 1 && !self.ready && self.phase != NetPhase::Finished {
                    self.start_level(s);
                }
            }
            Msg::Pong => {
                self.my_ping = wall_ms - self.ping_sent;
                let secs = (wall_ms - self.start_wall) as f64 / 1000.0;
                self.my_fps = if secs > 0.0 { (self.frame as f64 / secs).round() as i64 } else { 0 };
                self.ping_result_to_other = true;
            }
            Msg::Stats { ping, fps } => {
                self.remote_ping = ping;
                self.remote_fps = fps;
            }
            Msg::Draw => {
                self.drawn = true;
                if self.ready {
                    for p in &mut self.world.players {
                        p.die();
                    }
                }
            }
            Msg::Surrender(winner) => match winner {
                Some(w) if w == self.names[self.me] => self.end_now(NetOutcome::Win),
                Some(w) if w == self.names[r] => self.end_now(NetOutcome::Lose),
                _ => self.end_now(NetOutcome::Draw),
            },
            Msg::PlayerLeft(n) if n == self.names[r] => self.end_now(NetOutcome::Win),
            _ if !self.ready => {}
            Msg::Move(c) => self.remote_cmd(&c),
            Msg::Bomb { cmd, cx, cy } => {
                self.world.remote_place_bomb(cx, cy, r);
                self.remote_cmd(&cmd);
            }
            Msg::Pickup { cmd, cx, cy, speed, blast } => {
                self.world.remote_pickup(cx, cy, r, speed, blast);
                self.remote_cmd(&cmd);
            }
            Msg::Kick { cmd, cx, cy, id, dx, dy, t } => {
                self.world.remote_kick(cx, cy, r, dx, dy, t, id);
                self.remote_cmd(&cmd);
            }
            Msg::Stop { cmd, cx, cy, id, t } => {
                self.world.remote_stop(id, cx, cy, t);
                self.remote_cmd(&cmd);
            }
            Msg::Detonate { cmd, cx, cy, id } => {
                self.world.remote_detonate(id, cx, cy);
                self.remote_cmd(&cmd);
            }
            Msg::Die { x, y } => {
                self.world.remote_die(r, x as f64, y as f64);
                self.has_won = true;
                if matches!(self.phase, NetPhase::Init | NetPhase::Waiting | NetPhase::Playing) {
                    self.phase = NetPhase::GameOver;
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::grid::{Cell, Kind};
    use crate::player::Dir;
    use std::collections::VecDeque;

    /// Two clients joined by a link with a fixed delay in frames (messages arrive in order).
    struct Pair {
        a: NetMatch,
        b: NetMatch,
        a_to_b: VecDeque<(u64, Msg)>,
        b_to_a: VecDeque<(u64, Msg)>,
        delay: u64,
        t: u64,
        sent_by_a: Vec<Msg>,
        sent_by_b: Vec<Msg>,
    }

    impl Pair {
        fn new(delay: u64) -> Self {
            let names = ["Anna".to_string(), "Bob".to_string()];
            Pair {
                a: NetMatch::new(0, names.clone(), 4242, 0),
                b: NetMatch::new(1, names, 0, 0),
                a_to_b: VecDeque::new(),
                b_to_a: VecDeque::new(),
                delay,
                t: 0,
                sent_by_a: Vec::new(),
                sent_by_b: Vec::new(),
            }
        }

        fn step(&mut self) {
            self.t += 1;
            let wall = (self.t * DT_MS as u64) as i64;
            while let Some((due, _)) = self.a_to_b.front() {
                if *due > self.t {
                    break;
                }
                let (_, m) = self.a_to_b.pop_front().unwrap();
                self.b.handle(m, wall);
            }
            while let Some((due, _)) = self.b_to_a.front() {
                if *due > self.t {
                    break;
                }
                let (_, m) = self.b_to_a.pop_front().unwrap();
                self.a.handle(m, wall);
            }
            self.a.tick(wall);
            self.b.tick(wall);
            for m in self.a.take_outbox() {
                self.sent_by_a.push(m.clone());
                self.a_to_b.push_back((self.t + self.delay, m));
            }
            for m in self.b.take_outbox() {
                self.sent_by_b.push(m.clone());
                self.b_to_a.push_back((self.t + self.delay, m));
            }
        }

        fn run(&mut self, n: u32) {
            for _ in 0..n {
                self.step();
            }
        }

        /// Starts the match and frees both start corners so that tests can move and place bombs at will.
        fn started(delay: u64) -> Pair {
            let mut p = Pair::new(delay);
            p.run(INIT_FRAMES + 2 + delay as u32 * 2);
            assert_eq!(p.a.phase, NetPhase::Playing);
            assert_eq!(p.b.phase, NetPhase::Playing);
            p
        }
    }

    #[test]
    fn player_two_waits_for_the_seed_and_both_build_the_same_level() {
        let mut p = Pair::new(2);
        p.run(INIT_FRAMES);
        assert_eq!(p.a.phase, NetPhase::Playing);
        assert_eq!(p.b.phase, NetPhase::Waiting);
        assert!(!p.b.ready);
        p.run(3);
        assert_eq!(p.b.phase, NetPhase::Playing);
        assert!(p.sent_by_a.contains(&Msg::Seed(4242)));
        assert_eq!(p.a.world.cells, p.b.world.cells);
        assert_eq!(p.a.world.cells, crate::grid::generate(4242));
        assert_eq!(p.a.world.local, Some(0));
        assert_eq!(p.b.world.local, Some(1));
        assert_eq!(p.a.world.next_bomb_id, 5000);
        assert_eq!(p.b.world.next_bomb_id, 10000);
    }

    #[test]
    fn key_changes_are_sent_once_with_position_and_the_remote_penguin_follows() {
        let mut p = Pair::started(2);
        p.a.local_player_mut().key_down(Dir::Right);
        p.run(1);
        let moves: Vec<_> = p.sent_by_a.iter().filter(|m| matches!(m, Msg::Move(_))).collect();
        assert_eq!(moves.len(), 1);
        p.run(10);
        assert_eq!(p.sent_by_a.iter().filter(|m| matches!(m, Msg::Move(_))).count(), 1, "no message while the keys stay the same");
        // b simulates a's walking with the same rules, so both agree within the message delay
        let (xa, xb) = (p.a.world.players[0].x, p.b.world.players[0].x);
        assert!(xa > 150.0);
        assert!((xa - xb).abs() <= 2.0 * 0.15 * 25.0 + 1e-9, "a={xa} b={xb}");
        // stop: b snaps to the exact stop position
        p.a.local_player_mut().keys_released([false; 4], false);
        p.run(4);
        assert!((p.a.world.players[0].x - p.b.world.players[0].x).abs() <= 0.5, "positions are sent rounded to whole pixels");
        assert!(!p.b.world.players[0].right);
    }

    #[test]
    fn bomb_is_mirrored_and_explodes_on_both_sides() {
        let mut p = Pair::started(1);
        p.a.local_player_mut().key_space_down();
        p.run(3);
        assert_eq!(p.a.world.bombs.len(), 1);
        assert_eq!(p.b.world.bombs.len(), 1);
        assert_eq!((p.b.world.bombs[0].cx, p.b.world.bombs[0].cy, p.b.world.bombs[0].owner), (0, 0, 0));
        assert_eq!(p.b.world.cells[0][0].kind, Kind::ArmedBomb);
        // a walks away so that the blast does not hit; b's remote penguin is moved by a's messages
        p.a.local_player_mut().keys_released([false; 4], false);
        p.a.local_player_mut().key_down(Dir::Right);
        p.run(30);
        p.a.local_player_mut().key_down(Dir::Down);
        p.run(160);
        assert!(p.a.world.bombs.is_empty() && p.b.world.bombs.is_empty());
        assert!(!p.a.world.players[0].dead, "a escaped");
        assert!(!p.b.world.players[0].dead);
    }

    #[test]
    fn pickup_is_applied_once_by_its_owner_and_mirrored_to_the_other_client() {
        let mut p = Pair::started(1);
        for w in [&mut p.a.world, &mut p.b.world] {
            w.cells[0][1] = Cell { kind: Kind::ItemSpeed, open: true };
            w.cells[0][2] = Cell { kind: Kind::ItemBlast, open: true };
        }
        p.a.local_player_mut().key_down(Dir::Right);
        p.run(25);
        p.a.local_player_mut().keys_released([false; 4], false);
        p.run(5);
        assert!(p.a.world.players[0].speed > 0.15);
        assert_eq!(p.a.world.players[0].num_blast, 3);
        // b's copy of player 1 got speed and blast from the message, and the items are gone on both sides
        assert_eq!(p.b.world.players[0].speed, p.a.world.players[0].speed);
        assert_eq!(p.b.world.players[0].num_blast, 3);
        assert_eq!(p.b.world.cells[0][1], Cell::FREE);
        assert_eq!(p.b.world.cells[0][2], Cell::FREE);
        let picks = p.sent_by_a.iter().filter(|m| matches!(m, Msg::Pickup { .. })).count();
        assert_eq!(picks, 2);
        assert_eq!(p.sent_by_b.iter().filter(|m| matches!(m, Msg::Pickup { .. })).count(), 0, "b never reports a's pick-ups");
    }

    #[test]
    fn remote_penguin_does_not_pick_up_items_on_its_own() {
        let mut p = Pair::started(8); // long delay: b would walk over the item long before a's message arrives
        p.a.world.cells[0][1] = Cell { kind: Kind::ItemSpeed, open: true };
        p.b.world.cells[0][1] = Cell { kind: Kind::ItemSpeed, open: true };
        p.a.local_player_mut().key_down(Dir::Right);
        p.run(12);
        assert_eq!(p.b.world.players[0].speed, 0.15, "b waits for message 18");
        p.run(12);
        assert!(p.b.world.players[0].speed > 0.15);
    }

    #[test]
    fn own_death_is_reported_and_the_other_client_wins_after_the_delay() {
        let mut p = Pair::started(1);
        // a stands still on its bomb
        p.a.local_player_mut().key_space_down();
        p.run(3);
        let mut frames = 0;
        while p.a.phase == NetPhase::Playing && frames < 300 {
            p.step();
            frames += 1;
        }
        assert_eq!(p.a.phase, NetPhase::GameOver);
        assert!(p.a.world.players[0].dead);
        p.run(2);
        assert!(p.sent_by_a.iter().any(|m| matches!(m, Msg::Die { .. })));
        assert_eq!(p.b.phase, NetPhase::GameOver);
        assert!(p.b.world.players[0].dead);
        assert!(!p.b.world.players[1].dead);
        p.run(GAME_OVER_FRAMES + 5);
        assert_eq!(p.a.outcome, Some(NetOutcome::Lose));
        assert_eq!(p.b.outcome, Some(NetOutcome::Win));
        assert!(p.sent_by_b.contains(&Msg::WinGame));
        assert!(!p.sent_by_a.contains(&Msg::WinGame));
    }

    #[test]
    fn both_dying_before_hearing_of_each_other_is_a_draw_once_the_server_says_so() {
        let mut p = Pair::started(1);
        p.a.handle(Msg::Die { x: 600, y: 440 }, 0); // b died
        p.b.handle(Msg::Die { x: 120, y: 40 }, 0); // a died
        p.a.handle(Msg::Draw, 0);
        p.b.handle(Msg::Draw, 0);
        p.run(GAME_OVER_FRAMES + 5);
        assert_eq!(p.a.outcome, Some(NetOutcome::Draw));
        assert_eq!(p.b.outcome, Some(NetOutcome::Draw));
        assert!(p.sent_by_a.contains(&Msg::DrawGame));
    }

    #[test]
    fn surrender_and_leaving_end_the_match_immediately() {
        let mut p = Pair::started(1);
        p.a.handle(Msg::Surrender(Some("Anna".into())), 0);
        p.b.handle(Msg::Surrender(Some("Anna".into())), 0);
        assert_eq!(p.a.outcome, Some(NetOutcome::Win));
        assert_eq!(p.b.outcome, Some(NetOutcome::Lose));

        let mut p = Pair::started(1);
        p.a.handle(Msg::PlayerLeft("Bob".into()), 0);
        assert_eq!(p.a.outcome, Some(NetOutcome::Win));
        let mut p = Pair::started(1);
        p.a.handle(Msg::PlayerLeft("Somebody".into()), 0);
        assert_eq!(p.a.outcome, None);
    }

    #[test]
    fn kicked_bomb_is_mirrored_and_its_detonation_is_announced_by_the_kicker() {
        let mut p = Pair::started(1);
        p.a.local_player_mut().kick = true;
        // a bomb in the cell right of a, a keeps walking into it
        for w in [&mut p.a.world, &mut p.b.world] {
            w.set_bomb(1, 0, 1);
        }
        p.a.local_player_mut().key_down(Dir::Right);
        p.run(8);
        assert!(p.sent_by_a.iter().any(|m| matches!(m, Msg::Kick { id: 5000, .. })), "{:?}", p.sent_by_a);
        let ba = p.a.world.bombs.iter().find(|b| b.id == Some(5000)).unwrap();
        assert!(ba.moving && !ba.remote_kicked);
        let bb = p.b.world.bombs.iter().find(|b| b.id == Some(5000)).unwrap();
        assert!(bb.moving && bb.remote_kicked);
        // keep both penguins away from the blast
        p.a.local_player_mut().keys_released([false; 4], false);
        p.a.local_player_mut().key_down(Dir::Down);
        p.run(100);
        p.a.local_player_mut().keys_released([false; 4], false);
        // a's fuse runs out first (b waits for message 10)
        let mut seen_detonate = false;
        for _ in 0..200 {
            p.step();
            seen_detonate |= p.sent_by_a.iter().any(|m| matches!(m, Msg::Detonate { id: 5000, .. }));
            if p.a.world.bombs.is_empty() {
                break;
            }
        }
        assert!(seen_detonate);
        p.run(3);
        assert!(p.b.world.bombs.is_empty(), "b detonated on message 10, not on its own fuse");
        assert!(p.sent_by_b.iter().all(|m| !matches!(m, Msg::Detonate { .. })));
    }

    #[test]
    fn bomb_hitting_the_local_penguin_stops_and_reports_to_the_kicker() {
        let mut p = Pair::started(1);
        // b's penguin (player 2) stands at (600,440); a kicked bomb of player 1 flies towards it along the bottom row
        for w in [&mut p.a.world, &mut p.b.world] {
            for cx in 8..13 {
                w.cells[10][cx] = Cell::FREE;
            }
            w.set_bomb(8, 10, 0);
        }
        p.a.world.bombs[0].id = Some(5000);
        p.a.world.bombs[0].moving = true;
        p.a.world.bombs[0].kicked = true;
        p.a.world.bombs[0].kicked_by = Some(0);
        p.a.world.bombs[0].dx = 0.1;
        p.b.world.remote_kick(8, 10, 0, 0.1, 0.0, 3000, 5000);
        p.run(100);
        let stop = p.sent_by_b.iter().find(|m| matches!(m, Msg::Stop { .. })).unwrap_or_else(|| {
            panic!("b stops the bomb that hits its penguin: bombs={:?} player={:?} sent={:?}", p.b.world.bombs, p.b.world.players[1].x, p.sent_by_b)
        });
        let Msg::Stop { id, cx, .. } = stop else { unreachable!() };
        assert_eq!(*id, 5000);
        assert!(*cx <= 11);
        let bomb_b = p.b.world.bombs.iter().find(|b| b.id == Some(5000)).unwrap();
        assert!(!bomb_b.moving);
        let bomb_a = p.a.world.bombs.iter().find(|b| b.id == Some(5000)).unwrap();
        assert!(!bomb_a.moving && bomb_a.remote_kicked, "a adopts the stop position and waits for b's detonation");
        assert_eq!((bomb_a.cx, bomb_a.cy), (bomb_b.cx, bomb_b.cy));
    }

    #[test]
    fn ping_is_sent_after_a_second_and_stats_follow_the_pong() {
        let mut p = Pair::started(1);
        p.run(45);
        assert!(p.sent_by_a.contains(&Msg::Ping));
        p.a.handle(Msg::Pong, 1500);
        assert!(p.a.my_ping >= 0);
        p.run(2);
        assert!(p.sent_by_a.iter().any(|m| matches!(m, Msg::Stats { .. })));
        assert_eq!(p.b.remote_ping, p.a.my_ping);
    }

    #[test]
    fn input_is_only_accepted_while_playing() {
        let mut p = Pair::new(1);
        assert!(!p.a.accepts_input());
        p.run(INIT_FRAMES);
        assert!(p.a.accepts_input());
        assert!(!p.b.accepts_input());
    }
}
