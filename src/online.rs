//! Online screens: connect form, player room, running match and result (the original's `Connector`, `PlayerRoom`,
//! `Game` and `AfterMatch` frames).

use crate::assets::Assets;
use crate::ui::{self, clip, drain_chars, panel, text, text_centered, Field, Style, ERROR, INK};
use crate::{apply_keys, draw_game, game_bar, solo_hint, P1_BOMB, P1_DIRS, P2_BOMB, P2_DIRS, STEP};
use bomberpengu::i18n::{server_text, tr, tr_with, Key};
use bomberpengu::lobby::{Entry, Lobby, LobbyEvent, Reaction};
use bomberpengu::net::{Connection, NetEvent};
use bomberpengu::netgame::{NetMatch, NetOutcome, NetPhase};
use bomberpengu::proto::{valid_name, Msg};
use bomberpengu::world::Event;
use macroquad::prelude::*;

const ROW_H: f32 = 22.0;
const LIST_TOP: f32 = 142.0;
const VISIBLE_ROWS: usize = 9;
const CHAT_LINES: usize = 7;
const MAX_ADDR_CHARS: usize = 64;
const MAX_CHAT_INPUT: usize = 200;

const NAME_FIELD: Rect = Rect { x: 90.0, y: 240.0, w: 460.0, h: 38.0 };
const SERVER_FIELD: Rect = Rect { x: 90.0, y: 310.0, w: 460.0, h: 38.0 };
const CHAT_FIELD: Rect = Rect { x: 40.0, y: 532.0, w: 560.0, h: 28.0 };
const CONNECT_BTN: Rect = Rect { x: 90.0, y: 372.0, w: 220.0, h: 46.0 };
const BACK_BTN: Rect = Rect { x: 330.0, y: 372.0, w: 220.0, h: 46.0 };
const CANCEL_BTN: Rect = Rect { x: 220.0, y: 380.0, w: 200.0, h: 46.0 };
const ACT_BTN: Rect = Rect { x: 40.0, y: 570.0, w: 180.0, h: 36.0 };
const ALL_BTN: Rect = Rect { x: 230.0, y: 570.0, w: 190.0, h: 36.0 };
const LEAVE_BTN: Rect = Rect { x: 430.0, y: 570.0, w: 170.0, h: 36.0 };

enum Stage {
    Form,
    Connecting,
    Room,
    Match(Box<NetMatch>),
    After(NetOutcome),
}

pub struct Online {
    stage: Stage,
    name: Field,
    server: Field,
    edit_server: bool,
    status: String,
    conn: Option<Connection>,
    lobby: Lobby,
    chat_input: Field,
    selected: usize,
    acc: f64,
    surrendered: bool,
    after_since: f64,
    /// Test aids for scripted runs: challenge this player / accept every challenge automatically.
    pub auto_challenge: Option<String>,
    pub auto_accept: bool,
}

fn wall_ms() -> i64 {
    (get_time() * 1000.0) as i64
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
            name: crate::name_field(name),
            server: Field::new(server, MAX_ADDR_CHARS, ui::any_char),
            edit_server: false,
            status: String::new(),
            conn: None,
            lobby: Lobby::new(name),
            chat_input: Field::new("", MAX_CHAT_INPUT, ui::any_char),
            selected: 0,
            acc: 0.0,
            surrendered: false,
            after_since: 0.0,
            auto_challenge: None,
            auto_accept: false,
        }
    }

    pub fn name(&self) -> &str {
        &self.name.text
    }

    pub fn connect(&mut self) {
        let name = self.name.text.trim().to_string();
        if !valid_name(&name) {
            self.status = tr(Key::InvalidName).into();
            return;
        }
        self.lobby = Lobby::new(&name);
        self.conn = Some(Connection::connect(&self.server.text, &name));
        self.status = tr(Key::Connecting).into();
        self.stage = Stage::Connecting;
    }

    fn disconnect(&mut self, why: &str) {
        self.conn = None;
        if self.status.is_empty() || self.status == tr(Key::Connecting) {
            self.status = server_text(why);
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
            Some(LobbyEvent::Error(t)) => self.status = server_text(&t),
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
                    if let (Stage::After(outcome), Msg::EndGame { winner }) = (&mut self.stage, &m) {
                        *outcome = if winner.is_empty() { NetOutcome::Draw }
                            else if winner == &self.lobby.my_name { NetOutcome::Win }
                            else { NetOutcome::Lose };
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
        if ui::clicked(NAME_FIELD) {
            self.edit_server = false;
        } else if ui::clicked(SERVER_FIELD) {
            self.edit_server = true;
        }
        if self.edit_server {
            self.server.update(SERVER_FIELD);
        } else {
            self.name.update(NAME_FIELD);
        }
        draw_rectangle(0.0, 0.0, 640.0, 640.0, Color::new(0.75, 0.88, 1.0, 1.0));
        panel(60.0, 120.0, 520.0, 380.0);
        text_centered(tr(Key::OnlineLobby), 320.0, 180.0, 44.0, INK);
        text(tr(Key::Name), 90.0, 230.0, 20.0, INK);
        ui::draw_field(NAME_FIELD, &self.name, !self.edit_server);
        text(tr(Key::ServerAddr), 90.0, 300.0, 20.0, INK);
        ui::draw_field(SERVER_FIELD, &self.server, self.edit_server);
        let connect = ui::button(CONNECT_BTN, tr(Key::Connect), Style::Primary, true);
        let back = ui::button(BACK_BTN, tr(Key::Back), Style::Normal, true);
        text_centered(tr(Key::FormHint), 320.0, 446.0, 16.0, INK);
        if !self.status.is_empty() {
            ui::text_fit(&self.status, 90.0, 478.0, 460.0, 18.0, ERROR);
        }
        if back {
            return false;
        }
        if connect || is_key_pressed(KeyCode::Enter) {
            self.connect();
        }
        true
    }

    fn frame_connecting(&mut self) -> bool {
        draw_rectangle(0.0, 0.0, 640.0, 640.0, Color::new(0.75, 0.88, 1.0, 1.0));
        text_centered(&tr_with(Key::ConnectingTo, &[&clip(&self.server.text, 40)]), 320.0, 320.0, 28.0, INK);
        if ui::button(CANCEL_BTN, tr(Key::Cancel), Style::Normal, true) || is_key_pressed(KeyCode::Escape) {
            self.conn = None;
            self.status.clear();
            self.stage = Stage::Form;
        }
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

    /// Label and availability of the room's main button for the selected player.
    fn action_button(&self) -> (&'static str, bool) {
        match self.lobby.players.get(self.selected) {
            Some(p) if !p.me => match p.code() {
                "0" | "3" => (tr(Key::Challenge), true),
                "1" => (tr(Key::Accept), true),
                "2" | "23" => (tr(Key::Withdraw), true),
                _ => (tr(Key::Challenge), false),
            },
            _ => (tr(Key::Challenge), false),
        }
    }

    fn frame_room(&mut self, a: &Assets) -> bool {
        if is_key_pressed(KeyCode::Escape) || ui::clicked(LEAVE_BTN) {
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
        if is_key_pressed(KeyCode::F2) || ui::clicked(ALL_BTN) {
            let m = self.lobby.toggle_challenge_all();
            self.send_all(m);
        }
        if ui::clicked(ACT_BTN) && self.action_button().1 {
            self.act_on_selected(a);
        }
        self.chat_input.update(CHAT_FIELD);
        if is_key_pressed(KeyCode::Enter) {
            if self.chat_input.text.trim().is_empty() {
                self.act_on_selected(a);
            } else if let Some(m) = self.lobby.chat_message(&self.chat_input.text) {
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
        text_centered(tr(Key::Room), 320.0, 76.0, 38.0, INK);
        text(&tr_with(Key::RoomOn, &[&self.lobby.my_name, &clip(&self.server.text, 40)]), 48.0, 100.0, 16.0, INK);
        text(tr(Key::ColPlayer), 66.0, 134.0, 16.0, INK);
        text(tr(Key::ColRecord), 330.0, 134.0, 16.0, INK);
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
            text(&tr_with(Key::PlayerCount, &[&self.lobby.players.len()]), 48.0, LIST_TOP + ROW_H * VISIBLE_ROWS as f32 + 16.0, 14.0, INK);
        }
        // selected player
        draw_rectangle_lines(410.0, 120.0, 190.0, 220.0, 2.0, INK);
        if let Some(p) = self.lobby.players.get(self.selected) {
            let (w, l, d, pct) = p.record();
            text(&clip(&p.name, 16), 420.0, 146.0, 20.0, INK);
            let stats = [
                tr_with(Key::StatTotal, &[&(w + l + d)]),
                tr_with(Key::StatWon, &[&w]),
                tr_with(Key::StatLost, &[&l]),
                tr_with(Key::StatDrawn, &[&d]),
                tr_with(Key::StatRate, &[&pct]),
            ];
            for (i, s) in stats.iter().enumerate() {
                ui::text_fit(s, 420.0, 176.0 + i as f32 * 22.0, 176.0, 16.0, INK);
            }
            ui::text_fit(p.action_text(), 420.0, 310.0, 176.0, 13.0, INK);
            if p.me {
                text(tr(Key::You), 420.0, 326.0, 13.0, INK);
            }
        }
        // chat
        draw_rectangle(40.0, 362.0, 560.0, 162.0, WHITE);
        draw_rectangle_lines(40.0, 362.0, 560.0, 162.0, 2.0, INK);
        let start = self.lobby.chat.len().saturating_sub(CHAT_LINES);
        for (i, (n, m)) in self.lobby.chat[start..].iter().enumerate() {
            text(&clip(&format!("{n}: {m}"), 64), 48.0, 381.0 + i as f32 * 23.0, 17.0, INK);
        }
        ui::draw_field(CHAT_FIELD, &self.chat_input, true);
        let (label, enabled) = self.action_button();
        ui::draw_button(ACT_BTN, label, Style::Primary, enabled, false);
        ui::draw_button(ALL_BTN, tr(if self.lobby.challenged_all { Key::WithdrawAll } else { Key::ChallengeAll }), Style::Normal, true, false);
        ui::draw_button(LEAVE_BTN, tr(Key::Disconnect), Style::Normal, true, false);
        if !self.status.is_empty() {
            ui::text_fit(&self.status, 48.0, 118.0, 540.0, 16.0, ERROR);
        }
    }

    // ---- match ---------------------------------------------------------------------------------------------------

    fn frame_match(&mut self, a: &Assets) -> bool {
        let mut out: Vec<Msg> = Vec::new();
        let mut finished = None;
        let mut surrender = is_key_pressed(KeyCode::Escape);
        if let Stage::Match(nm) = &mut self.stage {
            if nm.accepts_input() {
                apply_keys(nm.local_player_mut(), &[P1_DIRS, P2_DIRS], &[P1_BOMB, P2_BOMB]);
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
            surrender |= Self::draw_match(a, nm, &self.lobby);
        }
        if surrender && !self.surrendered {
            self.surrendered = true;
            out.push(Msg::Surrender(None));
        }
        self.send_all(out);
        if let Some(o) = finished {
            self.stage = Stage::After(o);
            self.after_since = get_time();
            self.acc = 0.0;
        }
        true
    }

    /// Draws the match; `true` = the surrender button was clicked.
    fn draw_match(a: &Assets, nm: &NetMatch, lobby: &Lobby) -> bool {
        if !nm.ready {
            draw_texture(&a.bg_game, 0.0, 0.0, WHITE);
            text_centered(tr(Key::WaitingOpponent), 360.0, 250.0, 34.0, INK);
            return game_bar(a, solo_hint(), tr(Key::Surrender));
        }
        let tag = |i: usize| format!("{}{}", clip(&nm.names[i], 14), if i == nm.me { tr(Key::YouTag) } else { "" });
        let (n0, n1) = (tag(0), tag(1));
        let surrender = draw_game(a, &nm.world, [&n0, &n1], solo_hint(), tr(Key::Surrender));
        for i in 0..2 {
            let y0 = if i == 0 { 48.0 } else { 213.0 };
            let (ping, fps) = if i == nm.me { (nm.my_ping, nm.my_fps) } else { (nm.remote_ping, nm.remote_fps) };
            text(&format!("{ping}ms {fps}fps"), 12.0, y0 + 106.0, 14.0, INK);
        }
        let start = lobby.chat.len().saturating_sub(2);
        for (i, (n, m)) in lobby.chat[start..].iter().enumerate() {
            text(&clip(&format!("{n}: {m}"), 56), 110.0, 490.0 + i as f32 * 20.0, 16.0, INK);
        }
        surrender
    }

    // ---- result --------------------------------------------------------------------------------------------------

    fn frame_after(&mut self, a: &Assets) -> bool {
        let Stage::After(outcome) = self.stage else { return true };
        let opp = self.lobby.opponent.clone();
        let opp_here = opp.as_ref().is_some_and(|o| self.lobby.find(o).is_some());
        draw_texture(&a.bg_after, 0.0, 0.0, WHITE);
        panel(60.0, 40.0, 520.0, 440.0);
        text_centered(tr(Key::Result), 320.0, 95.0, 40.0, INK);
        let who = opp.as_deref().unwrap_or("");
        text_centered(&tr_with(Key::VersusLine, &[&clip(&self.lobby.my_name, 16), &clip(who, 16)]), 320.0, 125.0, 22.0, INK);
        let msg = match outcome {
            NetOutcome::Win => Key::Won,
            NetOutcome::Lose => Key::Lost,
            NetOutcome::Draw => Key::Draw,
        };
        text_centered(tr(msg), 320.0, 210.0, 54.0, INK);
        let note = if !opp_here {
            tr_with(Key::OpponentLeft, &[&who])
        } else if self.lobby.rematch_requested {
            tr_with(Key::RematchAsked, &[&who])
        } else {
            tr(Key::WhatNow).to_string()
        };
        text_centered(&note, 320.0, 330.0, 26.0, INK);
        text_centered(tr(Key::AfterHintOnline), 320.0, 462.0, 16.0, INK);
        let (again, back) = ui::after_buttons(opp_here, tr(Key::ToRoom));
        // Enter doubles as a bomb key, so it is ignored for a moment after the match
        let enter = is_key_pressed(KeyCode::Enter) && get_time() - self.after_since > 0.7;
        if (again || enter) && opp_here {
            let r = self.lobby.play_again();
            self.react(a, r);
        } else if back || is_key_pressed(KeyCode::Escape) {
            let m = self.lobby.leave_match();
            self.send_all(vec![m]);
            self.stage = Stage::Room;
        }
        true
    }
}
