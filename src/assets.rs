//! Loading of `assets/`.

use macroquad::audio::{load_sound, play_sound, PlaySoundParams, Sound};
use macroquad::prelude::*;
use serde_json::Value;

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
    pub start: Strip,
    pub play_again: Strip,
    pub to_menu: Strip,
    pub sound_btn: Strip,
    pub sounds: Sounds,
    pub sound_on: bool,
}

const ROOT: &str = "assets";

async fn tex(path: &str) -> Texture2D {
    let t = load_texture(&format!("{ROOT}/{path}")).await.unwrap_or_else(|e| panic!("cannot load {path}: {e}"));
    t.set_filter(FilterMode::Linear);
    t
}

async fn strip(meta: &Value, key: &str) -> Strip {
    let m = &meta[key];
    let files: Vec<String> = if let Some(arr) = m["frames"].as_array() {
        arr.iter().map(|v| v.as_str().unwrap().to_string()).collect()
    } else {
        vec![m["file"].as_str().unwrap_or_else(|| panic!("sprites.json: {key}")).to_string()]
    };
    strip_from(&files, m).await
}

async fn strip_from(files: &[String], m: &Value) -> Strip {
    let mut frames = Vec::new();
    for f in files {
        frames.push(tex(f).await);
    }
    Strip { frames, ox: m["ox"].as_f64().unwrap() as f32, oy: m["oy"].as_f64().unwrap() as f32 }
}

async fn pengu(meta: &Value, key: &str) -> Pengu {
    let m = &meta[key];
    let anim = |name: &str| -> Vec<String> { m[name].as_array().unwrap().iter().map(|v| v.as_str().unwrap().to_string()).collect() };
    Pengu {
        stand: strip_from(&anim("stand"), m).await,
        go: strip_from(&anim("go"), m).await,
        up: strip_from(&anim("up"), m).await,
        down: strip_from(&anim("down"), m).await,
        dead: strip_from(&anim("dead"), m).await,
    }
}

async fn snd(name: &str) -> Sound {
    load_sound(&format!("{ROOT}/sounds/{name}")).await.unwrap_or_else(|e| panic!("cannot load sound {name}: {e}"))
}

impl Assets {
    pub async fn load() -> Self {
        let text = std::fs::read_to_string(format!("{ROOT}/sprites.json")).expect("assets/sprites.json (run from the project directory)");
        let meta: Value = serde_json::from_str(&text).unwrap();
        Self {
            bg_game: tex("bg/game.png").await,
            bg_title: tex("bg/title.png").await,
            bg_after: tex("bg/after.png").await,
            stein: strip(&meta, "stein").await,
            boxes: strip(&meta, "box").await,
            bombs: [strip(&meta, "bomb1").await, strip(&meta, "bomb2").await],
            flame_mid: [strip(&meta, "flame1_mid").await, strip(&meta, "flame2_mid").await],
            flame_arm: [strip(&meta, "flame1_arm").await, strip(&meta, "flame2_arm").await],
            pengu: [pengu(&meta, "pengu1").await, pengu(&meta, "pengu2").await],
            start: strip(&meta, "ui_start").await,
            play_again: strip(&meta, "ui_play_again").await,
            to_menu: strip(&meta, "ui_to_menu").await,
            sound_btn: strip(&meta, "ui_sound").await,
            sounds: Sounds {
                donk: snd("donk.wav").await,
                explode: snd("explode1.wav").await,
                pling: snd("pling2.wav").await,
                landing: snd("landing.wav").await,
                music: snd("music_game.wav").await,
            },
            sound_on: true,
        }
    }

    /// `SoundPlayer.playV(name, 80)`
    pub fn play(&self, s: &Sound) {
        if self.sound_on {
            play_sound(s, PlaySoundParams { looped: false, volume: 0.8 });
        }
    }
}
