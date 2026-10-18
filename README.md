# Bomber Pengu (Rust)

A clean-room reimplementation of the old-school Flash game **Pengu Bomberman**, written in Rust with [macroquad](https://macroquad.rs/). Local play only: against the computer or two players on one keyboard.

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

## Command-line options

| Option | Description |
|---|---|
| `--mode ai\|2p\|demo` | Start a match directly (`demo`: computer vs. computer) |
| `--seed N` | Fixed level seed |
| `--screen title\|after` | Open a specific screen |
| `--shot FILE --shot-tick N` | Advance the simulation `N` ticks deterministically and save a screenshot to `FILE` |

## Layout

- `src/world.rs`, `src/grid.rs`, `src/player.rs`, `src/rng.rs` — game rules and simulation (no rendering)
- `src/ai.rs` — computer opponent
- `src/assets.rs`, `src/main.rs` — asset loading, rendering, input and screens
- `assets/` — sprites, sounds and fonts (Git LFS)
