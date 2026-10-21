//! Relay server for online Bomber Pengu matches: `bomberpengu-server [--listen ADDR:PORT]`.

#[cfg(target_arch = "wasm32")]
fn main() {}

#[cfg(not(target_arch = "wasm32"))]
use bomberpengu::server::{run, DEFAULT_PORT};

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    let args: Vec<String> = std::env::args().collect();
    let addr = args
        .iter()
        .position(|a| a == "--listen")
        .and_then(|i| args.get(i + 1))
        .cloned()
        .unwrap_or_else(|| format!("0.0.0.0:{DEFAULT_PORT}"));
    println!("Bomber Pengu server listening on {addr} (no encryption, no passwords)");
    if let Err(e) = run(addr.as_str()) {
        eprintln!("server error: {e}");
        std::process::exit(1);
    }
}
