//! Match flow (port of `Game.onEnterFrame` state machine): intro delay, play, 3 s death delay, result.

use crate::world::World;

/// `Game.frame == 20` ends the init phase (0.5 s).
pub const INIT_FRAMES: u32 = 20;
/// `Game.gOBound`: frames of the `GAME_OVER` state before the result screen (3 s).
pub const GAME_OVER_FRAMES: u32 = 120;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Phase {
    Init,
    Playing,
    GameOver,
    Finished,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Outcome {
    Win(usize),
    Draw,
}

#[derive(Clone, Debug)]
pub struct Match {
    pub world: World,
    pub phase: Phase,
    pub frame: u32,
    pub over_counter: u32,
    pub outcome: Option<Outcome>,
}

impl Match {
    pub fn new(seed: i64) -> Self {
        Self { world: World::new(seed), phase: Phase::Init, frame: 0, over_counter: 0, outcome: None }
    }

    pub fn with_world(world: World) -> Self {
        Self { world, phase: Phase::Init, frame: 0, over_counter: 0, outcome: None }
    }

    /// One 25 ms frame. Players only act in `Playing`; bombs and flames run in `Playing` and `GameOver`.
    pub fn tick(&mut self) {
        self.frame += 1;
        match self.phase {
            Phase::Playing => {
                self.world.update_players();
                self.world.update_objects();
                self.world.now += crate::world::DT_MS;
                if self.world.locked {
                    self.phase = Phase::GameOver;
                }
            }
            Phase::GameOver => {
                self.world.advance_death_anims();
                self.world.update_objects();
                self.world.now += crate::world::DT_MS;
                self.over_counter += 1;
                if self.over_counter > GAME_OVER_FRAMES {
                    self.outcome = Some(self.result());
                    self.phase = Phase::Finished;
                }
            }
            Phase::Init => {
                if self.frame == INIT_FRAMES {
                    self.phase = Phase::Playing;
                }
            }
            Phase::Finished => {}
        }
    }

    fn result(&self) -> Outcome {
        let p = &self.world.players;
        match (p[0].dead, p[1].dead) {
            (true, true) => Outcome::Draw,
            (true, false) => Outcome::Win(1),
            (false, true) => Outcome::Win(0),
            (false, false) => Outcome::Draw,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::grid::base_level;
    use crate::player::Dir;

    fn started() -> Match {
        let mut m = Match::with_world(World::with_cells(base_level()));
        for _ in 0..INIT_FRAMES {
            m.tick();
        }
        assert_eq!(m.phase, Phase::Playing);
        m
    }

    #[test]
    fn init_lasts_twenty_frames_and_nobody_moves_meanwhile() {
        let mut m = Match::with_world(World::with_cells(base_level()));
        m.world.players[0].key_down(Dir::Right);
        for _ in 0..19 {
            m.tick();
            assert_eq!(m.phase, Phase::Init);
        }
        m.tick();
        assert_eq!(m.phase, Phase::Playing);
        assert_eq!(m.world.players[0].x, 120.0);
        m.tick();
        assert!(m.world.players[0].x > 120.0);
    }

    #[test]
    fn game_over_waits_121_frames_then_finishes() {
        let mut m = started();
        m.world.kill(1);
        m.tick(); // detects the lock
        assert_eq!(m.phase, Phase::GameOver);
        for _ in 0..GAME_OVER_FRAMES {
            m.tick();
            assert_eq!(m.phase, Phase::GameOver);
        }
        m.tick();
        assert_eq!(m.phase, Phase::Finished);
        assert_eq!(m.outcome, Some(Outcome::Win(0)));
    }

    #[test]
    fn players_are_frozen_in_game_over_but_bombs_keep_burning() {
        let mut m = started();
        m.world.players[0].x = 400.0;
        m.world.set_bomb(8, 4, 0);
        m.world.bombs[0].start = -1500; // 2 s of fuse left
        m.world.players[0].bombs_set = 1;
        m.world.kill(1);
        m.tick();
        m.world.players[0].key_down(Dir::Right);
        let x = m.world.players[0].x;
        for _ in 0..40 {
            m.tick();
        }
        assert_eq!(m.world.players[0].x, x);
        for _ in 0..110 {
            m.tick();
        }
        assert!(m.world.bombs.is_empty(), "the bomb exploded during the 3 s delay");
    }

    #[test]
    fn simultaneous_deaths_are_a_draw() {
        let mut m = started();
        m.world.players[0].cx = 6;
        m.world.players[0].cy = 4;
        m.world.players[1].cx = 7;
        m.world.players[1].cy = 4;
        m.world.set_bomb(6, 4, 0);
        let i = m.world.bomb_index_at(6, 4).unwrap();
        m.world.explode(i);
        for _ in 0..(GAME_OVER_FRAMES + 3) {
            m.tick();
        }
        assert_eq!(m.outcome, Some(Outcome::Draw));
    }
}
