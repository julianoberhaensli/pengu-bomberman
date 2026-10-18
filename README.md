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
| Move | Arrow keys | W A S D |
| Place bomb | Space | Left Shift |

Title screen: `Enter` or click Start to play against the computer, `2` for two players on one keyboard, `Esc` to quit.

Collect items from destroyed boxes: extra bomb, speed, bigger blast, and bomb kick. A player who runs into flames loses.

## Online play

The original game talks to a server over an XML socket; its server is not available, so this repository contains a compatible one.

```sh
# one machine runs the server (default 0.0.0.0:6900)
cargo run --release --bin bomberpengu-server -- --listen 0.0.0.0:6900

# every player starts the game, presses O on the title screen and enters name + server
cargo run --release -- --name Anna --server 192.168.0.10:6900 --online
```

In the player room, pick a player with Up/Down or the mouse and press Enter (or click) to challenge him; clicking a player who challenged you accepts and starts the match (the one who accepts is player 1). `F2` challenges everybody, typing + Enter chats, `Esc` leaves. In a match use the arrow keys and Space; `Esc` surrenders. After a match you can ask for a rematch or go back to the room.

The server has no encryption and no passwords; use it only on networks you trust.

## Command-line options

| Option | Description |
|---|---|
| `--mode ai\|2p\|demo` | Start a match directly (`demo`: computer vs. computer) |
| `--seed N` | Fixed level seed |
| `--screen title\|after` | Open a specific screen |
| `--shot FILE --shot-tick N` | Advance the simulation `N` ticks deterministically and save a screenshot to `FILE` |
| `--online` | Open the online screen (`--name NAME`, `--server HOST:PORT`; `--connect` connects at once) |
| `--challenge NAME`, `--accept` | Scripted runs: challenge a player / accept every challenge automatically |
| `--shot-frames N` | Take the `--shot` screenshot after `N` rendered frames (default 2) |

## Layout

- `src/world.rs`, `src/grid.rs`, `src/player.rs`, `src/rng.rs` — game rules and simulation (no rendering)
- `src/ai.rs` — computer opponent
- `src/proto.rs`, `src/xml.rs`, `src/netgame.rs`, `src/lobby.rs`, `src/net.rs` — online protocol, match synchronisation, player room, TCP client
- `src/server.rs`, `src/server_main.rs` — relay server (`bomberpengu-server`)
- `src/online.rs` — online screens
- `src/assets.rs`, `src/main.rs` — asset loading, rendering, input and screens
- `assets/` — sprites, sounds and fonts (Git LFS)
