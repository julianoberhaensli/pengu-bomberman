//! Browser clients talk to the same server over WebSocket (same port as the TCP clients).

use bomberpengu::net::{Connection, NetEvent};
use bomberpengu::proto::*;
use bomberpengu::server;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant};
use tungstenite::{stream::MaybeTlsStream, Message, WebSocket};

type Ws = WebSocket<MaybeTlsStream<TcpStream>>;

#[test]
fn rust_web_server_serves_files_and_websockets_on_one_port() {
    let root = std::env::temp_dir().join(format!("pengu-web-{}-{}", std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
    std::fs::create_dir_all(root.join("assets")).unwrap();
    std::fs::write(root.join("index.html"), b"<canvas></canvas>").unwrap();
    std::fs::write(root.join("bomberpengu.wasm"), b"\0asm\x01\0\0\0").unwrap();
    std::fs::write(root.join("assets/test.png"), [0, 255, 128, 42]).unwrap();
    assert!(server::spawn_with_web_root("127.0.0.1:0", root.join("index.html")).is_err());
    #[cfg(unix)]
    std::os::unix::fs::symlink(std::env::current_dir().unwrap().join("Cargo.toml"), root.join("outside")).unwrap();
    let srv = server::spawn_with_web_root("127.0.0.1:0", &root).unwrap();
    let request = |method: &str, path: &str| {
        let mut socket = TcpStream::connect(srv.addr).unwrap();
        socket.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        write!(socket, "{method} {path} HTTP/1.1\r\nHost: localhost\r\n\r\n").unwrap();
        let mut response = Vec::new();
        socket.read_to_end(&mut response).unwrap();
        response
    };
    for (path, mime, body) in [
        ("/", "text/html; charset=utf-8", b"<canvas></canvas>".as_slice()),
        ("/bomberpengu.wasm?v=1", "application/wasm", b"\0asm\x01\0\0\0".as_slice()),
        ("/assets/test.png", "image/png", [0, 255, 128, 42].as_slice()),
    ] {
        let response = request("GET", path);
        let boundary = response.windows(4).position(|part| part == b"\r\n\r\n").unwrap() + 4;
        let headers = std::str::from_utf8(&response[..boundary]).unwrap();
        assert!(headers.starts_with("HTTP/1.1 200 OK\r\n"));
        assert!(headers.contains(&format!("Content-Type: {mime}\r\n")));
        assert!(headers.contains(&format!("Content-Length: {}\r\n", body.len())));
        assert_eq!(&response[boundary..], body);
    }
    let head = String::from_utf8(request("HEAD", "/")).unwrap();
    assert!(head.contains("Content-Length: 17\r\n"));
    assert!(head.ends_with("\r\n\r\n"));
    for path in ["/../Cargo.toml", "/%2e%2e/Cargo.toml", "/assets%5c..%5cCargo.toml", "/index.html:secret"] {
        assert!(request("GET", path).starts_with(b"HTTP/1.1 403"), "{path}");
    }
    assert!(request("GET", "/missing").starts_with(b"HTTP/1.1 404"));
    assert!(request("GET", "/assets/").starts_with(b"HTTP/1.1 404"));
    #[cfg(unix)]
    assert!(request("GET", "/outside").starts_with(b"HTTP/1.1 404"));
    assert!(request("POST", "/").starts_with(b"HTTP/1.1 405"));
    let config = String::from_utf8(request("GET", "/config.js")).unwrap();
    assert!(config.contains("text/javascript; charset=utf-8"));
    assert!(config.contains("location.host + '/ws'"));
    assert!(config.contains("'wss://' : 'ws://'"));
    let mut web = ws_connect(&format!("{}/ws", srv.addr), "Webby");
    ws_expect(&mut web, "user list", |message| matches!(message, Msg::UserList(_)));
    let root_web = ws_connect(&srv.addr.to_string(), "Root");
    let native = Connection::connect(&srv.addr.to_string(), "Nat");
    expect(&native, "user list", |message| matches!(message, Msg::UserList(list) if list.len() == 3));
    drop(native);
    drop(root_web);
    drop(web);
    srv.stop();
    std::fs::remove_dir_all(root).unwrap();
}

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
