//! Player room state of the client (`Connector.players`, `PlayerRoom`, the challenge handling of `onSockXML`).
//!
//! A player's status is kept as the original string codes: `0` free, `1` challenged me, `2` challenged by me,
//! `3` in a match, `13` / `23` combinations, `5` myself.

use crate::proto::{Msg, PlayerInfo};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    /// `won/lost/drawn`
    pub skill: String,
    pub req_me: bool,
    pub challenged: bool,
    pub in_game: bool,
    pub me: bool,
}

impl Entry {
    /// `Player.pStatus`
    pub fn code(&self) -> &'static str {
        match (self.me, self.req_me, self.challenged, self.in_game) {
            (true, ..) => "5",
            (_, true, _, true) => "13",
            (_, true, ..) => "1",
            (_, _, true, true) => "23",
            (_, _, true, _) => "2",
            (_, _, _, true) => "3",
            _ => "0",
        }
    }

    /// `Player.update`: parsed `won/lost/drawn` and the win percentage.
    pub fn record(&self) -> (u32, u32, u32, u32) {
        let mut it = self.skill.split('/').map(|s| s.parse::<u32>().unwrap_or(0));
        let (w, l, d) = (it.next().unwrap_or(0), it.next().unwrap_or(0), it.next().unwrap_or(0));
        let total = w + l + d;
        let pct = if total == 0 { 0 } else { (w as f64 / total as f64 * 100.0).round() as u32 };
        (w, l, d, pct)
    }

    /// Hint text of the room (`PlayerRoom.itemRollOver`).
    pub fn action_text(&self) -> &'static str {
        match self.code() {
            "0" | "3" => "Diesen Spieler herausfordern",
            "1" => "Herausforderung annehmen",
            "2" | "23" => "Herausforderung zurücknehmen",
            "13" => "Keine Aktion möglich",
            _ => "Das sind Sie...",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LobbyEvent {
    /// A match starts. `me` is the player number (1 = host of the seed, 2 = guest).
    StartMatch { opponent: String, number: u8 },
    /// `errorMsg`: the server refuses or drops us.
    Error(String),
    /// The opponent asked for a rematch.
    RematchRequested,
}

#[derive(Clone, Debug)]
pub struct Lobby {
    pub my_name: String,
    pub players: Vec<Entry>,
    pub chat: Vec<(String, String)>,
    pub challenged_all: bool,
    pub opponent: Option<String>,
    /// `Connector.gameStarted`
    pub in_match: bool,
    /// `Connector.playAgainReq`
    pub rematch_requested: bool,
}

/// What the UI has to do after a message: messages to send and an optional event.
#[derive(Default, Debug, PartialEq)]
pub struct Reaction {
    pub send: Vec<Msg>,
    pub event: Option<LobbyEvent>,
}

const MAX_CHAT_LINES: usize = 200;

impl Lobby {
    pub fn new(my_name: &str) -> Self {
        Self {
            my_name: my_name.to_string(),
            players: Vec::new(),
            chat: Vec::new(),
            challenged_all: false,
            opponent: None,
            in_match: false,
            rematch_requested: false,
        }
    }

    pub fn find(&self, name: &str) -> Option<usize> {
        self.players.iter().position(|p| p.name == name)
    }

    /// `Connector.addPlayer`
    fn add(&mut self, info: &PlayerInfo, force_challenged: bool) {
        let me = info.name == self.my_name;
        self.players.push(Entry {
            name: info.name.clone(),
            skill: info.skill.clone(),
            req_me: false,
            challenged: force_challenged && !me,
            in_game: info.state == "3" && !me,
            me,
        });
    }

    /// `PlayerRoom.sortPlayerList`: case-insensitive, guests ("Gast…") last.
    pub fn sort(&mut self) {
        self.players.sort_by_key(|p| (p.name.starts_with("Gast"), p.name.to_uppercase()));
    }

    /// `Connector.updatePlayer2`: set the status code directly.
    fn set_code(&mut self, name: &str, code: &str) {
        if let Some(i) = self.find(name) {
            let p = &mut self.players[i];
            p.req_me = code.starts_with('1');
            p.challenged = code.starts_with('2');
            p.in_game = code.ends_with('3');
        }
    }

    pub fn handle(&mut self, m: &Msg) -> Reaction {
        let mut r = Reaction::default();
        match m {
            Msg::UserList(list) => {
                self.players.clear();
                for i in list {
                    self.add(i, false);
                }
                self.sort();
            }
            Msg::NewPlayer(i) => {
                self.add(i, self.challenged_all);
                self.sort();
            }
            Msg::PlayerUpdate(i) => {
                if let Some(idx) = self.find(&i.name) {
                    let all = self.challenged_all;
                    let p = &mut self.players[idx];
                    if !p.me {
                        if all && !p.req_me {
                            p.challenged = true;
                        }
                        p.in_game = i.state == "3";
                    }
                    p.skill = i.skill.clone();
                }
            }
            Msg::PlayerLeft(n) => self.players.retain(|p| &p.name != n),
            Msg::Request(n) => {
                if let Some(i) = self.find(n) {
                    let p = &mut self.players[i];
                    p.req_me = true;
                    p.challenged = false;
                    p.in_game = false;
                }
            }
            Msg::RemRequest(n) => {
                if let Some(i) = self.find(n) {
                    let p = &mut self.players[i];
                    p.req_me = false;
                    p.challenged = false;
                }
            }
            Msg::StartGame(n) => {
                self.opponent = Some(n.clone());
                self.set_code(n, "3");
                if self.challenged_all {
                    r.send.push(Msg::RemChallengeAll);
                    for p in &mut self.players {
                        if p.challenged {
                            p.challenged = false;
                        }
                    }
                }
                self.in_match = true;
                self.rematch_requested = false;
                self.challenged_all = false;
                r.event = Some(LobbyEvent::StartMatch { opponent: n.clone(), number: 2 });
            }
            Msg::PlayAgain => {
                self.rematch_requested = true;
                r.event = Some(LobbyEvent::RematchRequested);
            }
            Msg::MsgAll { name, msg } => {
                if !self.in_match {
                    self.push_chat(name, msg);
                }
            }
            Msg::MsgPlayer { name, msg } => {
                if self.in_match {
                    self.push_chat(name, msg);
                }
            }
            Msg::ErrorMsg(t) => r.event = Some(LobbyEvent::Error(t.clone())),
            _ => {}
        }
        r
    }

    fn push_chat(&mut self, name: &str, msg: &str) {
        self.chat.push((name.to_string(), msg.to_string()));
        if self.chat.len() > MAX_CHAT_LINES {
            self.chat.remove(0);
        }
    }

    /// `PlayerRoom.change`: a click on a player. Returns the messages to send and, when accepting, the match start.
    pub fn click(&mut self, name: &str) -> Reaction {
        let mut r = Reaction::default();
        let Some(i) = self.find(name) else { return r };
        let code = self.players[i].code();
        match code {
            "0" | "3" => {
                self.set_code(name, if code == "0" { "2" } else { "23" });
                r.send.push(Msg::Challenge(name.to_string()));
            }
            "1" => r = self.start_match_as_host(name),
            "2" => {
                self.set_code(name, "0");
                r.send.push(Msg::RemChallenge(name.to_string()));
            }
            "13" | "23" => {
                self.set_code(name, "3");
                r.send.push(Msg::RemChallenge(name.to_string()));
            }
            _ => {}
        }
        r
    }

    /// `Connector.sendStartGame`: accept the challenge of `name`; we become player 1.
    pub fn start_match_as_host(&mut self, name: &str) -> Reaction {
        self.opponent = Some(name.to_string());
        self.set_code(name, "3");
        self.in_match = true;
        self.rematch_requested = false;
        self.challenged_all = false;
        Reaction {
            send: vec![Msg::StartGame(name.to_string())],
            event: Some(LobbyEvent::StartMatch { opponent: name.to_string(), number: 1 }),
        }
    }

    /// `ChallengeAllButton`: challenge everybody who is free, or take it back.
    pub fn toggle_challenge_all(&mut self) -> Vec<Msg> {
        if self.challenged_all {
            self.challenged_all = false;
            for p in &mut self.players {
                p.challenged = false;
            }
            vec![Msg::RemChallengeAll]
        } else {
            self.challenged_all = true;
            for p in &mut self.players {
                if !p.me && !p.req_me {
                    p.challenged = true;
                }
            }
            vec![Msg::ChallengeAll]
        }
    }

    /// Rematch button of the after-match screen: start right away if the opponent already asked, else ask.
    pub fn play_again(&mut self) -> Reaction {
        match (&self.opponent.clone(), self.rematch_requested) {
            (Some(o), true) => self.start_match_as_host(o),
            _ => Reaction { send: vec![Msg::PlayAgain], event: None },
        }
    }

    /// Back to the room (`toRoom`).
    pub fn leave_match(&mut self) -> Msg {
        self.in_match = false;
        self.opponent = None;
        self.rematch_requested = false;
        Msg::ToRoom
    }

    /// `Connector.sendChatMsg`: cleaned text and the message to use for the current context.
    pub fn chat_message(&self, text: &str) -> Option<Msg> {
        let msg = crate::proto::clean_chat(text);
        if msg.is_empty() {
            return None;
        }
        let name = self.my_name.clone();
        Some(if self.in_match { Msg::MsgPlayer { name, msg } } else { Msg::MsgAll { name, msg } })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(name: &str, state: &str) -> PlayerInfo {
        PlayerInfo { name: name.into(), skill: "2/1/1".into(), state: state.into() }
    }

    fn lobby() -> Lobby {
        let mut l = Lobby::new("Me");
        l.handle(&Msg::UserList(vec![info("Zed", "0"), info("Me", "0"), info("Gast42", "0"), info("anna", "3")]));
        l
    }

    fn code(l: &Lobby, n: &str) -> &'static str {
        l.players[l.find(n).unwrap()].code()
    }

    #[test]
    fn user_list_marks_myself_sorts_and_puts_guests_last() {
        let l = lobby();
        let names: Vec<_> = l.players.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["anna", "Me", "Zed", "Gast42"]);
        assert_eq!(code(&l, "Me"), "5");
        assert_eq!(code(&l, "anna"), "3");
        assert_eq!(code(&l, "Zed"), "0");
    }

    #[test]
    fn skill_is_parsed_into_a_record() {
        let l = lobby();
        assert_eq!(l.players[0].record(), (2, 1, 1, 50));
        let e = Entry { skill: "x".into(), ..l.players[0].clone() };
        assert_eq!(e.record(), (0, 0, 0, 0));
    }

    #[test]
    fn click_cycle_matches_the_original_room() {
        let mut l = lobby();
        let r = l.click("Zed");
        assert_eq!(r.send, vec![Msg::Challenge("Zed".into())]);
        assert_eq!(code(&l, "Zed"), "2");
        let r = l.click("Zed");
        assert_eq!(r.send, vec![Msg::RemChallenge("Zed".into())]);
        assert_eq!(code(&l, "Zed"), "0");
        // challenging a player in a match shows "23"; clicking again withdraws and keeps him "3"
        l.click("anna");
        assert_eq!(code(&l, "anna"), "23");
        l.click("anna");
        assert_eq!(code(&l, "anna"), "3");
        assert!(l.click("Me").send.is_empty());
        assert!(l.click("nobody").send.is_empty());
    }

    #[test]
    fn request_then_click_accepts_and_makes_me_player_one() {
        let mut l = lobby();
        l.handle(&Msg::Request("Zed".into()));
        assert_eq!(code(&l, "Zed"), "1");
        let r = l.click("Zed");
        assert_eq!(r.send, vec![Msg::StartGame("Zed".into())]);
        assert_eq!(r.event, Some(LobbyEvent::StartMatch { opponent: "Zed".into(), number: 1 }));
        assert!(l.in_match);
        assert_eq!(code(&l, "Zed"), "3");
    }

    #[test]
    fn request_on_a_player_in_a_match_and_withdrawals() {
        let mut l = lobby();
        l.handle(&Msg::Request("anna".into()));
        assert_eq!(code(&l, "anna"), "1", "updatePlayer2 sets the raw status like the original");
        l.handle(&Msg::PlayerUpdate(info("anna", "3")));
        assert_eq!(code(&l, "anna"), "13");
        let r = l.click("anna");
        assert_eq!(r.send, vec![Msg::RemChallenge("anna".into())]);
        assert_eq!(code(&l, "anna"), "3");
        l.handle(&Msg::Request("Zed".into()));
        l.handle(&Msg::RemRequest("Zed".into()));
        assert_eq!(code(&l, "Zed"), "0");
        l.handle(&Msg::Request("anna".into()));
        l.handle(&Msg::PlayerUpdate(info("anna", "3")));
        l.handle(&Msg::RemRequest("anna".into()));
        assert_eq!(code(&l, "anna"), "3");
    }

    #[test]
    fn player_update_keeps_challenge_flags_and_updates_skill() {
        let mut l = lobby();
        l.click("Zed");
        l.handle(&Msg::PlayerUpdate(PlayerInfo { name: "Zed".into(), skill: "5/0/0".into(), state: "3".into() }));
        assert_eq!(code(&l, "Zed"), "23");
        assert_eq!(l.players[l.find("Zed").unwrap()].skill, "5/0/0");
        l.handle(&Msg::PlayerUpdate(info("Zed", "0")));
        assert_eq!(code(&l, "Zed"), "2");
    }

    #[test]
    fn challenge_all_marks_everybody_and_newcomers_and_is_cleared_when_a_match_starts() {
        let mut l = lobby();
        assert_eq!(l.toggle_challenge_all(), vec![Msg::ChallengeAll]);
        assert_eq!(code(&l, "Zed"), "2");
        assert_eq!(code(&l, "anna"), "23");
        assert_eq!(code(&l, "Me"), "5");
        l.handle(&Msg::NewPlayer(info("New", "0")));
        assert_eq!(code(&l, "New"), "2");
        let r = l.handle(&Msg::StartGame("Zed".into()));
        assert_eq!(r.send, vec![Msg::RemChallengeAll]);
        assert_eq!(r.event, Some(LobbyEvent::StartMatch { opponent: "Zed".into(), number: 2 }));
        assert!(!l.challenged_all);
        assert_eq!(code(&l, "Zed"), "3");
        assert_eq!(code(&l, "New"), "0");
        assert_eq!(code(&l, "anna"), "3");

        let mut l = lobby();
        l.toggle_challenge_all();
        assert_eq!(l.toggle_challenge_all(), vec![Msg::RemChallengeAll]);
        assert_eq!(code(&l, "Zed"), "0");
    }

    #[test]
    fn chat_goes_to_the_room_or_to_the_opponent_depending_on_the_match() {
        let mut l = lobby();
        assert_eq!(l.chat_message("  hi <x>  "), Some(Msg::MsgAll { name: "Me".into(), msg: "hi (x)".into() }));
        assert_eq!(l.chat_message("   "), None);
        l.handle(&Msg::MsgAll { name: "Zed".into(), msg: "yo".into() });
        l.handle(&Msg::MsgPlayer { name: "Zed".into(), msg: "ignored outside a match".into() });
        assert_eq!(l.chat, vec![("Zed".to_string(), "yo".to_string())]);
        l.handle(&Msg::StartGame("Zed".into()));
        assert_eq!(l.chat_message("gl"), Some(Msg::MsgPlayer { name: "Me".into(), msg: "gl".into() }));
        l.handle(&Msg::MsgAll { name: "Zed".into(), msg: "room chat hidden".into() });
        l.handle(&Msg::MsgPlayer { name: "Zed".into(), msg: "gl hf".into() });
        assert_eq!(l.chat.len(), 2);
    }

    #[test]
    fn rematch_flow() {
        let mut l = lobby();
        l.handle(&Msg::StartGame("Zed".into()));
        // first ask ...
        assert_eq!(l.play_again().send, vec![Msg::PlayAgain]);
        // ... the opponent asked first: start immediately as player 1
        l.handle(&Msg::PlayAgain);
        assert!(l.rematch_requested);
        let r = l.play_again();
        assert_eq!(r.send, vec![Msg::StartGame("Zed".into())]);
        assert_eq!(r.event, Some(LobbyEvent::StartMatch { opponent: "Zed".into(), number: 1 }));
        assert_eq!(l.leave_match(), Msg::ToRoom);
        assert!(!l.in_match && l.opponent.is_none());
    }

    #[test]
    fn leaving_players_disappear_and_errors_are_reported() {
        let mut l = lobby();
        l.handle(&Msg::PlayerLeft("Zed".into()));
        assert!(l.find("Zed").is_none());
        let r = l.handle(&Msg::ErrorMsg("Name vergeben".into()));
        assert_eq!(r.event, Some(LobbyEvent::Error("Name vergeben".into())));
    }
}
