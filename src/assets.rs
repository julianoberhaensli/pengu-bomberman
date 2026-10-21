//! Loading of `assets/`.

use macroquad::audio::{load_sound, play_sound, PlaySoundParams, Sound};
use macroquad::prelude::*;
use serde_json::Value;
use std::cell::Cell;

/// A frame list with the registration offset of the original clip (frame top-left = origin + (ox, oy)).
pub struct Strip {
    pub frames: Vec<Texture2D>,
    pub ox: f32,
    pub oy: f32,
}

impl Strip {
    pub fn draw(&self, frame: usize, x: f32, y: f32) {
        draw_texture(&self.frames[frame % self.frames.len()], (x + self.ox).round(), (y + self.oy).round(), WHITE);
    }

    pub fn draw_ex(&self, frame: usize, x: f32, y: f32, flip_x: bool, rotation_deg: f32) {
        let t = &self.frames[frame % self.frames.len()];
        let (w, h) = (t.width(), t.height());
        let x0 = if flip_x { x - (self.ox + w) } else { x + self.ox };
        let params = DrawTextureParams {
            flip_x,
            rotation: rotation_deg.to_radians(),
            pivot: Some(vec2(x, y)),
            dest_size: Some(vec2(w, h)),
            ..Default::default()
        };
        draw_texture_ex(t, x0, y + self.oy, WHITE, params);
    }
}

pub struct Pengu {
    pub stand: Strip,
    pub go: Strip,
    pub up: Strip,
    pub down: Strip,
    pub dead: Strip,
}

pub struct Sounds {
    pub donk: Sound,
    pub explode: Sound,
    pub pling: Sound,
    pub landing: Sound,
    pub music: Sound,
}

pub struct Assets {
    pub bg_game: Texture2D,
    pub bg_title: Texture2D,
    pub bg_after: Texture2D,
    pub stein: Strip,
    pub boxes: Strip,
    pub bombs: [Strip; 2],
    pub flame_mid: [Strip; 2],
    pub flame_arm: [Strip; 2],
    pub pengu: [Pengu; 2],
    pub sounds: Sounds,
    pub sound_on: Cell<bool>,
}

const ROOT: &str = "assets";
/// Files loaded besides the ones listed in `sprites.json`: 3 backgrounds and 5 sounds.
const EXTRA_FILES: usize = 8;

/// Counts files while loading and redraws the loading screen (about 30 times per second).
struct Loader {
    done: usize,
    total: usize,
    last_draw: f64,
}

impl Loader {
    async fn frame(&mut self, force: bool) {
        let now = get_time();
        if force || now - self.last_draw >= 1.0 / 30.0 {
            self.last_draw = now;
            crate::ui::draw_loading(self.done, self.total);
            next_frame().await;
        }
    }

    async fn tex(&mut self, path: &str) -> Texture2D {
        let t = load_texture(&format!("{ROOT}/{path}")).await.unwrap_or_else(|e| panic!("cannot load {path}: {e}"));
        t.set_filter(FilterMode::Linear);
        self.done += 1;
        self.frame(false).await;
        t
    }

    async fn snd(&mut self, name: &str) -> Sound {
        let s = load_sound(&format!("{ROOT}/sounds/{name}")).await.unwrap_or_else(|e| panic!("cannot load sound {name}: {e}"));
        self.done += 1;
        self.frame(false).await;
        s
    }

    async fn strip(&mut self, meta: &Value, key: &str) -> Strip {
        let m = &meta[key];
        let files: Vec<String> = if let Some(arr) = m["frames"].as_array() {
            arr.iter().map(|v| v.as_str().unwrap().to_string()).collect()
        } else {
            vec![m["file"].as_str().unwrap_or_else(|| panic!("sprites.json: {key}")).to_string()]
        };
        self.strip_from(&files, m).await
    }

    async fn strip_from(&mut self, files: &[String], m: &Value) -> Strip {
        let mut frames = Vec::new();
        for f in files {
            frames.push(self.tex(f).await);
        }
        Strip { frames, ox: m["ox"].as_f64().unwrap() as f32, oy: m["oy"].as_f64().unwrap() as f32 }
    }

    async fn pengu(&mut self, meta: &Value, key: &str) -> Pengu {
        let m = &meta[key];
        let anim = |name: &str| -> Vec<String> { m[name].as_array().unwrap().iter().map(|v| v.as_str().unwrap().to_string()).collect() };
        Pengu {
            stand: self.strip_from(&anim("stand"), m).await,
            go: self.strip_from(&anim("go"), m).await,
            up: self.strip_from(&anim("up"), m).await,
            down: self.strip_from(&anim("down"), m).await,
            dead: self.strip_from(&anim("dead"), m).await,
        }
    }
}

/// Number of image files `sprites.json` refers to.
fn count_files(v: &Value) -> usize {
    match v {
        Value::String(s) => usize::from(s.ends_with(".png")),
        Value::Array(a) => a.iter().map(count_files).sum(),
        Value::Object(o) => o.values().map(count_files).sum(),
        _ => 0,
    }
}

impl Assets {
    pub async fn load() -> Self {
        let mut l = Loader { done: 0, total: 0, last_draw: f64::NEG_INFINITY };
        l.frame(true).await;
        let text = macroquad::file::load_string(&format!("{ROOT}/sprites.json")).await.expect("assets/sprites.json (run from the project directory)");
        let meta: Value = serde_json::from_str(&text).unwrap();
        l.total = count_files(&meta) + EXTRA_FILES;
        let a = Self {
            bg_game: l.tex("bg/game.png").await,
            bg_title: l.tex("bg/title.png").await,
            bg_after: l.tex("bg/after.png").await,
            stein: l.strip(&meta, "stein").await,
            boxes: l.strip(&meta, "box").await,
            bombs: [l.strip(&meta, "bomb1").await, l.strip(&meta, "bomb2").await],
            flame_mid: [l.strip(&meta, "flame1_mid").await, l.strip(&meta, "flame2_mid").await],
            flame_arm: [l.strip(&meta, "flame1_arm").await, l.strip(&meta, "flame2_arm").await],
            pengu: [l.pengu(&meta, "pengu1").await, l.pengu(&meta, "pengu2").await],
            sounds: Sounds {
                donk: l.snd("donk.wav").await,
                explode: l.snd("explode1.wav").await,
                pling: l.snd("pling2.wav").await,
                landing: l.snd("landing.wav").await,
                music: l.snd("music_game.wav").await,
            },
            sound_on: Cell::new(true),
        };
        l.frame(true).await;
        a
    }

    /// `SoundPlayer.playV(name, 80)`
    pub fn play(&self, s: &Sound) {
        if self.sound_on.get() {
            play_sound(s, PlaySoundParams { looped: false, volume: 0.8 });
        }
    }
}
