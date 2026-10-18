//! Online screens: connect form, player room, running match and result (the original's `Connector`, `PlayerRoom`,
//! `Game` and `AfterMatch` frames).

use crate::assets::Assets;
use crate::{apply_keys, button_rect, clicked, draw_game, text, text_centered, AGAIN_POS, INK, MENU_POS, STEP};
use bomberpengu::lobby::{Entry, Lobby, LobbyEvent, Reaction};
use bomberpengu::net::{Connection, NetEvent};
use bomberpengu::netgame::{NetMatch, NetOutcome, NetPhase};
use bomberpengu::proto::Msg;
use bomberpengu::server::valid_name;
use bomberpengu::world::Event;
use macroquad::prelude::*;

const ROW_H: f32 = 22.0;
const LIST_TOP: f32 = 142.0;
const VISIBLE_ROWS: usize = 9;
const CHAT_LINES: usize = 7;
const MAX_ADDR_CHARS: usize = 64;

enum Stage {
    Form,
    Connecting,
    Room,
    Match(Box<NetMatch>),
    After(NetOutcome),
}

pub struct Online {
    stage: Stage,
    name: String,
    server: String,
    edit_server: bool,
    status: String,
    conn: Option<Connection>,
    lobby: Lobby,
    chat_input: String,
    selected: usize,
    acc: f64,
    surrendered: bool,
    /// Test aids for scripted runs: challenge this player / accept every challenge automatically.
    pub auto_challenge: Option<String>,
    pub auto_accept: bool,
}

fn wall_ms() -> i64 {
    (get_time() * 1000.0) as i64
}

fn clip(s: &str, max: usize) -> String {
    s.chars().take(max).collect()
}

fn drain_chars() {
    while get_char_pressed().is_some() {}
}

/// Appends typed characters to `field` (and handles Backspace).
fn edit(field: &mut String, max: usize) {
    while let Some(c) = get_char_pressed() {
        if !c.is_control() && field.chars().count() < max {
            field.push(c);
        }
    }
    if is_key_pressed(KeyCode::Backspace) {
        field.pop();
    }
}

fn panel(x: f32, y: f32, w: f32, h: f32) {
    draw_rectangle(x, y, w, h, Color::new(0.93, 0.97, 1.0, 0.92));
    draw_rectangle_lines(x, y, w, h, 4.0, INK);
}

fn status_color(e: &Entry) -> Color {
    match e.code() {
        "0" => Color::new(0.2, 0.7, 0.3, 1.0),
        "1" | "13" => Color::new(0.95, 0.6, 0.1, 1.0),
        "2" | "23" => Color::new(0.2, 0.45, 0.9, 1.0),
        "3" => Color::new(0.8, 0.15, 0.1, 1.0),
        _ => GRAY,
    }
}

impl Online {
    pub fn new(name: &str, server: &str) -> Self {
        drain_chars();
        Self {
            stage: Stage::Form,
            name: name.to_string(),
            server: server.to_string(),
            edit_server: false,
            status: String::new(),
            conn: None,
            lobby: Lobby::new(name),
            chat_input: String::new(),
            selected: 0,
            acc: 0.0,
            surrendered: false,
            auto_challenge: None,
            auto_accept: false,
        }
    }

    pub fn connect(&mut self) {
        if !valid_name(&self.name) {
            self.status = "Ungueltiger Name (1-20 Zeichen: Buchstaben, Ziffern, Leerzeichen, _ - .)".into();
            return;
        }
        self.lobby = Lobby::new(&self.name);
        self.conn = Some(Connection::connect(&self.server, &self.name));
        self.status = "Verbinde...".into();
        self.stage = Stage::Connecting;
    }

    fn disconnect(&mut self, why: &str) {
        self.conn = None;
        if self.status.is_empty() || self.status == "Verbinde..." {
            self.status = why.to_string();
        }
        self.stage = Stage::Form;
    }

    fn send_all(&self, msgs: Vec<Msg>) {
        if let Some(c) = &self.conn {
            for m in msgs {
                c.send(m);
            }
        }
    }

    fn begin_match(&mut self, a: &Assets, opponent: &str, number: u8) {
        let me = usize::from(number.max(1)) - 1;
        let mine = self.lobby.my_name.clone();
        let names = if me == 0 { [mine, opponent.to_string()] } else { [opponent.to_string(), mine] };
        let seed = rand::gen_range(0, 20001);
        self.stage = Stage::Match(Box::new(NetMatch::new(me, names, seed, wall_ms())));
        self.surrendered = false;
        self.acc = 0.0;
        a.play(&a.sounds.music);
    }

    fn react(&mut self, a: &Assets, r: Reaction) {
        self.send_all(r.send);
        match r.event {
            Some(LobbyEvent::StartMatch { opponent, number }) => {
                if !matches!(self.stage, Stage::Match(_)) {
                    self.begin_match(a, &opponent, number);
                }
            }
            Some(LobbyEvent::Error(t)) => self.status = t,
            Some(LobbyEvent::RematchRequested) | None => {}
        }
    }

    fn pump(&mut self, a: &Assets) {
        let events = self.conn.as_ref().map(|c| c.poll()).unwrap_or_default();
        for e in events {
            match e {
                NetEvent::Closed(why) => {
                    self.disconnect(&why);
                    return;
                }
                NetEvent::Msg(m) => {
                    if matches!((&self.stage, &m), (Stage::Connecting, Msg::UserList(_))) {
                        self.stage = Stage::Room;
                        self.status.clear();
                    }
                    if let Stage::Match(nm) = &mut self.stage {
                        nm.handle(m.clone(), wall_ms());
                    }
                    let r = self.lobby.handle(&m);
                    self.react(a, r);
                }
            }
        }
        if self.lobby.players.len() <= self.selected {
            self.selected = self.lobby.players.len().saturating_sub(1);
        }
    }

    /// One frame; `false` = back to the title screen.
    pub fn frame(&mut self, a: &Assets, dt: f64) -> bool {
        self.acc = (self.acc + dt).min(STEP * 5.0);
        self.pump(a);
        let keep = match self.stage {
            Stage::Form => self.frame_form(),
            Stage::Connecting => self.frame_connecting(),
            Stage::Room => self.frame_room(a),
            Stage::Match(_) => self.frame_match(a),
            Stage::After(_) => self.frame_after(a),
        };
        if !keep {
            self.conn = None;
        }
        keep
    }

    // ---- connect form --------------------------------------------------------------------------------------------

    fn frame_form(&mut self) -> bool {
        if is_key_pressed(KeyCode::Escape) {
            return false;
        }
        if is_key_pressed(KeyCode::Tab) {
            self.edit_server = !self.edit_server;
        }
        let (field, max) = if self.edit_server { (&mut self.server, MAX_ADDR_CHARS) } else { (&mut self.name, 20) };
        edit(field, max);
        if is_key_pressed(KeyCode::Enter) {
            self.connect();
        }
        let a = &self.name;
        draw_rectangle(0.0, 0.0, 640.0, 640.0, Color::new(0.75, 0.88, 1.0, 1.0));
        panel(60.0, 120.0, 520.0, 380.0);
        text_centered("Online spielen", 320.0, 180.0, 44.0, INK);
        let rows = [("Name", a.as_str(), !self.edit_server), ("Server (Adresse:Port)", self.server.as_str(), self.edit_server)];
        for (i, (label, value, active)) in rows.iter().enumerate() {
            let y = 230.0 + i as f32 * 90.0;
            text(label, 90.0, y, 20.0, INK);
            draw_rectangle(90.0, y + 10.0, 460.0, 34.0, WHITE);
            draw_rectangle_lines(90.0, y + 10.0, 460.0, 34.0, if *active { 4.0 } else { 2.0 }, INK);
            let cursor = if *active && (get_time() * 2.0) as i64 % 2 == 0 { "|" } else { "" };
            text(&format!("{value}{cursor}"), 98.0, y + 34.0, 24.0, INK);
        }
        text("Tab: Feld wechseln    Enter: verbinden    Esc: zurueck", 90.0, 440.0, 18.0, INK);
        if !self.status.is_empty() {
            text(&clip(&self.status, 60), 90.0, 475.0, 20.0, Color::new(0.75, 0.1, 0.05, 1.0));
        }
        true
    }

    fn frame_connecting(&mut self) -> bool {
        if is_key_pressed(KeyCode::Escape) {
            self.conn = None;
            self.status.clear();
            self.stage = Stage::Form;
        }
        draw_rectangle(0.0, 0.0, 640.0, 640.0, Color::new(0.75, 0.88, 1.0, 1.0));
        text_centered(&format!("Verbinde mit {} ...", clip(&self.server, 40)), 320.0, 320.0, 28.0, INK);
        text_centered("Esc: abbrechen", 320.0, 360.0, 18.0, INK);
        true
    }

    // ---- player room ---------------------------------------------------------------------------------------------

    fn act_on_selected(&mut self, a: &Assets) {
        let Some(name) = self.lobby.players.get(self.selected).map(|p| p.name.clone()) else { return };
        let r = self.lobby.click(&name);
        self.react(a, r);
    }

    fn first_row(&self) -> usize {
        if self.selected >= VISIBLE_ROWS {
            self.selected + 1 - VISIBLE_ROWS
        } else {
            0
        }
    }

    fn frame_room(&mut self, a: &Assets) -> bool {
        if is_key_pressed(KeyCode::Escape) {
            self.disconnect("");
            self.status.clear();
            return true;
        }
        // scripted helpers
        if let Some(target) = self.auto_challenge.clone() {
            if self.lobby.players.iter().any(|p| p.name == target && p.code() == "0") {
                let r = self.lobby.click(&target);
                self.react(a, r);
            }
        }
        if self.auto_accept {
            if let Some(n) = self.lobby.players.iter().find(|p| p.code() == "1").map(|p| p.name.clone()) {
                let r = self.lobby.click(&n);
                self.react(a, r);
            }
        }
        if !matches!(self.stage, Stage::Room) {
            return true;
        }
        let count = self.lobby.players.len();
        if is_key_pressed(KeyCode::Down) && self.selected + 1 < count {
            self.selected += 1;
        }
        if is_key_pressed(KeyCode::Up) {
            self.selected = self.selected.saturating_sub(1);
        }
        if is_key_pressed(KeyCode::F2) {
            let m = self.lobby.toggle_challenge_all();
            self.send_all(m);
        }
        edit(&mut self.chat_input, 200);
        if is_key_pressed(KeyCode::Enter) {
            if self.chat_input.trim().is_empty() {
                self.act_on_selected(a);
            } else if let Some(m) = self.lobby.chat_message(&self.chat_input) {
                self.send_all(vec![m]);
                self.chat_input.clear();
            }
        }
        let first = self.first_row();
        if is_mouse_button_pressed(MouseButton::Left) {
            let (mx, my) = mouse_position();
            if (40.0..400.0).contains(&mx) && my >= LIST_TOP && my < LIST_TOP + ROW_H * VISIBLE_ROWS as f32 {
                let idx = first + ((my - LIST_TOP) / ROW_H) as usize;
                if idx < count {
                    self.selected = idx;
                    self.act_on_selected(a);
                }
            }
        }
        if !matches!(self.stage, Stage::Room) {
            return true;
        }
        self.draw_room(a);
        true
    }

    fn draw_room(&self, a: &Assets) {
        draw_texture(&a.bg_after, 0.0, 0.0, WHITE);
        panel(30.0, 30.0, 580.0, 580.0);
        text_centered("Spielerraum", 320.0, 76.0, 38.0, INK);
        text(&format!("{} auf {}", self.lobby.my_name, clip(&self.server, 40)), 48.0, 100.0, 16.0, INK);
        text("Spieler", 66.0, 134.0, 16.0, INK);
        text("G/V/U", 330.0, 134.0, 16.0, INK);
        let first = self.first_row();
        for (i, p) in self.lobby.players.iter().enumerate().skip(first).take(VISIBLE_ROWS) {
            let y = LIST_TOP + (i - first) as f32 * ROW_H;
            if i == self.selected {
                draw_rectangle(40.0, y, 360.0, ROW_H, Color::new(0.75, 0.85, 1.0, 1.0));
            }
            draw_circle(54.0, y + ROW_H / 2.0, 6.0, status_color(p));
            if p.code().len() == 2 {
                draw_circle_lines(54.0, y + ROW_H / 2.0, 8.0, 2.0, Color::new(0.8, 0.15, 0.1, 1.0));
            }
            text(&clip(&p.name, 22), 70.0, y + 16.0, 18.0, INK);
            let (w, l, d, _) = p.record();
            text(&format!("{w}/{l}/{d}"), 330.0, y + 16.0, 18.0, INK);
        }
        if self.lobby.players.len() > VISIBLE_ROWS {
            text(&format!("{} Spieler", self.lobby.players.len()), 48.0, LIST_TOP + ROW_H * VISIBLE_ROWS as f32 + 16.0, 14.0, INK);
        }
        // selected player
        draw_rectangle_lines(410.0, 120.0, 190.0, 220.0, 2.0, INK);
        if let Some(p) = self.lobby.players.get(self.selected) {
            let (w, l, d, pct) = p.record();
            text(&clip(&p.name, 16), 420.0, 146.0, 20.0, INK);
            for (i, s) in [format!("Gesamt: {}", w + l + d), format!("Gewonnen: {w}"), format!("Verloren: {l}"), format!("Unentschieden: {d}"), format!("Quote: {pct}%")].iter().enumerate() {
                text(s, 420.0, 176.0 + i as f32 * 22.0, 16.0, INK);
            }
            text(p.action_text(), 420.0, 310.0, 13.0, INK);
            if p.me {
                text("(Sie)", 420.0, 326.0, 13.0, INK);
            }
        }
        // chat
        draw_rectangle(40.0, 372.0, 560.0, 170.0, WHITE);
        draw_rectangle_lines(40.0, 372.0, 560.0, 170.0, 2.0, INK);
        let start = self.lobby.chat.len().saturating_sub(CHAT_LINES);
        for (i, (n, m)) in self.lobby.chat[start..].iter().enumerate() {
            text(&clip(&format!("{n}: {m}"), 64), 48.0, 392.0 + i as f32 * 23.0, 17.0, INK);
        }
        draw_rectangle(40.0, 550.0, 560.0, 28.0, WHITE);
        draw_rectangle_lines(40.0, 550.0, 560.0, 28.0, 2.0, INK);
        let cursor = if (get_time() * 2.0) as i64 % 2 == 0 { "|" } else { "" };
        let shown: String = self.chat_input.chars().rev().take(60).collect::<Vec<_>>().into_iter().rev().collect();
        text(&format!("{shown}{cursor}"), 48.0, 570.0, 18.0, INK);
        text("Enter: Aktion / senden   Hoch/Runter: waehlen   F2: alle herausfordern   Esc: trennen", 40.0, 598.0, 14.0, INK);
        if !self.status.is_empty() {
            text(&clip(&self.status, 60), 48.0, 118.0, 16.0, Color::new(0.75, 0.1, 0.05, 1.0));
        }
    }

    // ---- match ---------------------------------------------------------------------------------------------------

    fn frame_match(&mut self, a: &Assets) -> bool {
        let mut out: Vec<Msg> = Vec::new();
        let mut finished = None;
        let esc = is_key_pressed(KeyCode::Escape) && !self.surrendered;
        if esc {
            self.surrendered = true;
            out.push(Msg::Surrender(None));
        }
        if let Stage::Match(nm) = &mut self.stage {
            if nm.accepts_input() {
                let keys = [KeyCode::Left, KeyCode::Up, KeyCode::Right, KeyCode::Down];
                apply_keys(nm.local_player_mut(), keys, KeyCode::Space);
            }
            while self.acc >= STEP {
                self.acc -= STEP;
                nm.tick(wall_ms());
                out.extend(nm.take_outbox());
                for e in nm.world.events.drain(..) {
                    a.play(match e {
                        Event::BombPlaced => &a.sounds.donk,
                        Event::Explosion => &a.sounds.explode,
                        Event::PickUp => &a.sounds.pling,
                        Event::Kick => &a.sounds.landing,
                    });
                }
            }
            if nm.phase == NetPhase::Finished {
                finished = nm.outcome;
            }
            Self::draw_match(a, nm, &self.lobby);
        }
        self.send_all(out);
        if let Some(o) = finished {
            self.stage = Stage::After(o);
            self.acc = 0.0;
        }
        true
    }

    fn draw_match(a: &Assets, nm: &NetMatch, lobby: &Lobby) {
        if !nm.ready {
            draw_texture(&a.bg_game, 0.0, 0.0, WHITE);
            text_centered("Warte auf Gegner...", 360.0, 250.0, 34.0, INK);
            return;
        }
        let tag = |i: usize| format!("{}{}", clip(&nm.names[i], 14), if i == nm.me { " (du)" } else { "" });
        let (n0, n1) = (tag(0), tag(1));
        draw_game(a, &nm.world, [&n0, &n1], "Esc: Aufgeben");
        for i in 0..2 {
            let y0 = if i == 0 { 48.0 } else { 213.0 };
            let (ping, fps) = if i == nm.me { (nm.my_ping, nm.my_fps) } else { (nm.remote_ping, nm.remote_fps) };
            text(&format!("{ping}ms {fps}fps"), 12.0, y0 + 106.0, 14.0, INK);
        }
        let start = lobby.chat.len().saturating_sub(2);
        for (i, (n, m)) in lobby.chat[start..].iter().enumerate() {
            text(&clip(&format!("{n}: {m}"), 56), 110.0, 490.0 + i as f32 * 20.0, 16.0, INK);
        }
    }

    // ---- result --------------------------------------------------------------------------------------------------

    fn frame_after(&mut self, a: &Assets) -> bool {
        let Stage::After(outcome) = self.stage else { return true };
        let opp = self.lobby.opponent.clone();
        let opp_here = opp.as_ref().is_some_and(|o| self.lobby.find(o).is_some());
        if (is_key_pressed(KeyCode::Enter) || clicked(button_rect(&a.play_again, AGAIN_POS.0, AGAIN_POS.1))) && opp_here {
            let r = self.lobby.play_again();
            self.react(a, r);
        } else if is_key_pressed(KeyCode::Escape) || clicked(button_rect(&a.to_menu, MENU_POS.0, MENU_POS.1)) {
            let m = self.lobby.leave_match();
            self.send_all(vec![m]);
            self.stage = Stage::Room;
            return true;
        }
        if !matches!(self.stage, Stage::After(_)) {
            return true;
        }
        draw_texture(&a.bg_after, 0.0, 0.0, WHITE);
        panel(60.0, 40.0, 520.0, 440.0);
        text_centered("Ergebnis", 320.0, 95.0, 40.0, INK);
        let who = opp.as_deref().unwrap_or("");
        text_centered(&format!("{} gegen {}", clip(&self.lobby.my_name, 16), clip(who, 16)), 320.0, 125.0, 22.0, INK);
        let msg = match outcome {
            NetOutcome::Win => "Gewonnen!!!",
            NetOutcome::Lose => "Leider verloren...",
            NetOutcome::Draw => "Untenschieden!",
        };
        text_centered(msg, 320.0, 210.0, 54.0, INK);
        let note = if !opp_here {
            format!("{who} hat den Raum verlassen.")
        } else if self.lobby.rematch_requested {
            format!("{who} moechte nochmal spielen!")
        } else {
            "und jetzt? Entscheiden Sie...".to_string()
        };
        text_centered(&note, 320.0, 330.0, 26.0, INK);
        a.play_again.draw(0, AGAIN_POS.0, AGAIN_POS.1);
        let r = button_rect(&a.to_menu, MENU_POS.0, MENU_POS.1);
        draw_rectangle(r.x, r.y, r.w, r.h, Color::new(0.75, 0.1, 0.05, 1.0));
        draw_rectangle_lines(r.x, r.y, r.w, r.h, 2.0, INK);
        text_centered("ZUM RAUM", r.x + r.w / 2.0, r.y + r.h - 8.0, 22.0, WHITE);
        text("Enter: Nochmal Spielen", 100.0, 468.0, 18.0, INK);
        text("Esc: Zurueck zum Raum", 380.0, 468.0, 18.0, INK);
        true
    }
}
