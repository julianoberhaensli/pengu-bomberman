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

The original game's server is not available, so this repository includes its own server based on the XML socket protocol, with additional validation and a version handshake.

```sh
# one machine runs the server (default 0.0.0.0:6900)
cargo run --release --bin bomberpengu-server -- --listen 0.0.0.0:6900

# every player starts the game, picks "Online-Lobby" on the title screen and enters name + server
cargo run --release -- --name Anna --server 192.168.0.10:6900 --online
```

In the player room, pick a player with Up/Down or the mouse and press Enter (or click) to challenge him; clicking a player who challenged you accepts and starts the match (the one who accepts is player 1). `F2` challenges everybody, typing + Enter chats, `Esc` leaves. In a match use W A S D or the arrow keys and Space or Enter; `Esc` surrenders. After a match you can ask for a rematch or go back to the room.

Client and server must use exactly the same package version and wire-protocol revision. Both directions of the connection verify this before opening the lobby; older clients or servers are rejected. Rebuild and deploy the native client, browser WASM and server together. The shared version is defined in `src/proto.rs` from Cargo's package version plus a protocol revision. Bump the package version for releases and the protocol revision for incompatible wire changes. This is a compatibility check, not authentication of the executable.

### Security and deployment

- Connections are capped at 128 total and 32 per source IP, including unauthenticated connections. Authentication has a 10-second absolute deadline; later frames have a 45-second deadline. A connection may send at most 300 messages per second.
- Outgoing queues hold at most 256 frames; slow recipients are disconnected. TCP writes time out after 2 seconds. Browser queues, frames and pending connection attempts are bounded too.
- The server checks seeded board state, movement budgets, bomb capacity, actual item effects, kick permissions and bomb-control ownership before relaying gameplay. Remote kicked bombs also have a fuse fallback if their detonation message is lost.
- Returning to the room or disconnecting during a match forfeits it. Rematches require a completed match and the other player's request.
- Records are in memory only. At 10,000 retained names, inactive players' records are pruned on the next join.
- Names are still unauthenticated, and deaths and some movement decisions remain client-reported. These checks do not make the game a fully authoritative competitive server or prevent every form of cheating.
- Raw native TCP has no encryption or passwords: use it only on a trusted network or through a secure tunnel. For public browser play, use HTTPS/WSS through a TLS reverse proxy. HTTP, WebSocket and native TCP share the same listener; restrict direct access to that port when deploying behind a proxy. Clients behind the same reverse proxy share its per-IP connection limit.

### One Rust server

After building the browser game (see below), serve it and the online room from one process:

```sh
cargo run --release --bin bomberpengu-server -- --listen 127.0.0.1:8080 --web-root dist
```

Open `http://localhost:8080/`. The Rust binary serves static files, generates `/config.js` pointing the browser to same-origin `/ws`, and handles WebSocket and native TCP clients on that same port. URL overrides (`?server=...`) still work. Without `--web-root`, it remains a relay-only server. `SERVER_PORT` sets the default port; explicit `--listen` takes precedence. There is no Node.js, internal Caddy, or shell supervisor. The browser still needs JavaScript glue to connect Rust/WASM to browser APIs.

Only point `--web-root` at the built public `dist/` directory, never at the repository or a directory containing secrets. HTTP supports GET/HEAD, bounded headers, streaming file bodies and WASM MIME types; directory listing and paths outside the web root are rejected. TLS is deliberately left to an external reverse proxy.

### Combined Docker deployment (recommended)

The root `Dockerfile` builds both the browser game and relay from the same source version. One Rust process serves the game over HTTP and handles WebSockets at `/ws` on the same port. The browser automatically uses that endpoint, including `wss://` behind HTTPS. No separate static host or `PENGU_SERVER` setting is needed. URL overrides (`?server=...`) still work.

Install Git LFS and run `git lfs pull` before building; missing LFS assets fail the image build. For a local smoke test:

```sh
docker build -t bomberpengu:local .
docker run --rm --name bomberpengu -p 127.0.0.1:8080:8080 bomberpengu:local
```

Open `http://localhost:8080/`. The image runs as UID 1000 and uses `SERVER_PORT=8080` by default. Set `-e SERVER_PORT=9000 -p 127.0.0.1:9000:9000` to change it. Use unprivileged ports (1024-65535) in Docker and Pterodactyl. Stopping the container stops the Rust process directly. Native clients can connect to the same published port, so keep it private behind your HTTPS proxy for public browser deployments.

For a public server, point a hostname at the host, allow inbound TCP ports 80 and 443, and put this in `deploy/.env`:

```dotenv
PENGU_DOMAIN=pengu.example.org
```

```sh
docker compose -f deploy/docker-compose.yml up -d --build
```

Open `https://pengu.example.org/`. The Compose stack adds an external Caddy container that obtains and renews TLS certificates; the browser and relay still run together in the application container. Keep the Caddy volumes to retain certificates. If another proxy already owns ports 80/443, run just the application container and proxy the entire hostname (including WebSocket upgrades) to its HTTP port instead.

CI publishes this combined image as `ghcr.io/<owner>/<repo>-server:latest` and a `sha-...` tag. To deploy that image rather than building locally, set `PENGU_IMAGE` in `deploy/.env` and run `docker compose -f deploy/docker-compose.yml pull` followed by `docker compose -f deploy/docker-compose.yml up -d --no-build`. Prefer an immutable tag or digest for predictable deployments. Image updates include both browser and server; refresh open browser tabs after an update. No game data is persisted.

### Pterodactyl

Import `deploy/egg-bomber-pengu.json` into a nest in a Pterodactyl 1.x panel. Before importing, replace `ghcr.io/OWNER/REPO-server:latest` with your published image (or edit the egg's Docker Images field afterward). The existing CI image job can publish it once this repository is pushed. Make the package public or configure Wings registry credentials for a private image.

1. Create a server with this egg and image. A starting limit of 256 MiB RAM and 256 MiB disk is reasonable; increase it if monitoring shows pressure. Compilation happens during the image build, not in Wings.
2. Assign one TCP allocation with a port between 1024 and 65535. Pterodactyl supplies `SERVER_PORT` from the primary allocation. The image's fixed Rust entrypoint and default arguments serve `/srv/pengu`; the panel startup field is informational, not a shell command launcher.
3. Start the server and open `http://<node-address>:<allocation-port>/` to test. The installer intentionally downloads nothing: the binary, WASM and assets are inside the image, not the panel file manager.
4. For public play, configure an HTTPS reverse proxy for your game hostname to `http://<node-address>:<allocation-port>`, forwarding the whole site and WebSocket upgrades. Open the HTTPS hostname to play. The panel's own HTTPS configuration does not provide TLS for game allocations.

If using Caddy on the node or another trusted host, a site such as `pengu.example.org { reverse_proxy 127.0.0.1:8080 }` works when the allocation is reachable there on port 8080. Use the actual node address when the proxy runs elsewhere. Restrict direct allocation access to the proxy for production. Only one room server runs per container, and all players through the external proxy share the server's 32-connections-per-IP limit.

## Play in the browser (WebAssembly)

The game also runs in a browser, and browser and native players meet in the same player room: the server accepts TCP clients and WebSocket clients on the same port.

```sh
rustup target add wasm32-unknown-unknown
web/build.sh        # Windows: powershell -File web\build.ps1
```

This creates `dist/` (page, `.wasm`, JavaScript glue and `assets/`). Serve it with the Rust server's `--web-root dist`, or upload it to any static web host (GitHub Pages, itch.io, nginx, ...) and run the relay somewhere reachable. Everybody who opens the page and picks "Online-Lobby" ends up in the same room.

- The page connects to the server given in the URL, e.g. `https://example.org/bomberpengu/?server=wss://pengu.example.org`. The integrated Rust web server defaults to same-origin `/ws`. On other static hosts, configure `web/config.js`; without a configured default it uses `wss://<page host>` on HTTPS pages and `ws://<page host>:6900` on HTTP pages. The server field in the connect form can still be edited.
- A page served over HTTPS can only use `wss://`. The server itself does not speak TLS, so put a TLS-terminating reverse proxy in front of it (Caddy, nginx, Cloudflare, a platform router, ...) that forwards WebSocket connections to the server port.
- Locally: `cargo run --release --bin bomberpengu-server -- --listen 127.0.0.1:8080 --web-root dist`, then open `http://localhost:8080/`.
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
- `src/server.rs`, `src/server_main.rs`, `src/http.rs` — Rust HTTP/static-file and relay server (`bomberpengu-server`), TCP and WebSocket
- `src/online.rs` — online screens
- `src/assets.rs`, `src/main.rs`, `src/ui.rs` — asset loading with progress bar, rendering, input, screens and shared widgets (buttons, fields)
- `web/` — page, JavaScript glue (`bp_net.js` is the WebSocket transport) and build scripts for the browser version
- `assets/` — sprites, sounds and fonts (Git LFS)
