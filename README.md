# Bomber Pengu (Rust)

A clean-room reimplementation of the old-school Flash game **Pengu Bomberman**, written in Rust with [macroquad](https://macroquad.rs/). Play against the computer, two players on one keyboard, or online against another player through the bundled server.

This project is an unofficial fan reimplementation and is not affiliated with the original authors. The original game code is not included.

## Requirements

- Rust (stable, edition 2021) via [rustup](https://rustup.rs/)
- [Git LFS](https://git-lfs.com/) — the `assets/` directory is stored with LFS

## Get the sources

```sh
git lfs install
git clone <repository-url>
cd bomberpengu-rs
```

If you cloned before installing Git LFS, run `git lfs pull` to fetch the assets.

## Build and run

The game loads `assets/` relative to the working directory, so run it from the project root.

```sh
cargo run --release
```

Run the tests with `cargo test`.

On Linux, macroquad needs the usual X11/OpenGL and ALSA development packages (e.g. `libx11-dev libxi-dev libgl1-mesa-dev libasound2-dev`).

## Controls

| Action | Player 1 | Player 2 (two-player mode) |
|---|---|---|
| Move | W A S D | Arrow keys |
| Place bomb | Space | Enter |

Against the computer and online either key set works. The title screen has four buttons (play against the computer, two players, online lobby, settings); pick one with the mouse, arrow keys + `Enter`, or `1`-`4`. `Esc` quits. The settings screen changes your player name (also used online) and toggles the sound.

Collect items from destroyed boxes: extra bomb, speed, bigger blast, and bomb kick. A player who runs into flames loses.

## Online play

The original game talks to a server over an XML socket; its server is not available, so this repository contains a compatible one.

```sh
# one machine runs the server (default 0.0.0.0:6900)
cargo run --release --bin bomberpengu-server -- --listen 0.0.0.0:6900

# every player starts the game, picks "Online-Lobby" on the title screen and enters name + server
cargo run --release -- --name Anna --server 192.168.0.10:6900 --online
```

In the player room, pick a player with Up/Down or the mouse and press Enter (or click) to challenge him; clicking a player who challenged you accepts and starts the match (the one who accepts is player 1). `F2` challenges everybody, typing + Enter chats, `Esc` leaves. In a match use W A S D or the arrow keys and Space or Enter; `Esc` surrenders. After a match you can ask for a rematch or go back to the room.

The server has no encryption and no passwords; use it only on networks you trust.

## Play in the browser (WebAssembly)

The game also runs in a browser, and browser and native players meet in the same player room: the server accepts TCP clients and WebSocket clients on the same port.

```sh
rustup target add wasm32-unknown-unknown
web/build.sh        # Windows: powershell -File web\build.ps1
```

This creates `dist/` (page, `.wasm`, JavaScript glue and `assets/`). Upload it to any static web host (GitHub Pages, itch.io, nginx, ...) and run `bomberpengu-server` somewhere reachable. Everybody who opens the page and picks "Online-Lobby" ends up in the same room.

- The page connects to the server given in the URL, e.g. `https://example.org/bomberpengu/?server=wss://pengu.example.org`. Without `?server=` it uses `wss://<page host>` on HTTPS pages and `ws://<page host>:6900` on HTTP pages. The server field in the connect form can still be edited.
- A page served over HTTPS can only use `wss://`. The server itself does not speak TLS, so put a TLS-terminating reverse proxy in front of it (Caddy, nginx, Cloudflare, a platform router, ...) that forwards WebSocket connections to the server port.
- Locally: `cargo run --release --bin bomberpengu-server`, then serve `dist/` with any static file server (e.g. `python -m http.server 8080 --directory dist`) and open `http://localhost:8080/`.
- `web/gl.js` and `web/audio.js` are copied from the `miniquad` 0.4.11 and `quad-snd` 0.2.8 crates; replace them when those dependencies are updated.

## Command-line options

| Option | Description |
|---|---|
| `--mode ai\|2p\|demo` | Start a match directly (`demo`: computer vs. computer) |
| `--seed N` | Fixed level seed |
| `--screen title\|settings\|after` | Open a specific screen |
| `--shot FILE --shot-tick N` | Advance the simulation `N` ticks deterministically and save a screenshot to `FILE` |
| `--online` | Open the online screen (`--name NAME`, `--server HOST:PORT`; `--connect` connects at once) |
| `--challenge NAME`, `--accept` | Scripted runs: challenge a player / accept every challenge automatically |
| `--shot-frames N` | Take the `--shot` screenshot after `N` rendered frames (default 2) |

## Layout

- `src/world.rs`, `src/grid.rs`, `src/player.rs`, `src/rng.rs` — game rules and simulation (no rendering)
- `src/ai.rs` — computer opponent
- `src/proto.rs`, `src/xml.rs`, `src/netgame.rs`, `src/lobby.rs`, `src/net.rs` — online protocol, match synchronisation, player room, TCP client
- `src/server.rs`, `src/server_main.rs` — relay server (`bomberpengu-server`), TCP and WebSocket
- `src/online.rs` — online screens
- `src/assets.rs`, `src/main.rs`, `src/ui.rs` — asset loading with progress bar, rendering, input, screens and shared widgets (buttons, fields)
- `web/` — page, JavaScript glue (`bp_net.js` is the WebSocket transport) and build scripts for the browser version
- `assets/` — sprites, sounds and fonts (Git LFS)
