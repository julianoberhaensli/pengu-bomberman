//! Simple computer opponent. The original game has none (it is online-only), so this is new code that
//! drives a player through the same controls a human uses.

use crate::grid::*;
use crate::player::Dir;
use crate::world::World;
use std::collections::VecDeque;

type Pos = (i32, i32);

const DIRS: [Dir; 4] = [Dir::Right, Dir::Left, Dir::Down, Dir::Up];

impl World {
    /// Cells a bomb at `(cx, cy)` with `range` tiles per arm would burn, using the same stopping rules as `explode`.
    /// With `pessimistic`, open items do not stop the blast: somebody may pick them up before the bomb goes off.
    pub fn blast_cells(&self, cx: i32, cy: i32, range: i32, pessimistic: bool) -> Vec<Pos> {
        let mut out = vec![(cx, cy)];
        for (vx, vy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
            for k in 1..=range {
                let (x, y) = (cx + vx * k, cy + vy * k);
                if !in_bounds(x, y) {
                    break;
                }
                let cell = self.cells[y as usize][x as usize];
                match cell.kind {
                    Kind::Free => out.push((x, y)),
                    Kind::Solid | Kind::ArmedBomb => break,
                    _ if pessimistic && cell.open => out.push((x, y)),
                    _ => {
                        out.push((x, y));
                        break;
                    }
                }
            }
        }
        out
    }

    /// Every cell that is burning or will burn when the bombs on the board go off (chains included).
    pub fn danger_map(&self, extra: Option<(Pos, i32)>) -> [[bool; COLS]; ROWS] {
        let mut danger = [[false; COLS]; ROWS];
        for y in 0..ROWS {
            for x in 0..COLS {
                danger[y][x] = self.flame_map[y][x].is_some();
            }
        }
        let mut bombs: Vec<(Pos, i32)> = self
            .bombs
            .iter()
            .filter(|b| !b.exploded)
            .map(|b| ((b.cx, b.cy), self.players[b.owner].num_blast as i32))
            .collect();
        if let Some(e) = extra {
            bombs.push(e);
        }
        let mut done = vec![false; bombs.len()];
        let mut changed = true;
        while changed {
            changed = false;
            for i in 0..bombs.len() {
                let ((bx, by), range) = bombs[i];
                if done[i] {
                    continue;
                }
                // bombs are always dangerous; their blast is added once
                done[i] = true;
                changed = true;
                for (x, y) in self.blast_cells(bx, by, range, true) {
                    danger[y as usize][x as usize] = true;
                }
            }
        }
        danger
    }
}

#[derive(Clone, Debug, Default)]
pub struct Ai {
    pending_release: bool,
}

fn passable(world: &World, p: Pos, danger: &[[bool; COLS]; ROWS], blocked: Option<Pos>) -> bool {
    in_bounds(p.0, p.1)
        && Some(p) != blocked
        && world.cells[p.1 as usize][p.0 as usize].can_enter()
        && !danger[p.1 as usize][p.0 as usize]
}

/// Breadth-first search from `start`; returns the first step towards the nearest cell accepted by `goal`.
fn bfs(
    world: &World,
    start: Pos,
    danger: &[[bool; COLS]; ROWS],
    blocked: Option<Pos>,
    goal: impl Fn(Pos) -> bool,
) -> Option<(Dir, usize)> {
    let mut first: [[Option<Dir>; COLS]; ROWS] = [[None; COLS]; ROWS];
    let mut dist = [[usize::MAX; COLS]; ROWS];
    let mut q = VecDeque::new();
    dist[start.1 as usize][start.0 as usize] = 0;
    q.push_back(start);
    while let Some(cur) = q.pop_front() {
        let d = dist[cur.1 as usize][cur.0 as usize];
        if cur != start && goal(cur) {
            return first[cur.1 as usize][cur.0 as usize].map(|f| (f, d));
        }
        for dir in DIRS {
            let (vx, vy) = dir.vec();
            let n = (cur.0 + vx, cur.1 + vy);
            if !in_bounds(n.0, n.1) || dist[n.1 as usize][n.0 as usize] != usize::MAX {
                continue;
            }
            if !passable(world, n, danger, blocked) {
                continue;
            }
            dist[n.1 as usize][n.0 as usize] = d + 1;
            first[n.1 as usize][n.0 as usize] = if cur == start { Some(dir) } else { first[cur.1 as usize][cur.0 as usize] };
            q.push_back(n);
        }
    }
    None
}

impl Ai {
    pub fn new() -> Self {
        Self::default()
    }

    /// Decide the controls for player `me` for the coming tick.
    pub fn control(&mut self, world: &mut World, me: usize) {
        if self.pending_release {
            world.players[me].space = false;
            world.players[me].space_released = true;
            self.pending_release = false;
        }
        if world.players[me].dead {
            return;
        }
        let (px, py) = (world.players[me].x, world.players[me].y);
        let here = (stage_x_to_cell(px), stage_y_to_cell(py));
        if !in_bounds(here.0, here.1) {
            return;
        }
        let danger = world.danger_map(None);
        let enemy = 1 - me;
        let enemy_cell = (stage_x_to_cell(world.players[enemy].x), stage_y_to_cell(world.players[enemy].y));

        // 1. get out of danger: the route may cross cells that will burn, only live flames are forbidden
        if danger[here.1 as usize][here.0 as usize] {
            let mut flames = [[false; COLS]; ROWS];
            for y in 0..ROWS {
                for x in 0..COLS {
                    flames[y][x] = world.flame_map[y][x].is_some();
                }
            }
            let step = bfs(world, here, &flames, None, |p| !danger[p.1 as usize][p.0 as usize]);
            world.players[me].set_direction(step.map(|s| s.0));
            return;
        }

        // 2. bomb if it pays off and we can run away afterwards
        let range = world.players[me].num_blast as i32;
        // one bomb at a time: several own bombs make the escape routes unpredictable
        let can_bomb = world.players[me].bombs_set == 0
            && world.cells[here.1 as usize][here.0 as usize].kind == Kind::Free;
        if can_bomb && self.bomb_is_useful(world, here, range, enemy_cell) && self.can_escape(world, here, range) {
            world.players[me].space = true;
            world.players[me].space_released = true;
            self.pending_release = true;
            return;
        }

        // 3. head for items, then for a good bombing spot, then for the enemy
        let step = bfs(world, here, &danger, None, |p| world.cells[p.1 as usize][p.0 as usize].is_item())
            .or_else(|| {
                bfs(world, here, &danger, None, |p| {
                    can_bomb_at(world, p, me) && self.bomb_is_useful(world, p, range, enemy_cell) && self.can_escape(world, p, range)
                })
            })
            .or_else(|| bfs(world, here, &danger, None, |p| p == enemy_cell || near(p, enemy_cell)));
        world.players[me].set_direction(step.map(|s| s.0));
    }

    fn bomb_is_useful(&self, world: &World, at: Pos, range: i32, enemy: Pos) -> bool {
        world.blast_cells(at.0, at.1, range, false).iter().any(|&(x, y)| {
            (x, y) == enemy
                || (!(x == at.0 && y == at.1)
                    && matches!(
                        world.cells[y as usize][x as usize].kind,
                        Kind::Crate | Kind::ItemBomb | Kind::ItemSpeed | Kind::ItemBlast | Kind::ItemKick
                    )
                    && !world.cells[y as usize][x as usize].open)
        })
    }

    /// Is there a safe cell reachable once a bomb with `range` has been placed on `at`?
    fn can_escape(&self, world: &World, at: Pos, range: i32) -> bool {
        let danger = world.danger_map(Some((at, range)));
        // the start cell itself is the bomb; leave it by walking to the nearest cell outside the new danger zone
        let mut seen = [[false; COLS]; ROWS];
        let mut q = VecDeque::new();
        q.push_back((at, 0));
        seen[at.1 as usize][at.0 as usize] = true;
        while let Some((cur, d)) = q.pop_front() {
            if !danger[cur.1 as usize][cur.0 as usize] {
                return true;
            }
            if d >= 7 {
                continue; // 7 tiles at 150 px/s are covered long before the 3.5 s fuse ends
            }
            for dir in DIRS {
                let (vx, vy) = dir.vec();
                let n = (cur.0 + vx, cur.1 + vy);
                if !in_bounds(n.0, n.1) || seen[n.1 as usize][n.0 as usize] {
                    continue;
                }
                let cell = world.cells[n.1 as usize][n.0 as usize];
                if !cell.can_enter() || world.flame_map[n.1 as usize][n.0 as usize].is_some() {
                    continue;
                }
                seen[n.1 as usize][n.0 as usize] = true;
                q.push_back((n, d + 1));
            }
        }
        false
    }
}

fn can_bomb_at(world: &World, p: Pos, _me: usize) -> bool {
    world.cells[p.1 as usize][p.0 as usize].kind == Kind::Free
}

fn near(a: Pos, b: Pos) -> bool {
    (a.0 - b.0).abs() + (a.1 - b.1).abs() <= 2
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::player::Anim;

    fn run(world: &mut World, ai: &mut Ai, ticks: usize) {
        for _ in 0..ticks {
            ai.control(world, 1);
            world.tick();
        }
    }

    #[test]
    fn blast_cells_follow_explosion_rules() {
        let mut w = World::with_cells(base_level());
        w.cells[4][7] = Cell { kind: Kind::Crate, open: false };
        let mut c = w.blast_cells(6, 4, 2, false);
        c.sort();
        assert_eq!(c, vec![(4, 4), (5, 4), (6, 2), (6, 3), (6, 4), (6, 5), (6, 6), (7, 4)]);
    }

    #[test]
    fn runs_away_from_a_bomb_it_stands_next_to() {
        let mut w = World::with_cells(base_level());
        w.players[1].x = 440.0;
        w.players[1].y = 200.0; // cell (8,4), the bomb's own cell
        w.set_bomb(8, 4, 0);
        w.players[0].bombs_set = 1;
        let mut ai = Ai::new();
        run(&mut w, &mut ai, 120);
        assert!(!w.players[1].dead, "the AI must leave the blast area before the fuse ends");
    }

    #[test]
    fn computer_vs_computer_matches_reach_a_result() {
        use crate::game::{Match, Phase};
        let mut finished = 0;
        for seed in [1, 2, 3, 4, 5, 6] {
            let mut m = Match::new(seed * 1111);
            let (mut a, mut b) = (Ai::new(), Ai::new());
            for _ in 0..20_000 {
                if m.phase == Phase::Playing {
                    a.control(&mut m.world, 0);
                    b.control(&mut m.world, 1);
                }
                m.tick();
                if m.phase == Phase::Finished {
                    finished += 1;
                    assert!(m.outcome.is_some());
                    break;
                }
            }
        }
        assert!(finished >= 4, "only {finished} of 6 matches ended");
    }

    #[test]
    fn lone_ai_survives_most_games() {
        // The human is parked and revived every tick so that only the AI's own bombs can end the game.
        let (mut deaths, games) = (0, 30);
        for seed in 0..games {
            let mut w = World::new(seed * 313 + 7);
            let mut ai = Ai::new();
            for _ in 0..2500 {
                ai.control(&mut w, 1);
                w.tick();
                if w.players[1].dead {
                    deaths += 1;
                    break;
                }
                w.players[0].dead = false;
                w.locked = false;
                w.players[0].x = 120.0;
                w.players[0].y = 40.0;
            }
        }
        assert!(deaths * 5 <= games, "the AI blew itself up in {deaths} of {games} games");
    }

    #[test]
    fn bombs_crates_and_survives_alone() {
        let mut w = World::new(5);
        w.players[0].dead = false;
        let mut ai = Ai::new();
        let mut placed = false;
        for _ in 0..600 {
            ai.control(&mut w, 1);
            w.tick();
            if w.players[1].dead {
                break;
            }
            if w.players[1].bombs_set > 0 {
                placed = true;
            }
            // keep the human out of the picture
            w.players[0].x = 120.0;
            w.players[0].y = 40.0;
            w.players[0].anim = Anim::Stand;
        }
        assert!(placed, "the AI should have placed at least one bomb");
        assert!(!w.players[1].dead, "the AI must not blow itself up");
    }
}


