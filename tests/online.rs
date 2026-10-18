use bomberpengu::net::{Connection, NetEvent};
use bomberpengu::proto::*;
use bomberpengu::server;
use std::time::{Duration, Instant};

/// Waits for a message matching `pred`, collecting everything seen on the way.
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

/// Everything received until the connection closes.
fn until_closed(c: &Connection) -> (Vec<Msg>, Option<String>) {
    let mut msgs = Vec::new();
    let end = Instant::now() + Duration::from_secs(5);
    while Instant::now() < end {
        for e in c.poll() {
            match e {
                NetEvent::Msg(m) => msgs.push(m),
                NetEvent::Closed(why) => return (msgs, Some(why)),
            }
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    (msgs, None)
}

#[test]
fn two_clients_meet_challenge_play_and_chat_over_tcp() {
    let srv = server::spawn("127.0.0.1:0").unwrap();
    let addr = srv.addr.to_string();
    let anna = Connection::connect(&addr, "Anna");
    expect(&anna, "Anna's user list", |m| matches!(m, Msg::UserList(l) if l.len() == 1));
    let bob = Connection::connect(&addr, "Bob");
    expect(&bob, "Bob's user list", |m| matches!(m, Msg::UserList(l) if l.len() == 2));
    expect(&anna, "newPlayer Bob", |m| matches!(m, Msg::NewPlayer(p) if p.name == "Bob"));

    bob.send(Msg::Challenge("Anna".into()));
    expect(&anna, "request", |m| *m == Msg::Request("Bob".into()));
    anna.send(Msg::StartGame("Bob".into()));
    expect(&bob, "startGame", |m| *m == Msg::StartGame("Anna".into()));

    // player 1 sends the seed, both sides exchange moves
    anna.send(Msg::Seed(777));
    expect(&bob, "seed", |m| *m == Msg::Seed(777));
    let mv = Msg::Move(Cmd { x: 160, y: 40, c: "0010".into() });
    bob.send(mv.clone());
    expect(&anna, "move", |m| *m == mv);
    bob.send(Msg::Ping);
    expect(&bob, "pong", |m| *m == Msg::Pong);

    anna.send(Msg::MsgPlayer { name: "Anna".into(), msg: "gl".into() });
    expect(&bob, "chat", |m| matches!(m, Msg::MsgPlayer { name, msg } if name == "Anna" && msg == "gl"));

    bob.send(Msg::Die { x: 600, y: 440 });
    expect(&anna, "die", |m| matches!(m, Msg::Die { .. }));
    anna.send(Msg::WinGame);
    expect(&bob, "endGame", |m| matches!(m, Msg::EndGame { winner } if winner == "Anna"));

    // leaving ends the connection for the other side's player list
    drop(bob);
    expect(&anna, "playerLeft", |m| *m == Msg::PlayerLeft("Bob".into()));
    srv.stop();
}

#[test]
fn duplicate_names_and_wrong_versions_are_refused() {
    let srv = server::spawn("127.0.0.1:0").unwrap();
    let addr = srv.addr.to_string();
    let first = Connection::connect(&addr, "Anna");
    expect(&first, "list", |m| matches!(m, Msg::UserList(_)));

    let dup = Connection::connect(&addr, "Anna");
    let (msgs, why) = until_closed(&dup);
    assert!(msgs.iter().any(|m| matches!(m, Msg::ErrorMsg(t) if t.contains("vergeben"))), "{msgs:?}");
    assert!(why.is_some(), "the server closes the connection after the error");

    // raw client with a wrong version string
    use std::io::{BufReader, Write};
    let mut s = std::net::TcpStream::connect(&addr).unwrap();
    s.write_all(b"<auth name=\"Old\" version=\"0.9\" hash=\"\"/>\n\0").unwrap();
    let mut r = BufReader::new(s);
    let frame = read_frame(&mut r, 4096).unwrap().unwrap();
    assert!(matches!(Msg::from_xml(&frame), Some(Msg::ErrorMsg(_))), "{frame}");
    assert_eq!(read_frame(&mut r, 4096).unwrap(), None, "server closes after the error");
    srv.stop();
}

#[test]
fn oversized_and_garbage_frames_do_not_hurt_the_server() {
    let srv = server::spawn("127.0.0.1:0").unwrap();
    let addr = srv.addr.to_string();
    use std::io::{Read, Write};
    let mut bad = std::net::TcpStream::connect(&addr).unwrap();
    bad.write_all(&vec![b'x'; 10_000]).unwrap();
    let mut buf = [0u8; 16];
    // the server drops the connection without a reply
    assert!(matches!(bad.read(&mut buf), Ok(0) | Err(_)));

    let mut junk = std::net::TcpStream::connect(&addr).unwrap();
    junk.write_all(b"<nonsense/>\0").unwrap();
    assert!(matches!(junk.read(&mut buf), Ok(0) | Err(_)));

    let ok = Connection::connect(&addr, "Anna");
    expect(&ok, "list", |m| matches!(m, Msg::UserList(_)));
    srv.stop();
}
