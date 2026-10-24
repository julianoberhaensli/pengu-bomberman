//! Browser clients talk to the same server over WebSocket (same port as the TCP clients).

use bomberpengu::net::{Connection, NetEvent};
use bomberpengu::proto::*;
use bomberpengu::server;
use std::net::TcpStream;
use std::time::{Duration, Instant};
use tungstenite::{stream::MaybeTlsStream, Message, WebSocket};

type Ws = WebSocket<MaybeTlsStream<TcpStream>>;

fn ws_connect(addr: &str, name: &str) -> Ws {
    let (mut ws, _) = tungstenite::connect(format!("ws://{addr}")).unwrap();
    if let MaybeTlsStream::Plain(socket) = ws.get_mut() {
        socket.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    }
    let auth = Msg::Auth { name: name.into(), version: VERSION.into(), hash: String::new() };
    ws.send(Message::text(auth.to_xml())).unwrap();
    assert_eq!(ws_expect(&mut ws, "server version", |message| matches!(message, Msg::ServerVersion(_))),
        Msg::ServerVersion(VERSION.into()));
    ws
}

fn ws_expect(ws: &mut Ws, what: &str, pred: impl Fn(&Msg) -> bool) -> Msg {
    loop {
        match ws.read().unwrap_or_else(|e| panic!("waiting for {what}: {e}")) {
            Message::Text(t) => {
                assert!(!t.as_str().contains('\0'), "WebSocket messages carry no frame terminator: {t:?}");
                let m = Msg::from_xml(t.as_str()).expect("server sends valid messages");
                if pred(&m) {
                    return m;
                }
            }
            Message::Close(_) => panic!("closed while waiting for {what}"),
            _ => {}
        }
    }
}

fn expect(c: &Connection, what: &str, pred: impl Fn(&Msg) -> bool) -> Msg {
    let end = Instant::now() + Duration::from_secs(5);
    while Instant::now() < end {
        for e in c.poll() {
            match e {
                NetEvent::Msg(m) if pred(&m) => return m,
                NetEvent::Closed(why) => panic!("closed while waiting for {what}: {why}"),
                _ => {}
            }
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!("timeout waiting for {what}");
}

#[test]
fn browser_and_native_players_share_one_room_and_play_each_other() {
    let srv = server::spawn("127.0.0.1:0").unwrap();
    let addr = srv.addr.to_string();
    let mut web = ws_connect(&addr, "Webby");
    ws_expect(&mut web, "user list", |m| matches!(m, Msg::UserList(l) if l.len() == 1));

    let native = Connection::connect(&addr, "Nat");
    expect(&native, "user list", |m| matches!(m, Msg::UserList(l) if l.len() == 2));
    ws_expect(&mut web, "newPlayer", |m| matches!(m, Msg::NewPlayer(p) if p.name == "Nat"));

    native.send(Msg::Challenge("Webby".into()));
    ws_expect(&mut web, "request", |m| *m == Msg::Request("Nat".into()));
    web.send(Message::text(Msg::StartGame("Nat".into()).to_xml())).unwrap();
    expect(&native, "startGame", |m| *m == Msg::StartGame("Webby".into()));

    web.send(Message::text(Msg::Seed(777).to_xml())).unwrap();
    expect(&native, "seed", |m| *m == Msg::Seed(777));
    let mv = Msg::Move(Cmd { x: 560, y: 440, c: "0001".into() });
    native.send(mv.clone());
    ws_expect(&mut web, "move", |m| *m == mv);
    web.send(Message::text(Msg::Ping.to_xml())).unwrap();
    ws_expect(&mut web, "pong", |m| *m == Msg::Pong);

    web.send(Message::text(Msg::Beat.to_xml())).unwrap();
    drop(native);
    ws_expect(&mut web, "playerLeft", |m| *m == Msg::PlayerLeft("Nat".into()));
    srv.stop();
}

#[test]
fn websocket_clients_are_refused_like_tcp_clients() {
    let srv = server::spawn("127.0.0.1:0").unwrap();
    let addr = srv.addr.to_string();
    let mut first = ws_connect(&addr, "Anna");
    ws_expect(&mut first, "user list", |m| matches!(m, Msg::UserList(_)));

    let mut dup = ws_connect(&addr, "Anna");
    let err = ws_expect(&mut dup, "error", |m| matches!(m, Msg::ErrorMsg(_)));
    assert!(matches!(err, Msg::ErrorMsg(t) if t.contains("vergeben")));
    assert!(matches!(dup.read(), Ok(Message::Close(_)) | Err(_)), "the server closes after the error");
    let (mut old, _) = tungstenite::connect(format!("ws://{addr}")).unwrap();
    if let MaybeTlsStream::Plain(socket) = old.get_mut() {
        socket.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    }
    old.send(Message::text(Msg::Auth { name: "Old".into(), version: "old-release/1".into(), hash: String::new() }.to_xml())).unwrap();
    assert!(matches!(ws_expect(&mut old, "version error", |message| matches!(message, Msg::ErrorMsg(_))),
        Msg::ErrorMsg(reason) if reason.contains("Spielversion")));
    srv.stop();
}
