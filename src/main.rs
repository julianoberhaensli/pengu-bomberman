mod assets;
mod online;
mod ui;

use assets::Assets;
use bomberpengu::ai::Ai;
use bomberpengu::game::{Match, Outcome, Phase};
use bomberpengu::grid::*;
use bomberpengu::player::{Anim, Dir, Player};
use bomberpengu::proto::{valid_name, MAX_NAME_CHARS};
use bomberpengu::world::{Event, World, DT_MS};
use macroquad::prelude::*;
use ui::{text, text_centered, text_fit, Style, INK};

/// Flash frame rate of the original: 40 fps -> fixed 25 ms step.
const STEP: f64 = DT_MS as f64 / 1000.0;

/// Movement keys in the order left, up, right, down.
const P1_DIRS: [KeyCode; 4] = [KeyCode::A, KeyCode::W, KeyCode::D, KeyCode::S];
const P1_BOMB: KeyCode = KeyCode::Space;
const P2_DIRS: [KeyCode; 4] = [KeyCode::Left, KeyCode::Up, KeyCode::Right, KeyCode::Down];
const P2_BOMB: KeyCode = KeyCode::Enter;
const SOLO_HINT: &str = "WASD / Pfeile + Leertaste / Enter";

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
    names: [String; 2],
}

enum Screen {
    Title { focus: usize },
    Settings { name: ui::Field },
    Playing(Game),
    After { mode: Mode, outcome: Outcome, born: f64 },
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
    fn new(mode: Mode, seed: i64, assets: &Assets, player: &str) -> Self {
        let ai = match mode {
            Mode::VsAi => [None, Some(Ai::new())],
            Mode::TwoPlayers => [None, None],
            Mode::Demo => [Some(Ai::new()), Some(Ai::new())],
        };
        let names = match mode {
            Mode::VsAi => [player, "Computer"],
            Mode::TwoPlayers => [player, "Spieler 2"],
            Mode::Demo => ["Computer 1", "Computer 2"],
        };
        assets.play(&assets.sounds.music);
        Self { m: Match::new(seed), mode, ai, names: names.map(String::from) }
    }

    fn hint(&self) -> &'static str {
        match self.mode {
            Mode::TwoPlayers => "1: WASD + Leertaste   2: Pfeile + Enter",
            _ => SOLO_HINT,
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

    /// Two players share the keyboard (1: WASD + Space, 2: arrows + Enter); a single human may use either set.
    fn input(&mut self) {
        match self.mode {
            Mode::VsAi => apply_keys(&mut self.m.world.players[0], &[P1_DIRS, P2_DIRS], &[P1_BOMB, P2_BOMB]),
            Mode::TwoPlayers => {
                apply_keys(&mut self.m.world.players[0], &[P1_DIRS], &[P1_BOMB]);
                apply_keys(&mut self.m.world.players[1], &[P2_DIRS], &[P2_BOMB]);
            }
            Mode::Demo => {}
        }
    }
}

/// Keyboard -> the original `onKeyDown/onKeyUp` handlers for one penguin; any of the given key sets controls it.
fn apply_keys(p: &mut Player, dirs: &[[KeyCode; 4]], bombs: &[KeyCode]) {
    let order = [Dir::Left, Dir::Up, Dir::Right, Dir::Down];
    for set in dirs {
        for (k, d) in set.iter().zip(order) {
            if is_key_pressed(*k) {
                p.key_down(d);
            }
        }
    }
    if bombs.iter().any(|k| is_key_pressed(*k)) {
        p.key_space_down();
    }
    let released = dirs.iter().flatten().chain(bombs).any(|k| is_key_released(*k));
    if released {
        let held = [0, 1, 2, 3].map(|i| dirs.iter().any(|s| is_key_down(s[i])));
        p.keys_released(held, bombs.iter().any(|k| is_key_down(*k)));
    }
}

// ---- drawing ---------------------------------------------------------------------------------------------------

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

fn draw_hud(w: &World, names: [&str; 2]) {
    for (i, p) in w.players.iter().enumerate() {
        let y0 = if i == 0 { 48.0 } else { 213.0 };
        text_fit(names[i], 8.0, y0, 86.0, 18.0, INK);
        text(&format!("Bomben  {}", p.num_bombs), 12.0, y0 + 36.0, 16.0, INK);
        text(&format!("Flamme  {}", p.num_blast), 12.0, y0 + 54.0, 16.0, INK);
        text(&format!("Tempo   {:.0}", p.speed * 1000.0), 12.0, y0 + 72.0, 16.0, INK);
        text(&format!("Kick    {}", if p.kick { "ja" } else { "-" }), 12.0, y0 + 90.0, 16.0, INK);
    }
}

const SOUND_BTN: Rect = Rect { x: 376.0, y: 592.0, w: 118.0, h: 38.0 };
const LEAVE_BTN: Rect = Rect { x: 504.0, y: 592.0, w: 128.0, h: 38.0 };

/// Bottom bar of every game screen: control hint, sound toggle and the leave button. Returns `true` when leave was clicked.
fn game_bar(a: &Assets, hint: &str, leave: &str) -> bool {
    text_fit(hint, 8.0, 616.0, 360.0, 16.0, INK);
    if ui::button(SOUND_BTN, if a.sound_on.get() { "Ton: an" } else { "Ton: aus" }, Style::Normal, true) {
        a.sound_on.set(!a.sound_on.get());
    }
    ui::button(LEAVE_BTN, leave, Style::Normal, true)
}

enum Fx<'a> {
    Bomb(&'a bomberpengu::world::Bomb),
    Flame(&'a bomberpengu::world::Flame),
}

/// Draws the board, HUD and bottom bar; `true` = the leave button was clicked.
fn draw_game(a: &Assets, w: &World, names: [&str; 2], hint: &str, leave: &str) -> bool {
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
    draw_hud(w, names);
    game_bar(a, hint, leave)
}

const MENU: [&str; 4] = ["Gegen Computer", "Zwei Spieler", "Online-Lobby", "Einstellungen"];

/// The mode selection is a fixed 2x2 grid of equally styled buttons in the bottom panel of the title background.
fn menu_rect(i: usize) -> Rect {
    Rect::new(20.0 + (i % 2) as f32 * 308.0, 507.0 + (i / 2) as f32 * 56.0, 292.0, 46.0)
}

/// Draws the title screen; returns the menu entry that was clicked.
fn draw_title(a: &Assets, player: &str, focus: usize) -> Option<usize> {
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
    text("Spielername", 32.0, 418.0, 16.0, INK);
    text_fit(player, 32.0, 442.0, 262.0, 24.0, INK);
    text("Steuerung", 340.0, 185.0, 30.0, INK);
    let lines = [
        "Spieler 1",
        "  W A S D: laufen",
        "  Leertaste: Bombe",
        "Spieler 2 (Zwei Spieler)",
        "  Pfeiltasten: laufen",
        "  Enter: Bombe",
        "Allein: beide Tastensaetze",
        "Wer in die Flammen laeuft, verliert.",
    ];
    for (i, l) in lines.iter().enumerate() {
        text_fit(l, 340.0, 215.0 + i as f32 * 24.0, 270.0, 18.0, INK);
    }
    let mut hit = None;
    for (i, label) in MENU.iter().enumerate() {
        let r = menu_rect(i);
        ui::draw_button(r, label, if i == 0 { Style::Primary } else { Style::Normal }, true, i == focus);
        if ui::clicked(r) {
            hit = Some(i);
        }
    }
    text_centered("Pfeiltasten / Maus: waehlen    Enter: starten    1-4: Direktwahl", 320.0, 628.0, 16.0, INK);
    hit
}

const NAME_FIELD: Rect = Rect { x: 90.0, y: 222.0, w: 460.0, h: 40.0 };

pub fn name_field(name: &str) -> ui::Field {
    ui::Field::new(name, MAX_NAME_CHARS, ui::name_char)
}

/// Settings screen; `Some(true)` = apply the name, `Some(false)` = cancel.
fn frame_settings(a: &Assets, name: &mut ui::Field) -> Option<bool> {
    name.update(NAME_FIELD);
    let valid = valid_name(name.text.trim());
    draw_texture(&a.bg_after, 0.0, 0.0, WHITE);
    ui::panel(60.0, 110.0, 520.0, 370.0);
    text_centered("Einstellungen", 320.0, 165.0, 40.0, INK);
    text("Spielername", 90.0, 212.0, 20.0, INK);
    ui::draw_field(NAME_FIELD, name, true);
    if valid {
        text("Bis zu 20 Zeichen: Buchstaben, Ziffern, Leerzeichen, _ - .", 90.0, 288.0, 15.0, INK);
    } else {
        text("Bitte einen Namen eingeben.", 90.0, 288.0, 16.0, ui::ERROR);
    }
    text("Ton", 90.0, 335.0, 20.0, INK);
    if ui::button(Rect::new(90.0, 345.0, 200.0, 44.0), if a.sound_on.get() { "Ton: an" } else { "Ton: aus" }, Style::Normal, true) {
        a.sound_on.set(!a.sound_on.get());
    }
    let done = ui::button(Rect::new(250.0, 405.0, 140.0, 44.0), "Fertig", Style::Primary, valid);
    text_centered("Enter: uebernehmen    Esc: abbrechen", 320.0, 468.0, 16.0, INK);
    if (done || is_key_pressed(KeyCode::Enter)) && valid {
        Some(true)
    } else if is_key_pressed(KeyCode::Escape) {
        Some(false)
    } else {
        None
    }
}

/// Result screen; returns (play again, back to menu) clicks.
fn draw_after(a: &Assets, mode: Mode, outcome: Outcome, player: &str) -> (bool, bool) {
    draw_texture(&a.bg_after, 0.0, 0.0, WHITE);
    ui::panel(60.0, 40.0, 520.0, 440.0);
    text_centered("Ergebnis", 320.0, 95.0, 40.0, INK);
    text_centered("Pinguin Bomberman", 320.0, 125.0, 22.0, INK);
    // AfterMatch.matchResult frames: "Untenschieden!" / "Gewonnen!!!" / "Leider verloren..." (original spelling)
    let (msg, who) = match (mode, outcome) {
        (_, Outcome::Draw) => ("Untenschieden!", ""),
        (Mode::VsAi, Outcome::Win(0)) => ("Gewonnen!!!", ""),
        (Mode::VsAi, Outcome::Win(_)) => ("Leider verloren...", ""),
        (Mode::TwoPlayers, Outcome::Win(i)) => ("Gewonnen!!!", if i == 0 { player } else { "Spieler 2" }),
        (Mode::Demo, Outcome::Win(i)) => ("Gewonnen!!!", if i == 0 { "Computer 1" } else { "Computer 2" }),
    };
    text_centered(msg, 320.0, 210.0, 54.0, INK);
    if !who.is_empty() {
        text_centered(&ui::clip(who, MAX_NAME_CHARS), 320.0, 250.0, 28.0, INK);
    }
    text_centered("und jetzt? Entscheiden Sie...", 320.0, 330.0, 26.0, INK);
    text_centered("Enter: nochmal spielen    Esc: zurueck zum Menue", 320.0, 462.0, 16.0, INK);
    ui::after_buttons(true, "Zum Menue")
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
        server: val("--server").unwrap_or_else(bomberpengu::net::default_server),
    }
}

#[macroquad::main(window_conf)]
async fn main() {
    rand::srand(macroquad::miniquad::date::now() as u64);
    let assets = Assets::load().await;
    let cli = cli();
    let mut player_name = cli.name.clone();
    let mut screen = match (cli.mode, cli.screen.as_deref()) {
        _ if cli.online => {
            let mut o = online::Online::new(&player_name, &cli.server);
            o.auto_challenge = cli.challenge.clone();
            o.auto_accept = cli.accept;
            if cli.autoconnect {
                o.connect();
            }
            Screen::Online(Box::new(o))
        }
        (_, Some("title")) => Screen::Title { focus: 0 },
        (_, Some("settings")) => Screen::Settings { name: name_field(&player_name) },
        (_, Some("after")) => Screen::After { mode: Mode::VsAi, outcome: Outcome::Win(0), born: get_time() },
        (Some(mode), _) => Screen::Playing(Game::new(mode, cli.seed.unwrap_or_else(new_seed), &assets, &player_name)),
        _ => Screen::Title { focus: 0 },
    };
    if let (Screen::Playing(g), true) = (&mut screen, cli.shot.is_some()) {
        // deterministic screenshots: advance the simulation without waiting for real time
        for _ in 0..cli.shot_tick {
            g.tick(&assets);
        }
    }
    let mut acc = 0.0;
    let mut frames = 0;
    let mut last_mouse = mouse_position();
    loop {
        let mouse_moved = mouse_position() != last_mouse;
        last_mouse = mouse_position();
        acc = (acc + get_frame_time() as f64).min(STEP * 5.0);
        let mut next: Option<Screen> = None;
        match &mut screen {
            Screen::Title { focus } => {
                if is_key_pressed(KeyCode::Right) || is_key_pressed(KeyCode::Left) {
                    *focus ^= 1;
                }
                if is_key_pressed(KeyCode::Down) || is_key_pressed(KeyCode::Up) {
                    *focus ^= 2;
                }
                if mouse_moved {
                    if let Some(i) = (0..MENU.len()).find(|&i| ui::hovered(menu_rect(i))) {
                        *focus = i;
                    }
                }
                clear_background(GRAY);
                let mut pick = draw_title(&assets, &player_name, *focus);
                for (i, k) in [KeyCode::Key1, KeyCode::Key2, KeyCode::Key3, KeyCode::Key4].into_iter().enumerate() {
                    if is_key_pressed(k) {
                        pick = Some(i);
                    }
                }
                if is_key_pressed(KeyCode::Enter) || is_key_pressed(KeyCode::Space) {
                    pick = Some(*focus);
                }
                match pick {
                    Some(0) => next = Some(Screen::Playing(Game::new(Mode::VsAi, new_seed(), &assets, &player_name))),
                    Some(1) => next = Some(Screen::Playing(Game::new(Mode::TwoPlayers, new_seed(), &assets, &player_name))),
                    Some(2) => next = Some(Screen::Online(Box::new(online::Online::new(&player_name, &cli.server)))),
                    Some(_) => {
                        ui::drain_chars();
                        next = Some(Screen::Settings { name: name_field(&player_name) });
                    }
                    None => {
                        if cfg!(not(target_arch = "wasm32")) && is_key_pressed(KeyCode::Escape) {
                            return;
                        }
                    }
                }
            }
            Screen::Settings { name } => {
                if let Some(apply) = frame_settings(&assets, name) {
                    if apply {
                        player_name = name.text.trim().to_string();
                    }
                    next = Some(Screen::Title { focus: 3 });
                }
            }
            Screen::Playing(g) => {
                g.input();
                if cli.shot.is_none() {
                    while acc >= STEP {
                        acc -= STEP;
                        g.tick(&assets);
                    }
                }
                clear_background(GRAY);
                let names = [g.names[0].as_str(), g.names[1].as_str()];
                let leave = draw_game(&assets, &g.m.world, names, g.hint(), "Menue (Esc)");
                if g.m.phase == Phase::Finished {
                    next = Some(Screen::After { mode: g.mode, outcome: g.m.outcome.unwrap_or(Outcome::Draw), born: get_time() });
                } else if leave || is_key_pressed(KeyCode::Escape) {
                    next = Some(Screen::Title { focus: 0 });
                }
            }
            Screen::After { mode, outcome, born } => {
                let mode = *mode;
                clear_background(GRAY);
                let (again, menu) = draw_after(&assets, mode, *outcome, &player_name);
                // two players share the keyboard and Enter is player 2's bomb key: ignore it right after the match
                let enter = is_key_pressed(KeyCode::Enter) && get_time() - *born > 0.7;
                if again || enter {
                    next = Some(Screen::Playing(Game::new(mode, new_seed(), &assets, &player_name)));
                } else if menu || is_key_pressed(KeyCode::Escape) {
                    next = Some(Screen::Title { focus: 0 });
                }
            }
            Screen::Online(o) => {
                clear_background(GRAY);
                if !o.frame(&assets, get_frame_time() as f64) {
                    if valid_name(o.name()) {
                        player_name = o.name().to_string();
                    }
                    next = Some(Screen::Title { focus: 2 });
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
