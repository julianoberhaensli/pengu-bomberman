//! Client side of the socket connection (`Connector`'s `XMLSocket`).
//!
//! Native builds speak zero-terminated XML over TCP on background threads; the browser build sends the same
//! messages as WebSocket text through `web/bp_net.js`.

use crate::proto::Msg;

#[derive(Debug)]
pub enum NetEvent {
    Msg(Msg),
    /// The connection is gone (`onSockClose` / failed connect); the text says why.
    Closed(String),
}

#[cfg(not(target_arch = "wasm32"))]
pub use tcp::{default_server, Connection, HEARTBEAT};
#[cfg(target_arch = "wasm32")]
pub use web::{default_server, Connection};

#[cfg(not(target_arch = "wasm32"))]
mod tcp {
    use super::NetEvent;
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

    pub fn default_server() -> String {
        format!("127.0.0.1:{}", crate::server::DEFAULT_PORT)
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
}

#[cfg(target_arch = "wasm32")]
mod web {
    use super::NetEvent;
    use crate::proto::*;

    const KIND_MSG: i32 = 1;

    extern "C" {
        fn bp_ws_open(url: *const u8, len: usize) -> u32;
        fn bp_ws_send(id: u32, text: *const u8, len: usize);
        fn bp_ws_close(id: u32);
        /// Kind of the oldest queued event (0 none, 1 message, 2 closed).
        fn bp_ws_next_kind(id: u32) -> i32;
        fn bp_ws_next_len(id: u32) -> usize;
        /// Moves the oldest event's text into `dst` (which must hold `bp_ws_next_len` bytes) and drops it from the queue.
        fn bp_ws_take(id: u32, dst: *mut u8);
        fn bp_page_server(dst: *mut u8, cap: usize) -> usize;
    }

    /// Checked by miniquad against `version` in `bp_net.js`.
    #[no_mangle]
    pub extern "C" fn bp_net_crate_version() -> u32 {
        1
    }

    /// The page decides (`?server=` in the URL, otherwise the host that served the page).
    pub fn default_server() -> String {
        let mut buf = [0u8; 256];
        let n = unsafe { bp_page_server(buf.as_mut_ptr(), buf.len()) };
        String::from_utf8_lossy(&buf[..n.min(buf.len())]).into_owned()
    }

    pub struct Connection {
        id: u32,
    }

    impl Connection {
        pub fn connect(addr: &str, name: &str) -> Connection {
            let url = if addr.starts_with("ws://") || addr.starts_with("wss://") { addr.to_string() } else { format!("ws://{addr}") };
            let id = unsafe { bp_ws_open(url.as_ptr(), url.len()) };
            let c = Connection { id };
            c.send(Msg::Auth { name: name.to_string(), version: VERSION.to_string(), hash: String::new() });
            c
        }

        pub fn send(&self, m: Msg) {
            let xml = m.to_xml();
            unsafe { bp_ws_send(self.id, xml.as_ptr(), xml.len()) };
        }

        /// Everything that arrived since the last call.
        pub fn poll(&self) -> Vec<NetEvent> {
            let mut out = Vec::new();
            loop {
                let kind = unsafe { bp_ws_next_kind(self.id) };
                if kind == 0 {
                    return out;
                }
                let mut buf = vec![0u8; unsafe { bp_ws_next_len(self.id) }];
                unsafe { bp_ws_take(self.id, buf.as_mut_ptr()) };
                let text = String::from_utf8_lossy(&buf);
                if kind == KIND_MSG {
                    if let Some(m) = Msg::from_xml(&text) {
                        out.push(NetEvent::Msg(m));
                    }
                } else {
                    out.push(NetEvent::Closed(text.into_owned()));
                }
            }
        }

        pub fn close(&self) {
            unsafe { bp_ws_close(self.id) };
        }
    }

    impl Drop for Connection {
        fn drop(&mut self) {
            self.close();
        }
    }
}
