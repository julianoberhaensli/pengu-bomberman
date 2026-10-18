//! Wire messages of the original client/server protocol (`Connector.onSockXML` and the `send*` functions).
//!
//! Every message is one XML element followed by a zero byte (XMLSocket framing). In-game messages carry the
//! sender's position and key state (`x`, `y`, `c`) so that the other client can correct the remote penguin.

use crate::xml::Element;
use std::io::{self, BufRead};

/// `Connector.versionNr`
pub const VERSION: &str = "1.1.0.spika";
pub const MAX_FRAME: usize = 4096;

/// Position and key state (`[left, up, right, down]` as four `0`/`1` characters) attached to game messages.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cmd {
    pub x: i32,
    pub y: i32,
    pub c: String,
}

impl Cmd {
    pub fn new(x: f64, y: f64, keys: [bool; 4]) -> Self {
        Self { x: x.round() as i32, y: y.round() as i32, c: keys.iter().map(|&k| if k { '1' } else { '0' }).collect() }
    }

    pub fn keys(&self) -> [bool; 4] {
        let mut k = [false; 4];
        for (i, ch) in self.c.chars().take(4).enumerate() {
            k[i] = ch == '1';
        }
        k
    }
}

/// A line of the player list; `skill` is `won/lost/drawn`, `state` is `0` (free) or `3` (playing).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlayerInfo {
    pub name: String,
    pub skill: String,
    pub state: String,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Msg {
    // --- client -> server (lobby)
    Auth { name: String, version: String, hash: String },
    Beat,
    Challenge(String),
    RemChallenge(String),
    ChallengeAll,
    RemChallengeAll,
    /// client -> server: accept a challenge / start the rematch; server -> client: the opponent that started the game.
    StartGame(String),
    MsgAll { name: String, msg: String },
    MsgPlayer { name: String, msg: String },
    // --- client -> server (match)
    ToRoom,
    /// client -> server, relayed to the opponent: "I want a rematch"
    PlayAgain,
    WinGame,
    DrawGame,
    Ping,
    /// client -> server: give up; server -> clients: `winner` is the player who won by it.
    Surrender(Option<String>),
    // --- relayed to the opponent
    /// `12`: key state changed
    Move(Cmd),
    /// `14`: ping and frame rate of the sender
    Stats { ping: i64, fps: i64 },
    /// `16`: level seed chosen by player 1
    Seed(i64),
    /// `17`: bomb placed
    Bomb { cmd: Cmd, cx: i32, cy: i32 },
    /// `18`: item taken; the sender's resulting speed and blast range
    Pickup { cmd: Cmd, cx: i32, cy: i32, speed: f64, blast: u32 },
    /// `19`: bomb kicked; `t` = milliseconds until it explodes
    Kick { cmd: Cmd, cx: i32, cy: i32, id: i64, dx: f64, dy: f64, t: i64 },
    /// `11`: kicked bomb stopped
    Stop { cmd: Cmd, cx: i32, cy: i32, id: i64, t: i64 },
    /// `10`: kicked bomb exploded
    Detonate { cmd: Cmd, cx: i32, cy: i32, id: i64 },
    Die { x: i32, y: i32 },
    // --- server -> client
    UserList(Vec<PlayerInfo>),
    PlayerUpdate(PlayerInfo),
    NewPlayer(PlayerInfo),
    PlayerLeft(String),
    Request(String),
    RemRequest(String),
    Pong,
    /// Both players died at the same time.
    Draw,
    EndGame { winner: String },
    ErrorMsg(String),
}

fn num(e: &Element, key: &str) -> Option<f64> {
    e.get(key)?.trim().parse::<f64>().ok().filter(|v| v.is_finite())
}

fn int(e: &Element, key: &str) -> Option<i64> {
    num(e, key).map(|v| v.round() as i64)
}

fn string(e: &Element, key: &str) -> Option<String> {
    e.get(key).map(str::to_string)
}

fn cmd(e: &Element) -> Option<Cmd> {
    Some(Cmd { x: int(e, "x")? as i32, y: int(e, "y")? as i32, c: e.get("c")?.to_string() })
}

fn with_cmd(mut e: Element, c: &Cmd) -> Element {
    e = e.attr("c", &c.c).attr("x", c.x).attr("y", c.y);
    e
}

fn info(e: &Element) -> Option<PlayerInfo> {
    Some(PlayerInfo { name: string(e, "name")?, skill: string(e, "skill")?, state: string(e, "state")? })
}

fn info_el(name: &str, p: &PlayerInfo) -> Element {
    Element::new(name).attr("name", &p.name).attr("skill", &p.skill).attr("state", &p.state)
}

impl Msg {
    /// Messages the server forwards unchanged to the opponent while a match is running.
    pub fn is_relay(&self) -> bool {
        matches!(
            self,
            Msg::Move(_)
                | Msg::Stats { .. }
                | Msg::Seed(_)
                | Msg::Bomb { .. }
                | Msg::Pickup { .. }
                | Msg::Kick { .. }
                | Msg::Stop { .. }
                | Msg::Detonate { .. }
                | Msg::Die { .. }
        )
    }

    pub fn to_element(&self) -> Element {
        let el = Element::new;
        match self {
            Msg::Auth { name, version, hash } => el("auth").attr("name", name).attr("version", version).attr("hash", hash),
            Msg::Beat => el("beat"),
            Msg::Challenge(n) => el("challenge").attr("name", n).attr("hash", "xxxxxx"),
            Msg::RemChallenge(n) => el("remChallenge").attr("name", n).attr("hash", "xxxxxx"),
            Msg::ChallengeAll => el("challengeAll"),
            Msg::RemChallengeAll => el("remChallengeAll"),
            Msg::StartGame(n) => el("startGame").attr("name", n).attr("hash", "xxxxxx"),
            Msg::MsgAll { name, msg } => el("msgAll").attr("name", name).attr("msg", msg),
            Msg::MsgPlayer { name, msg } => el("msgPlayer").attr("name", name).attr("msg", msg),
            Msg::ToRoom => el("toRoom"),
            Msg::PlayAgain => el("playAgain"),
            Msg::WinGame => el("winGame"),
            Msg::DrawGame => el("drawGame"),
            Msg::Ping => el("ping"),
            Msg::Surrender(None) => el("surrender"),
            Msg::Surrender(Some(w)) => el("surrender").attr("winner", w),
            Msg::Move(c) => with_cmd(el("12"), c),
            Msg::Stats { ping, fps } => el("14").attr("p", ping).attr("f", fps),
            Msg::Seed(s) => el("16").attr("s", s),
            Msg::Bomb { cmd, cx, cy } => with_cmd(el("17"), cmd).attr("xp", cx).attr("yp", cy),
            Msg::Pickup { cmd, cx, cy, speed, blast } => {
                with_cmd(el("18"), cmd).attr("xp", cx).attr("yp", cy).attr("s", speed).attr("b", blast)
            }
            Msg::Kick { cmd, cx, cy, id, dx, dy, t } => with_cmd(el("19"), cmd)
                .attr("xp", cx)
                .attr("yp", cy)
                .attr("i", id)
                .attr("dx", dx)
                .attr("dy", dy)
                .attr("t", t),
            Msg::Stop { cmd, cx, cy, id, t } => {
                with_cmd(el("11"), cmd).attr("i", id).attr("xp", cx).attr("yp", cy).attr("t", t)
            }
            Msg::Detonate { cmd, cx, cy, id } => with_cmd(el("10"), cmd).attr("i", id).attr("xp", cx).attr("yp", cy),
            Msg::Die { x, y } => el("die").attr("x", x).attr("y", y),
            Msg::UserList(list) => list.iter().fold(el("userList"), |e, p| e.child(info_el("user", p))),
            Msg::PlayerUpdate(p) => info_el("playerUpdate", p),
            Msg::NewPlayer(p) => info_el("newPlayer", p),
            Msg::PlayerLeft(n) => el("playerLeft").attr("name", n),
            Msg::Request(n) => el("request").attr("name", n),
            Msg::RemRequest(n) => el("remRequest").attr("name", n),
            Msg::Pong => el("pong"),
            Msg::Draw => el("draw"),
            Msg::EndGame { winner } => el("endGame").attr("winner", winner),
            Msg::ErrorMsg(t) => el("errorMsg").text(t),
        }
    }

    pub fn to_xml(&self) -> String {
        self.to_element().to_xml()
    }

    /// Frame as sent on the wire: XML, newline, zero byte.
    pub fn to_frame(&self) -> Vec<u8> {
        let mut v = self.to_xml().into_bytes();
        v.extend_from_slice(b"\n\0");
        v
    }

    pub fn from_xml(src: &str) -> Option<Msg> {
        Msg::from_element(&Element::parse(src)?)
    }

    /// Parses a message; unknown names and missing attributes give `None` (the original prints "unknown command").
    pub fn from_element(e: &Element) -> Option<Msg> {
        Some(match e.name.as_str() {
            "auth" => Msg::Auth { name: string(e, "name")?, version: string(e, "version")?, hash: e.get("hash").unwrap_or("").to_string() },
            "beat" => Msg::Beat,
            "challenge" => Msg::Challenge(string(e, "name")?),
            "remChallenge" => Msg::RemChallenge(string(e, "name")?),
            "challengeAll" => Msg::ChallengeAll,
            "remChallengeAll" => Msg::RemChallengeAll,
            "startGame" => Msg::StartGame(string(e, "name")?),
            "msgAll" => Msg::MsgAll { name: string(e, "name")?, msg: string(e, "msg")? },
            "msgPlayer" => Msg::MsgPlayer { name: string(e, "name")?, msg: string(e, "msg")? },
            "toRoom" => Msg::ToRoom,
            "playAgain" => Msg::PlayAgain,
            "winGame" => Msg::WinGame,
            "drawGame" => Msg::DrawGame,
            "ping" => Msg::Ping,
            "surrender" => Msg::Surrender(string(e, "winner")),
            "12" => Msg::Move(cmd(e)?),
            "14" => Msg::Stats { ping: int(e, "p")?, fps: int(e, "f")? },
            "16" => Msg::Seed(int(e, "s")?),
            "17" => Msg::Bomb { cmd: cmd(e)?, cx: int(e, "xp")? as i32, cy: int(e, "yp")? as i32 },
            "18" => Msg::Pickup {
                cmd: cmd(e)?,
                cx: int(e, "xp")? as i32,
                cy: int(e, "yp")? as i32,
                speed: num(e, "s")?,
                blast: int(e, "b")?.clamp(0, 64) as u32,
            },
            "19" => Msg::Kick {
                cmd: cmd(e)?,
                cx: int(e, "xp")? as i32,
                cy: int(e, "yp")? as i32,
                id: int(e, "i")?,
                dx: num(e, "dx")?,
                dy: num(e, "dy")?,
                t: int(e, "t")?,
            },
            "11" => Msg::Stop { cmd: cmd(e)?, cx: int(e, "xp")? as i32, cy: int(e, "yp")? as i32, id: int(e, "i")?, t: int(e, "t")? },
            "10" => Msg::Detonate { cmd: cmd(e)?, cx: int(e, "xp")? as i32, cy: int(e, "yp")? as i32, id: int(e, "i")? },
            "die" => Msg::Die { x: int(e, "x")? as i32, y: int(e, "y")? as i32 },
            "userList" => Msg::UserList(e.children.iter().filter_map(info).collect()),
            "playerUpdate" => Msg::PlayerUpdate(info(e)?),
            "newPlayer" => Msg::NewPlayer(info(e)?),
            "playerLeft" => Msg::PlayerLeft(string(e, "name")?),
            "request" => Msg::Request(string(e, "name")?),
            "remRequest" => Msg::RemRequest(string(e, "name")?),
            "pong" => Msg::Pong,
            "draw" => Msg::Draw,
            "endGame" => Msg::EndGame { winner: e.get("winner").unwrap_or("").to_string() },
            "errorMsg" => Msg::ErrorMsg(e.text.clone()),
            _ => return None,
        })
    }
}

/// Reads one zero-terminated frame; `Ok(None)` at a clean end of stream. Frames above `max` bytes are an error.
pub fn read_frame<R: BufRead>(r: &mut R, max: usize) -> io::Result<Option<String>> {
    let mut buf: Vec<u8> = Vec::new();
    loop {
        let chunk = r.fill_buf()?;
        if chunk.is_empty() {
            return if buf.is_empty() { Ok(None) } else { Err(io::ErrorKind::UnexpectedEof.into()) };
        }
        let (take, done) = match chunk.iter().position(|&b| b == 0) {
            Some(p) => (p, true),
            None => (chunk.len(), false),
        };
        if buf.len() + take > max {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "frame too long"));
        }
        buf.extend_from_slice(&chunk[..take]);
        r.consume(take + usize::from(done));
        if done {
            return Ok(Some(String::from_utf8_lossy(&buf).into_owned()));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn cmd0() -> Cmd {
        Cmd::new(121.6, 40.2, [false, true, false, true])
    }

    fn all() -> Vec<Msg> {
        let p = PlayerInfo { name: "Anna".into(), skill: "3/1/0".into(), state: "0".into() };
        vec![
            Msg::Auth { name: "Anna".into(), version: VERSION.into(), hash: "h".into() },
            Msg::Beat,
            Msg::Challenge("Bob".into()),
            Msg::RemChallenge("Bob".into()),
            Msg::ChallengeAll,
            Msg::RemChallengeAll,
            Msg::StartGame("Bob".into()),
            Msg::MsgAll { name: "Anna".into(), msg: "hi".into() },
            Msg::MsgPlayer { name: "Anna".into(), msg: "gl hf".into() },
            Msg::ToRoom,
            Msg::PlayAgain,
            Msg::WinGame,
            Msg::DrawGame,
            Msg::Ping,
            Msg::Surrender(None),
            Msg::Surrender(Some("Bob".into())),
            Msg::Move(cmd0()),
            Msg::Stats { ping: 35, fps: 40 },
            Msg::Seed(12345),
            Msg::Bomb { cmd: cmd0(), cx: 3, cy: 4 },
            Msg::Pickup { cmd: cmd0(), cx: 3, cy: 4, speed: 0.16999999999999998, blast: 3 },
            Msg::Kick { cmd: cmd0(), cx: 3, cy: 4, id: 5000, dx: 0.1, dy: 0.0, t: 2100 },
            Msg::Stop { cmd: cmd0(), cx: 5, cy: 4, id: 5000, t: 1500 },
            Msg::Detonate { cmd: cmd0(), cx: 5, cy: 4, id: 5000 },
            Msg::Die { x: 120, y: 40 },
            Msg::UserList(vec![p.clone(), PlayerInfo { name: "Bob".into(), skill: "0/0/0".into(), state: "3".into() }]),
            Msg::PlayerUpdate(p.clone()),
            Msg::NewPlayer(p),
            Msg::PlayerLeft("Anna".into()),
            Msg::Request("Anna".into()),
            Msg::RemRequest("Anna".into()),
            Msg::Pong,
            Msg::Draw,
            Msg::EndGame { winner: "Anna".into() },
            Msg::ErrorMsg("Name vergeben".into()),
        ]
    }

    #[test]
    fn every_message_survives_a_roundtrip() {
        for m in all() {
            assert_eq!(Msg::from_xml(&m.to_xml()), Some(m.clone()), "{}", m.to_xml());
        }
    }

    #[test]
    fn game_messages_use_the_original_element_names_and_attributes() {
        assert_eq!(Msg::Move(cmd0()).to_xml(), "<12 c=\"0101\" x=\"122\" y=\"40\"/>");
        let b = Msg::Bomb { cmd: cmd0(), cx: 3, cy: 4 }.to_xml();
        assert!(b.starts_with("<17 ") && b.contains("xp=\"3\"") && b.contains("yp=\"4\""));
        assert_eq!(Msg::Seed(77).to_xml(), "<16 s=\"77\"/>");
        assert!(Msg::Ping.to_xml() == "<ping/>" && Msg::Pong.to_xml() == "<pong/>");
    }

    #[test]
    fn original_client_strings_parse() {
        // exact strings built by the original Flash client
        let m = Msg::from_xml("<12 x=\"120\"  y=\"40\" c=\"0010\"/>\n").unwrap();
        assert_eq!(m, Msg::Move(Cmd { x: 120, y: 40, c: "0010".into() }));
        assert_eq!(Msg::from_xml("<die x=\"5\"  y=\"7\"/>"), Some(Msg::Die { x: 5, y: 7 }));
        assert_eq!(Msg::from_xml("<14 p=\"30\" f=\"39\" />"), Some(Msg::Stats { ping: 30, fps: 39 }));
        assert!(Msg::from_xml("<17 c=\"0000\" x=\"1\"  y=\"2\" xp=\"0\" yp=\"0\"/>").is_some());
    }

    #[test]
    fn unknown_or_incomplete_messages_are_rejected() {
        for bad in ["<foo/>", "<12 x=\"1\"/>", "<16 s=\"abc\"/>", "<challenge/>", "<16 s=\"NaN\"/>", "garbage"] {
            assert_eq!(Msg::from_xml(bad), None, "{bad}");
        }
    }

    #[test]
    fn key_state_encoding() {
        let c = Cmd::new(0.0, 0.0, [true, false, false, true]);
        assert_eq!(c.c, "1001");
        assert_eq!(c.keys(), [true, false, false, true]);
        assert_eq!(Cmd { x: 0, y: 0, c: "1".into() }.keys(), [true, false, false, false]);
    }

    #[test]
    fn frames_are_split_on_zero_bytes_and_bounded() {
        let mut data = Vec::new();
        data.extend(Msg::Ping.to_frame());
        data.extend(Msg::Beat.to_frame());
        let mut r = Cursor::new(data);
        assert_eq!(read_frame(&mut r, 100).unwrap().as_deref(), Some("<ping/>\n"));
        assert_eq!(read_frame(&mut r, 100).unwrap().as_deref(), Some("<beat/>\n"));
        assert_eq!(read_frame(&mut r, 100).unwrap(), None);

        let mut long = Cursor::new(vec![b'a'; 500]);
        assert!(read_frame(&mut long, 100).is_err());
        let mut cut = Cursor::new(b"<ping/>".to_vec());
        assert!(read_frame(&mut cut, 100).is_err());
    }
}
