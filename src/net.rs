//! Client side of the socket connection (`Connector`'s `XMLSocket`), on background threads.

use crate::proto::*;
use std::io::{BufReader, Write};
use std::net::{Shutdown, TcpStream, ToSocketAddrs};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

/// `Connector.msgInt`: `<beat/>` after this much silence.
pub const HEARTBEAT: Duration = Duration::from_secs(15);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_SERVER_FRAME: usize = 64 * 1024;

#[derive(Debug)]
pub enum NetEvent {
    Msg(Msg),
    /// The connection is gone (`onSockClose` / failed connect); the text says why.
    Closed(String),
}

pub struct Connection {
    tx: Sender<Msg>,
    rx: Receiver<NetEvent>,
    stream: Arc<Mutex<Option<TcpStream>>>,
}

impl Connection {
    /// Connects in the background and sends `auth` as soon as the socket is up.
    pub fn connect(addr: &str, name: &str) -> Connection {
        let (tx, out_rx) = mpsc::channel::<Msg>();
        let (in_tx, rx) = mpsc::channel::<NetEvent>();
        let stream: Arc<Mutex<Option<TcpStream>>> = Arc::new(Mutex::new(None));
        let _ = tx.send(Msg::Auth { name: name.to_string(), version: VERSION.to_string(), hash: String::new() });
        let (addr, shared) = (addr.to_string(), stream.clone());
        thread::spawn(move || run(&addr, out_rx, in_tx, shared));
        Connection { tx, rx, stream }
    }

    pub fn send(&self, m: Msg) {
        let _ = self.tx.send(m);
    }

    /// Everything that arrived since the last call.
    pub fn poll(&self) -> Vec<NetEvent> {
        self.rx.try_iter().collect()
    }

    pub fn close(&self) {
        if let Some(s) = self.stream.lock().unwrap_or_else(|e| e.into_inner()).take() {
            let _ = s.shutdown(Shutdown::Both);
        }
    }
}

impl Drop for Connection {
    fn drop(&mut self) {
        self.close();
    }
}

fn open(addr: &str) -> std::io::Result<TcpStream> {
    let target = addr
        .to_socket_addrs()?
        .next()
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidInput, "Adresse nicht gefunden"))?;
    let s = TcpStream::connect_timeout(&target, CONNECT_TIMEOUT)?;
    s.set_nodelay(true)?;
    Ok(s)
}

fn run(addr: &str, out_rx: Receiver<Msg>, in_tx: Sender<NetEvent>, shared: Arc<Mutex<Option<TcpStream>>>) {
    let stream = match open(addr) {
        Ok(s) => s,
        Err(e) => {
            let _ = in_tx.send(NetEvent::Closed(format!("Verbindung fehlgeschlagen: {e}")));
            return;
        }
    };
    let Ok(mut writer) = stream.try_clone() else {
        let _ = in_tx.send(NetEvent::Closed("Verbindung fehlgeschlagen".into()));
        return;
    };
    if let Ok(copy) = stream.try_clone() {
        *shared.lock().unwrap_or_else(|e| e.into_inner()) = Some(copy);
    }
    thread::spawn(move || {
        loop {
            let m = match out_rx.recv_timeout(HEARTBEAT) {
                Ok(m) => m,
                Err(RecvTimeoutError::Timeout) => Msg::Beat,
                Err(RecvTimeoutError::Disconnected) => break,
            };
            if writer.write_all(&m.to_frame()).is_err() {
                break;
            }
        }
        let _ = writer.shutdown(Shutdown::Both);
    });
    let mut reader = BufReader::new(stream);
    while let Ok(Some(frame)) = read_frame(&mut reader, MAX_SERVER_FRAME) {
        if let Some(m) = Msg::from_xml(&frame) {
            if in_tx.send(NetEvent::Msg(m)).is_err() {
                return;
            }
        }
    }
    let _ = in_tx.send(NetEvent::Closed("Verbindung getrennt".into()));
}
