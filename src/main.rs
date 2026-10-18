mod assets;
mod online;

use assets::Assets;
use bomberpengu::ai::Ai;
use bomberpengu::game::{Match, Outcome, Phase};
use bomberpengu::grid::*;
use bomberpengu::player::{Anim, Dir, Player};
use bomberpengu::world::{Event, World, DT_MS};
use macroquad::prelude::*;

/// Flash frame rate of the original: 40 fps -> fixed 25 ms step.
const STEP: f64 = DT_MS as f64 / 1000.0;
const INK: Color = Color::new(0.08, 0.16, 0.28, 1.0);

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    VsAi,
    TwoPlayers,
    /// Both penguins played by the computer (used for `--demo` and automated screenshots).
    Demo,
}

struct Game {
    m: Match,
    mode: Mode,
    ai: [Option<Ai>; 2],
}

enum Screen {
    Title,
    Playing(Game),
    After { mode: Mode, outcome: Outcome },
    Online(Box<online::Online>),
}

fn window_conf() -> Conf {
    Conf { window_title: "Bomber Pengu".to_string(), window_width: 640, window_height: 640, window_resizable: false, ..Default::default() }
}

fn new_seed() -> i64 {
    // original: Util.randomMinMax(0, 20000) with Math.random
    rand::gen_range(0, 20001)
}

impl Game {
    fn new(mode: Mode, seed: i64, assets: &Assets) -> Self {
        let ai = match mode {
            Mode::VsAi => [None, Some(Ai::new())],
            Mode::TwoPlayers => [None, None],
            Mode::Demo => [Some(Ai::new()), Some(Ai::new())],
        };
        assets.play(&assets.sounds.music);
        Self { m: Match::new(seed), mode, ai }
    }

    fn names(&self) -> [&'static str; 2] {
        match self.mode {
            Mode::VsAi => ["Spieler", "Computer"],
            Mode::TwoPlayers => ["Spieler 1", "Spieler 2"],
            Mode::Demo => ["Computer 1", "Computer 2"],
        }
    }

    fn tick(&mut self, assets: &Assets) {
        if self.m.phase == Phase::Playing {
            for pi in 0..2 {
                if let Some(ai) = &mut self.ai[pi] {
                    ai.control(&mut self.m.world, pi);
                }
            }
        }
        self.m.tick();
        for e in self.m.world.events.drain(..) {
            assets.play(match e {
                Event::BombPlaced => &assets.sounds.donk,
                Event::Explosion => &assets.sounds.explode,
                Event::PickUp => &assets.sounds.pling,
                Event::Kick => &assets.sounds.landing,
            });
        }
    }

    /// Keyboard -> the original `onKeyDown/onKeyUp` handlers. Player 1: arrows + Space, player 2 (hot seat): WASD + Left Shift.
    fn input(&mut self) {
        let maps: [([KeyCode; 4], KeyCode); 2] = [
            ([KeyCode::Left, KeyCode::Up, KeyCode::Right, KeyCode::Down], KeyCode::Space),
            ([KeyCode::A, KeyCode::W, KeyCode::D, KeyCode::S], KeyCode::LeftShift),
        ];
        let humans = match self.mode {
            Mode::VsAi => 1,
            Mode::TwoPlayers => 2,
            Mode::Demo => 0,
        };
        for (pi, (keys, bomb)) in maps.iter().enumerate().take(humans) {
            apply_keys(&mut self.m.world.players[pi], *keys, *bomb);
        }
    }
}

/// Keyboard -> the original `onKeyDown/onKeyUp` handlers for one penguin.
fn apply_keys(p: &mut Player, keys: [KeyCode; 4], bomb: KeyCode) {
    let dirs = [Dir::Left, Dir::Up, Dir::Right, Dir::Down];
    for (k, d) in keys.iter().zip(dirs) {
        if is_key_pressed(*k) {
            p.key_down(d);
        }
    }
    if is_key_pressed(bomb) {
        p.key_space_down();
    }
    if keys.iter().any(|k| is_key_released(*k)) || is_key_released(bomb) {
        p.keys_released(keys.map(is_key_down), is_key_down(bomb));
    }
}

// ---- drawing ---------------------------------------------------------------------------------------------------

fn text(s: &str, x: f32, y: f32, size: f32, color: Color) {
    draw_text(s, x, y, size, color);
}

fn text_centered(s: &str, cx: f32, y: f32, size: f32, color: Color) {
    let d = measure_text(s, None, size as u16, 1.0);
    draw_text(s, (cx - d.width / 2.0).round(), y, size, color);
}

fn draw_player(a: &Assets, p: &Player) {
    let pg = &a.pengu[p.id];
    let t = p.anim_ticks.saturating_sub(1) as usize;
    let (strip, frame) = match p.anim {
        Anim::Stand => (&pg.stand, 0),
        Anim::Go => (&pg.go, t),
        Anim::Up => (&pg.up, t),
        Anim::Down => (&pg.down, t),
        Anim::Dead => (&pg.dead, 0),
    };
    strip.draw_ex(frame, p.x as f32, p.y as f32, p.facing_left, 0.0);
}

fn draw_hud(a: &Assets, w: &World, names: [&str; 2], hint: &str) {
    for (i, p) in w.players.iter().enumerate() {
        let y0 = if i == 0 { 48.0 } else { 213.0 };
        text(names[i], 8.0, y0, 18.0, INK);
        text(&format!("Bomben  {}", p.num_bombs), 12.0, y0 + 36.0, 16.0, INK);
        text(&format!("Flamme  {}", p.num_blast), 12.0, y0 + 54.0, 16.0, INK);
        text(&format!("Tempo   {:.0}", p.speed * 1000.0), 12.0, y0 + 72.0, 16.0, INK);
        text(&format!("Kick    {}", if p.kick { "ja" } else { "-" }), 12.0, y0 + 90.0, 16.0, INK);
    }
    // SoundButton: frame 1 = on, frame 2 = off, placed at (528.2, 470.4) scaled 0.8
    a.sound_btn.draw(if a.sound_on { 0 } else { 1 }, 528.2, 470.4);
    text(hint, 8.0, 632.0, 16.0, INK);
}

enum Fx<'a> {
    Bomb(&'a bomberpengu::world::Bomb),
    Flame(&'a bomberpengu::world::Flame),
}

fn draw_game(a: &Assets, w: &World, names: [&str; 2], hint: &str) {
    draw_texture(&a.bg_game, 0.0, 0.0, WHITE);
    let (bx, by) = (BOARD_X as f32, BOARD_Y as f32);
    for cy in 0..ROWS {
        for cx in 0..COLS {
            let cell = w.cells[cy][cx];
            let (x, y) = (bx + cell_center(cx as i32) as f32, by + cell_center(cy as i32) as f32);
            match cell.kind {
                Kind::Solid => a.stein.draw(0, x, y),
                Kind::Crate => a.boxes.draw(0, x, y),
                Kind::ItemBomb | Kind::ItemSpeed | Kind::ItemBlast | Kind::ItemKick => {
                    // Box clip: frame 1 = closed box, frame = sType once opened (sType == enum discriminant)
                    a.boxes.draw(if cell.open { cell.kind as usize - 1 } else { 0 }, x, y)
                }
                _ => {}
            }
        }
    }
    // bombs and flames share the grid clip: later creation = higher depth
    let mut fx: Vec<(u32, Fx)> = w.bombs.iter().map(|b| (b.uid, Fx::Bomb(b))).collect();
    fx.extend(w.flames.iter().map(|f| (f.uid, Fx::Flame(f))));
    fx.sort_by_key(|(uid, _)| *uid);
    for (_, e) in fx {
        match e {
            Fx::Bomb(b) => {
                let frame = ((w.now - b.start).max(0) / DT_MS) as usize;
                a.bombs[b.owner].draw(frame, bx + b.x as f32, by + b.y as f32);
            }
            Fx::Flame(f) => {
                let frame = (((w.now - f.start).max(0) / DT_MS) as usize).min(19);
                let (x, y) = (bx + cell_center(f.cx) as f32, by + cell_center(f.cy) as f32);
                if f.center {
                    a.flame_mid[f.owner].draw(frame, x, y);
                } else {
                    a.flame_arm[f.owner].draw_ex(frame, x, y, false, f.rot as f32);
                }
            }
        }
    }
    // players sit on the root timeline above the grid clip
    for p in &w.players {
        draw_player(a, p);
    }
    draw_hud(a, w, names, hint);
}

fn button_rect(a: &assets::Strip, x: f32, y: f32) -> Rect {
    let t = &a.frames[0];
    Rect::new(x + a.ox, y + a.oy, t.width(), t.height())
}

fn clicked(r: Rect) -> bool {
    is_mouse_button_pressed(MouseButton::Left) && r.contains(vec2(mouse_position().0, mouse_position().1))
}

const START_POS: (f32, f32) = (386.6, 453.8);
const AGAIN_POS: (f32, f32) = (130.7, 417.95);
const MENU_POS: (f32, f32) = (439.75, 372.2);

fn draw_title(a: &Assets) {
    draw_texture(&a.bg_title, 0.0, 0.0, WHITE);
    text_centered("Pinguin Bomberman", 320.0, 70.0, 56.0, INK);
    // left panel (x 18..310): item legend, right panel (x 325..620): controls
    text("Items", 32.0, 125.0, 30.0, INK);
    let items = ["Bombe: eine Bombe mehr", "Tempo: schneller laufen", "Flamme: groessere Explosion", "Kick: Bomben wegschieben"];
    for (i, l) in items.iter().enumerate() {
        let y = 160.0 + i as f32 * 62.0;
        a.boxes.draw(2 + i, 58.0, y + 20.0);
        text(l, 88.0, y + 26.0, 18.0, INK);
    }
    text("Steuerung", 340.0, 185.0, 30.0, INK);
    let lines = [
        "Spieler 1",
        "  Pfeiltasten: laufen",
        "  Leertaste: Bombe",
        "Spieler 2 (2-Spieler-Modus)",
        "  W A S D: laufen",
        "  Linke Shift: Bombe",
        "Wer in die Flammen laeuft, verliert.",
    ];
    for (i, l) in lines.iter().enumerate() {
        text(l, 340.0, 222.0 + i as f32 * 26.0, 18.0, INK);
    }
    a.start.draw(0, START_POS.0, START_POS.1);
    text("Enter / Klick: gegen Computer", 330.0, 500.0, 20.0, INK);
    text("2: zwei Spieler an einer Tastatur", 330.0, 524.0, 20.0, INK);
    text("O: online spielen (Server noetig)", 330.0, 548.0, 20.0, INK);
    a.sound_btn.draw(if a.sound_on { 0 } else { 1 }, 528.2, 470.4);
}

fn draw_after(a: &Assets, mode: Mode, outcome: Outcome) {
    draw_texture(&a.bg_after, 0.0, 0.0, WHITE);
    draw_rectangle(60.0, 40.0, 520.0, 440.0, Color::new(0.93, 0.97, 1.0, 0.92));
    draw_rectangle_lines(60.0, 40.0, 520.0, 440.0, 4.0, INK);
    text_centered("Ergebnis", 320.0, 95.0, 40.0, INK);
    text_centered("Pinguin Bomberman", 320.0, 125.0, 22.0, INK);
    // AfterMatch.matchResult frames: "Untenschieden!" / "Gewonnen!!!" / "Leider verloren..." (original spelling)
    let (msg, who) = match (mode, outcome) {
        (_, Outcome::Draw) => ("Untenschieden!", ""),
        (Mode::VsAi, Outcome::Win(0)) => ("Gewonnen!!!", ""),
        (Mode::VsAi, Outcome::Win(_)) => ("Leider verloren...", ""),
        (Mode::TwoPlayers, Outcome::Win(i)) => ("Gewonnen!!!", if i == 0 { "Spieler 1" } else { "Spieler 2" }),
        (Mode::Demo, Outcome::Win(i)) => ("Gewonnen!!!", if i == 0 { "Computer 1" } else { "Computer 2" }),
    };
    text_centered(msg, 320.0, 210.0, 54.0, INK);
    if !who.is_empty() {
        text_centered(who, 320.0, 250.0, 28.0, INK);
    }
    text_centered("und jetzt? Entscheiden Sie...", 320.0, 330.0, 26.0, INK);
    a.play_again.draw(0, AGAIN_POS.0, AGAIN_POS.1);
    // the original button says "PlayerRoom" (online lobby); locally it returns to the title, so it gets its own label
    let r = button_rect(&a.to_menu, MENU_POS.0, MENU_POS.1);
    draw_rectangle(r.x, r.y, r.w, r.h, Color::new(0.75, 0.1, 0.05, 1.0));
    draw_rectangle_lines(r.x, r.y, r.w, r.h, 2.0, INK);
    text_centered("ZUM MENUE", r.x + r.w / 2.0, r.y + r.h - 8.0, 22.0, WHITE);
    text("Enter: Nochmal Spielen", 100.0, 468.0, 18.0, INK);
    text("Esc: Zurueck zum Menue", 380.0, 468.0, 18.0, INK);
}

// ---- main ------------------------------------------------------------------------------------------------------

struct Cli {
    mode: Option<Mode>,
    seed: Option<i64>,
    shot: Option<String>,
    shot_tick: u32,
    screen: Option<String>,
    online: bool,
    autoconnect: bool,
    challenge: Option<String>,
    accept: bool,
    shot_frames: u32,
    name: String,
    server: String,
}

fn cli() -> Cli {
    let args: Vec<String> = std::env::args().collect();
    let val = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned();
    Cli {
        mode: match val("--mode").as_deref() {
            Some("ai") => Some(Mode::VsAi),
            Some("2p") => Some(Mode::TwoPlayers),
            Some("demo") => Some(Mode::Demo),
            _ => None,
        },
        seed: val("--seed").and_then(|s| s.parse().ok()),
        shot: val("--shot"),
        shot_tick: val("--shot-tick").and_then(|s| s.parse().ok()).unwrap_or(0),
        screen: val("--screen"),
        online: args.iter().any(|a| a == "--online"),
        autoconnect: args.iter().any(|a| a == "--connect"),
        challenge: val("--challenge"),
        accept: args.iter().any(|a| a == "--accept"),
        shot_frames: val("--shot-frames").and_then(|s| s.parse().ok()).unwrap_or(2),
        // original: "Gast" + Math.round(Math.random() * 100000) when no user name is given
        name: val("--name").unwrap_or_else(|| format!("Gast{}", rand::gen_range(0, 100_001))),
        server: val("--server").unwrap_or_else(|| format!("127.0.0.1:{}", bomberpengu::server::DEFAULT_PORT)),
    }
}

#[macroquad::main(window_conf)]
async fn main() {
    rand::srand(macroquad::miniquad::date::now() as u64);
    let mut assets = Assets::load().await;
    let cli = cli();
    let mut screen = match (cli.mode, cli.screen.as_deref()) {
        _ if cli.online => {
            let mut o = online::Online::new(&cli.name, &cli.server);
            o.auto_challenge = cli.challenge.clone();
            o.auto_accept = cli.accept;
            if cli.autoconnect {
                o.connect();
            }
            Screen::Online(Box::new(o))
        }
        (_, Some("title")) => Screen::Title,
        (_, Some("after")) => Screen::After { mode: Mode::VsAi, outcome: Outcome::Win(0) },
        (Some(mode), _) => Screen::Playing(Game::new(mode, cli.seed.unwrap_or_else(new_seed), &assets)),
        _ => Screen::Title,
    };
    if let (Screen::Playing(g), true) = (&mut screen, cli.shot.is_some()) {
        // deterministic screenshots: advance the simulation without waiting for real time
        for _ in 0..cli.shot_tick {
            g.tick(&assets);
        }
    }
    let mut acc = 0.0;
    let mut frames = 0;
    loop {
        // sound toggle (SoundButton)
        if clicked(Rect::new(528.2 + assets.sound_btn.ox * 0.8, 470.4 + assets.sound_btn.oy * 0.8, 40.0, 19.0)) {
            assets.sound_on = !assets.sound_on;
        }
        acc = (acc + get_frame_time() as f64).min(STEP * 5.0);
        let mut next: Option<Screen> = None;
        match &mut screen {
            Screen::Title => {
                if is_key_pressed(KeyCode::Enter) || clicked(button_rect(&assets.start, START_POS.0, START_POS.1)) {
                    next = Some(Screen::Playing(Game::new(Mode::VsAi, new_seed(), &assets)));
                } else if is_key_pressed(KeyCode::Key2) {
                    next = Some(Screen::Playing(Game::new(Mode::TwoPlayers, new_seed(), &assets)));
                } else if is_key_pressed(KeyCode::O) {
                    next = Some(Screen::Online(Box::new(online::Online::new(&cli.name, &cli.server))));
                } else if is_key_pressed(KeyCode::Escape) {
                    return;
                }
                clear_background(GRAY);
                draw_title(&assets);
            }
            Screen::Playing(g) => {
                g.input();
                if cli.shot.is_none() {
                    while acc >= STEP {
                        acc -= STEP;
                        g.tick(&assets);
                    }
                }
                if g.m.phase == Phase::Finished {
                    next = Some(Screen::After { mode: g.mode, outcome: g.m.outcome.unwrap_or(Outcome::Draw) });
                } else if is_key_pressed(KeyCode::Escape) {
                    next = Some(Screen::Title);
                }
                clear_background(GRAY);
                draw_game(&assets, &g.m.world, g.names(), "Esc: Menü");
            }
            Screen::After { mode, outcome } => {
                let mode = *mode;
                if is_key_pressed(KeyCode::Enter) || clicked(button_rect(&assets.play_again, AGAIN_POS.0, AGAIN_POS.1)) {
                    next = Some(Screen::Playing(Game::new(mode, new_seed(), &assets)));
                } else if is_key_pressed(KeyCode::Escape) || clicked(button_rect(&assets.to_menu, MENU_POS.0, MENU_POS.1)) {
                    next = Some(Screen::Title);
                }
                clear_background(GRAY);
                draw_after(&assets, mode, *outcome);
            }
            Screen::Online(o) => {
                clear_background(GRAY);
                if !o.frame(&assets, get_frame_time() as f64) {
                    next = Some(Screen::Title);
                }
            }
        }
        if let Some(n) = next {
            screen = n;
            acc = 0.0;
        }
        frames += 1;
        if let Some(path) = &cli.shot {
            if frames == cli.shot_frames {
                get_screen_data().export_png(path);
                return;
            }
        }
        next_frame().await;
    }
}
