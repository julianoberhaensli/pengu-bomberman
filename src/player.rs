//! Port of the game-character state of `MyPlayer` (constants and key handling; movement lives in `world`).

use crate::grid::stage_center;

pub const MAX_SPEED: f64 = 0.18;
pub const SPEED_STEP: f64 = 0.01;
pub const START_SPEED: f64 = 0.15;
pub const START_BOMBS: u32 = 1;
pub const START_BLAST: u32 = 2;
/// Lookahead (px) for the flame check while walking.
pub const DIST_TO_DIE: f64 = 18.0;
pub const KICK_SPEED: f64 = 0.1;
pub const LEFT_BORDER: f64 = 120.0;
pub const RIGHT_BORDER: f64 = 600.0;
pub const UPPER_BORDER: f64 = 40.0;
pub const LOWER_BORDER: f64 = 440.0;

/// Approximate `hitTest` bounding box of the penguin clip relative to its position (inferred from sprite bounds).
pub const HIT_LEFT: f64 = -30.11;
pub const HIT_RIGHT: f64 = 26.59;
pub const HIT_TOP: f64 = -32.75;
pub const HIT_BOTTOM: f64 = 25.31;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Dir {
    Left,
    Up,
    Right,
    Down,
}

impl Dir {
    pub fn vec(self) -> (i32, i32) {
        match self {
            Dir::Left => (-1, 0),
            Dir::Up => (0, -1),
            Dir::Right => (1, 0),
            Dir::Down => (0, 1),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Anim {
    Stand,
    Go,
    Up,
    Down,
    Dead,
}

#[derive(Clone, Debug)]
pub struct Player {
    pub id: usize,
    /// Stage px, like the original `_x/_y`.
    pub x: f64,
    pub y: f64,
    pub speed: f64,
    pub num_bombs: u32,
    pub bombs_set: u32,
    pub num_blast: u32,
    pub kick: bool,
    pub left: bool,
    pub up: bool,
    pub right: bool,
    pub down: bool,
    pub space: bool,
    pub space_released: bool,
    /// Cell computed at the start of each update (`xxPos/yyPos`).
    pub cx: i32,
    pub cy: i32,
    pub dead: bool,
    pub facing_left: bool,
    pub anim: Anim,
    /// Ticks since the animation was (re)started.
    pub anim_ticks: u32,
}

impl Player {
    pub fn new(id: usize) -> Self {
        let (cx, cy) = if id == 0 { (0, 0) } else { (12, 10) };
        let (x, y) = stage_center(cx, cy);
        Self {
            id,
            x,
            y,
            speed: START_SPEED,
            num_bombs: START_BOMBS,
            bombs_set: 0,
            num_blast: START_BLAST,
            kick: false,
            left: false,
            up: false,
            right: false,
            down: false,
            space: false,
            space_released: true,
            cx,
            cy,
            dead: false,
            facing_left: id == 1,
            anim: Anim::Stand,
            anim_ticks: 0,
        }
    }

    fn set_anim(&mut self, a: Anim) {
        self.anim = a;
        self.anim_ticks = 0;
    }

    /// `Game.onKeyDown` for one arrow key: the last pressed direction wins and clears the others.
    /// Quirk kept: pressing Down while `up` is set is ignored (the original tests `up != true`).
    pub fn key_down(&mut self, dir: Dir) {
        match dir {
            Dir::Right if !self.right => {
                self.set_anim(Anim::Go);
                self.facing_left = false;
                self.set_flags(false, false, true, false);
            }
            Dir::Left if !self.left => {
                self.set_anim(Anim::Go);
                self.facing_left = true;
                self.set_flags(true, false, false, false);
            }
            Dir::Up if !self.up => {
                self.set_anim(Anim::Up);
                self.set_flags(false, true, false, false);
            }
            Dir::Down if !self.up => {
                self.set_anim(Anim::Down);
                self.set_flags(false, false, false, true);
            }
            _ => {}
        }
    }

    pub fn key_space_down(&mut self) {
        self.space = true;
    }

    /// `Game.onKeyUp`: clears flags of keys that are no longer held; never re-sets any flag.
    pub fn keys_released(&mut self, held: [bool; 4], space_held: bool) {
        let [l, u, r, d] = held;
        if !r {
            self.right = false;
        }
        if !l {
            self.left = false;
        }
        if !u {
            self.up = false;
        }
        if !d {
            self.down = false;
        }
        if !space_held {
            self.space = false;
            self.space_released = true;
        }
        if !l && !u && !r && !d {
            self.set_anim(Anim::Stand);
        }
    }

    /// Direct control used by the AI: exactly the given direction (or none) is active.
    pub fn set_direction(&mut self, dir: Option<Dir>) {
        let cur = self.direction();
        if cur == dir {
            return;
        }
        match dir {
            None => {
                self.set_flags(false, false, false, false);
                self.set_anim(Anim::Stand);
            }
            Some(Dir::Left) => {
                self.facing_left = true;
                self.set_flags(true, false, false, false);
                self.set_anim(Anim::Go);
            }
            Some(Dir::Right) => {
                self.facing_left = false;
                self.set_flags(false, false, true, false);
                self.set_anim(Anim::Go);
            }
            Some(Dir::Up) => {
                self.set_flags(false, true, false, false);
                self.set_anim(Anim::Up);
            }
            Some(Dir::Down) => {
                self.set_flags(false, false, false, true);
                self.set_anim(Anim::Down);
            }
        }
    }

    pub fn direction(&self) -> Option<Dir> {
        if self.right {
            Some(Dir::Right)
        } else if self.left {
            Some(Dir::Left)
        } else if self.up {
            Some(Dir::Up)
        } else if self.down {
            Some(Dir::Down)
        } else {
            None
        }
    }

    fn set_flags(&mut self, left: bool, up: bool, right: bool, down: bool) {
        self.left = left;
        self.up = up;
        self.right = right;
        self.down = down;
    }

    pub fn die(&mut self) {
        self.dead = true;
        self.set_anim(Anim::Dead);
    }

    /// `getDxDy`: distance per update, capped at 20 px.
    pub fn step_distance(&self, diff_ms: f64) -> f64 {
        (self.speed * diff_ms).min(20.0)
    }

    /// Bounding-box point test (`MovieClip.hitTest(x, y, false)`), stage px.
    pub fn hit_test(&self, px: f64, py: f64) -> bool {
        px >= self.x + HIT_LEFT && px <= self.x + HIT_RIGHT && py >= self.y + HIT_TOP && py <= self.y + HIT_BOTTOM
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn start_values_match_myplayer_prototype() {
        let p = Player::new(0);
        assert_eq!(p.speed, 0.15);
        assert_eq!(p.num_bombs, 1);
        assert_eq!(p.num_blast, 2);
        assert!(!p.kick);
        assert_eq!((p.x, p.y), (120.0, 40.0));
        let q = Player::new(1);
        assert_eq!((q.x, q.y), (600.0, 440.0));
    }

    #[test]
    fn step_distance_is_speed_times_diff_capped() {
        let p = Player::new(0);
        assert!((p.step_distance(25.0) - 3.75).abs() < 1e-12);
        assert_eq!(p.step_distance(1000.0), 20.0);
    }

    #[test]
    fn last_pressed_direction_wins_and_release_stops() {
        let mut p = Player::new(0);
        p.key_down(Dir::Right);
        assert_eq!(p.direction(), Some(Dir::Right));
        p.key_down(Dir::Up);
        assert_eq!(p.direction(), Some(Dir::Up));
        // right is still held on the keyboard but is not re-activated when up is released
        p.keys_released([false, false, true, false], false);
        assert_eq!(p.direction(), None);
        assert_eq!(p.anim, Anim::Up);
        p.keys_released([false; 4], false);
        assert_eq!(p.anim, Anim::Stand);
    }

    #[test]
    fn down_is_ignored_while_up_is_active() {
        let mut p = Player::new(0);
        p.key_down(Dir::Up);
        p.key_down(Dir::Down);
        assert_eq!(p.direction(), Some(Dir::Up));
    }

    #[test]
    fn space_fires_once_per_press() {
        let mut p = Player::new(0);
        assert!(p.space_released);
        p.key_space_down();
        assert!(p.space && p.space_released);
        p.keys_released([false; 4], true); // still held: nothing changes
        assert!(p.space);
        p.keys_released([false; 4], false);
        assert!(!p.space && p.space_released);
    }
}
