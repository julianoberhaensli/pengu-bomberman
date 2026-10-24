//! HTTP and relay server: `bomberpengu-server [--listen ADDR:PORT] [--web-root DIR]`.

#[cfg(target_arch = "wasm32")]
fn main() {}

#[cfg(not(target_arch = "wasm32"))]
use bomberpengu::server::{run, run_with_web_root, DEFAULT_PORT};

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    if let Err(e) = serve() {
        eprintln!("server error: {e}");
        std::process::exit(1);
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn serve() -> std::io::Result<()> {
    use std::io::{Error, ErrorKind};
    let mut args = std::env::args().skip(1);
    let mut listen = None;
    let mut web_root = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--listen" | "--web-root" => {
                let value = args.next().filter(|value| !value.starts_with("--"))
                    .ok_or_else(|| Error::new(ErrorKind::InvalidInput, format!("{arg} requires a value")))?;
                if arg == "--listen" { listen = Some(value); } else { web_root = Some(value); }
            }
            "--help" | "-h" => {
                println!("Usage: bomberpengu-server [--listen ADDR:PORT] [--web-root DIR]\n\
                    SERVER_PORT sets the default listening port (otherwise 6900). --listen takes precedence.\n\
                    --web-root enables HTTP files and same-origin WebSockets alongside native TCP.\n\
                    No TLS or passwords: use an HTTPS reverse proxy for public browser play.");
                return Ok(());
            }
            _ => return Err(Error::new(ErrorKind::InvalidInput, format!("unknown option: {arg}"))),
        }
    }
    let addr = match listen {
        Some(addr) => addr,
        None => {
            let port = match std::env::var("SERVER_PORT") {
                Ok(value) => value.parse::<u16>().ok().filter(|port| *port != 0)
                    .ok_or_else(|| Error::new(ErrorKind::InvalidInput, "SERVER_PORT must be between 1 and 65535"))?,
                Err(std::env::VarError::NotPresent) => DEFAULT_PORT,
                Err(error) => return Err(Error::new(ErrorKind::InvalidInput, error)),
            };
            format!("0.0.0.0:{port}")
        }
    };
    match web_root {
        Some(root) => run_with_web_root(addr.as_str(), root),
        None => run(addr.as_str()),
    }
}
