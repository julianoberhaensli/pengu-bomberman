//! Game rules: movement, bombs, blasts, flames, pick-ups. Pure logic, no rendering.
//!
//! One `tick` is one Flash frame at 40 fps (25 ms). The original measures real milliseconds with `getTimer()`;
//! here time is the simulated clock `now`.

use crate::grid::*;
use crate::player::*;

pub const DT_MS: i64 = 25;
/// `Bomb.TIME_TO_EXPLODE`
pub const BOMB_FUSE_MS: i64 = 3500;
/// `Bliz.TIME_TO_HIDE`
pub const FLAME_MS: i64 = 500;

#[derive(Clone, Debug)]
pub struct Bomb {
    pub uid: u32,
    pub owner: usize,
    /// Grid-local px (centre).
    pub x: f64,
    pub y: f64,
    pub cx: i32,
    pub cy: i32,
    pub start: i64,
    pub moving: bool,
    pub dx: f64,
    pub dy: f64,
    pub kicked_by: Option<usize>,
    pub kicked: bool,
    pub exploded: bool,
    /// Stone that was in the cell the bomb currently occupies (restored when it moves on).
    pub old: Option<Cell>,
    /// `moveId`: assigned on the first kick, shared with the other client.
    pub id: Option<i64>,
    /// Kicked or stopped by the remote player: its detonation is announced by the other client, not by the fuse.
    pub remote_kicked: bool,
}

#[derive(Clone, Debug)]
pub struct Flame {
    pub uid: u32,
    pub cx: i32,
    pub cy: i32,
    pub owner: usize,
    pub start: i64,
    pub center: bool,
    /// Degrees: 90 for horizontal arms, 0 for vertical ones.
    pub rot: u16,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Event {
    BombPlaced,
    Explosion,
    PickUp,
    Kick,
}

/// Something the local player did that the other client has to be told about (only produced when `World::local` is set).
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Outgoing {
    Pickup { cx: i32, cy: i32 },
    BombPlaced { cx: i32, cy: i32 },
    Kick { cx: i32, cy: i32, id: i64, dx: f64, dy: f64, t: i64 },
    Stop { cx: i32, cy: i32, id: i64, t: i64 },
    Detonate { cx: i32, cy: i32, id: i64 },
    Die,
}

#[derive(Clone, Debug)]
pub struct World {
    pub cells: Cells,
    pub flame_map: [[Option<u32>; COLS]; ROWS],
    pub players: [Player; 2],
    pub bombs: Vec<Bomb>,
    pub flames: Vec<Flame>,
    pub now: i64,
    pub events: Vec<Event>,
    /// `Game.locked`: the first death freezes the result.
    pub locked: bool,
    lock_now: i64,
    next_uid: u32,
    /// Online play: index of the player this client controls (`Game.thisPlayer`). Pick-ups, kicks, deaths and bomb
    /// placement of the other player only arrive as messages. `None` = local game where the world decides everything.
    pub local: Option<usize>,
    pub outgoing: Vec<Outgoing>,
    /// `BombManager.nextId`: 5000 for player 1, 10000 for player 2 so that ids never collide.
    pub next_bomb_id: i64,
}

impl World {
    pub fn new(seed: i64) -> Self {
        Self::with_cells(generate(seed))
    }

    pub fn with_cells(cells: Cells) -> Self {
        Self {
            cells,
            flame_map: [[None; COLS]; ROWS],
            players: [Player::new(0), Player::new(1)],
            bombs: Vec::new(),
            flames: Vec::new(),
            now: 0,
            events: Vec::new(),
            locked: false,
            lock_now: -1,
            next_uid: 1,
            local: None,
            outgoing: Vec::new(),
            next_bomb_id: 5000,
        }
    }

    /// Whether this world decides the events of player `pi` (always in a local game, only for `local` online).
    fn authority(&self, pi: usize) -> bool {
        self.local.map_or(true, |l| l == pi)
    }

    fn notify(&mut self, o: Outgoing) {
        if self.local.is_some() {
            self.outgoing.push(o);
        }
    }

    fn uid(&mut self) -> u32 {
        self.next_uid += 1;
        self.next_uid
    }

    // ---- lookups ---------------------------------------------------------------------------------------------

    /// `MyPlayer.getStone`: stone under a stage position, `None` outside the grid (`BANNED`).
    pub fn stone_at_px(&self, x: f64, y: f64) -> Option<(i32, i32, Cell)> {
        let cx = stage_x_to_cell(x);
        let cy = stage_y_to_cell(y);
        if in_bounds(cx, cy) {
            Some((cx, cy, self.cells[cy as usize][cx as usize]))
        } else {
            None
        }
    }

    /// `Grid.getStone`: out of range counts as a solid stone.
    pub fn get_stone(&self, cx: i32, cy: i32) -> Cell {
        if in_bounds(cx, cy) {
            self.cells[cy as usize][cx as usize]
        } else {
            Cell::SOLID
        }
    }

    pub fn flame_at(&self, cx: i32, cy: i32) -> bool {
        in_bounds(cx, cy) && self.flame_map[cy as usize][cx as usize].is_some()
    }

    pub fn bomb_index_at(&self, cx: i32, cy: i32) -> Option<usize> {
        self.bombs.iter().position(|b| !b.exploded && b.cx == cx && b.cy == cy)
    }

    // ---- tick ------------------------------------------------------------------------------------------------

    pub fn update_players(&mut self) {
        for pi in 0..2 {
            self.update_player(pi);
        }
    }

    pub fn update_objects(&mut self) {
        for i in 0..self.bombs.len() {
            self.update_bomb(i);
        }
        self.bombs.retain(|b| !b.exploded);
        self.update_flames();
    }

    /// Players, then bombs, then flames (order inferred), then the clock advances.
    pub fn tick(&mut self) {
        self.update_players();
        self.update_objects();
        self.now += DT_MS;
    }

    // ---- death -----------------------------------------------------------------------------------------------

    /// `Game.thisPlayerDie`: ignored once the game is locked, except for deaths in the very same tick (draw).
    pub fn kill(&mut self, pi: usize) {
        if self.players[pi].dead || !self.authority(pi) {
            return;
        }
        if self.locked && self.lock_now != self.now {
            return;
        }
        if !self.locked {
            self.locked = true;
            self.lock_now = self.now;
        }
        self.players[pi].die();
        self.notify(Outgoing::Die);
    }

    // ---- players ---------------------------------------------------------------------------------------------

    fn update_player(&mut self, pi: usize) {
        {
            let p = &mut self.players[pi];
            p.cx = stage_x_to_cell(p.x);
            p.cy = stage_y_to_cell(p.y);
            p.anim_ticks += 1;
        }
        self.pick_up(pi);
        self.try_place_bomb(pi);
        let p = &self.players[pi];
        let (right, left, up, down) = (p.right, p.left, p.up, p.down);
        if right {
            self.walk(pi, Dir::Right);
        } else if left {
            self.walk(pi, Dir::Left);
        }
        if up {
            self.walk(pi, Dir::Up);
        } else if down {
            self.walk(pi, Dir::Down);
        }
    }

    fn pick_up(&mut self, pi: usize) {
        if !self.authority(pi) {
            return;
        }
        let (x, y) = (self.players[pi].x, self.players[pi].y);
        let Some((cx, cy, cell)) = self.stone_at_px(x, y) else { return };
        if !cell.is_item() {
            return;
        }
        self.events.push(Event::PickUp);
        let p = &mut self.players[pi];
        match cell.kind {
            Kind::ItemBomb => p.num_bombs += 1,
            Kind::ItemSpeed => {
                if p.speed < MAX_SPEED {
                    p.speed += SPEED_STEP;
                } else {
                    p.speed = MAX_SPEED;
                }
            }
            Kind::ItemBlast => p.num_blast += 1,
            Kind::ItemKick => p.kick = true,
            _ => {}
        }
        self.cells[cy as usize][cx as usize] = Cell::FREE;
        self.notify(Outgoing::Pickup { cx, cy });
    }

    fn try_place_bomb(&mut self, pi: usize) {
        if !self.authority(pi) || !(self.players[pi].space && self.players[pi].space_released) {
            return;
        }
        self.players[pi].space_released = false;
        let (x, y) = (self.players[pi].x, self.players[pi].y);
        let Some((cx, cy, cell)) = self.stone_at_px(x, y) else { return };
        if cell.kind != Kind::Free || self.players[pi].bombs_set >= self.players[pi].num_bombs {
            return;
        }
        self.events.push(Event::BombPlaced);
        self.players[pi].bombs_set += 1;
        self.set_bomb(cx, cy, pi);
        self.notify(Outgoing::BombPlaced { cx, cy });
    }

    /// `Grid.setBomb`
    pub fn set_bomb(&mut self, cx: i32, cy: i32, owner: usize) {
        let uid = self.uid();
        self.bombs.push(Bomb {
            uid,
            owner,
            x: cell_center(cx),
            y: cell_center(cy),
            cx,
            cy,
            start: self.now,
            moving: false,
            dx: 0.0,
            dy: 0.0,
            kicked_by: None,
            kicked: false,
            exploded: false,
            old: None,
            id: None,
            remote_kicked: false,
        });
        self.cells[cy as usize][cx as usize] = Cell { kind: Kind::ArmedBomb, open: false };
    }

    /// One axis of `MyPlayer.update` for a held direction.
    fn walk(&mut self, pi: usize, dir: Dir) {
        let (vx, vy) = dir.vec();
        let (vx, vy) = (vx as f64, vy as f64);
        let (px, py) = (self.players[pi].x, self.players[pi].y);
        let probe = self.stone_at_px(px + 21.0 * vx, py + 21.0 * vy);
        let enterable = probe.map_or(false, |(_, _, c)| c.can_enter());

        if enterable {
            let fx = stage_x_to_cell(px + DIST_TO_DIE * vx);
            let fy = stage_y_to_cell(py + DIST_TO_DIE * vy);
            if self.flame_at(fx, fy) {
                self.kill(pi);
            }
            let step = self.players[pi].step_distance(DT_MS as f64);
            let mut dist = step;
            let horizontal = vx != 0.0;
            let lat = if horizontal { py } else { px };
            let m = lat % 40.0;
            let mut lat_new = lat;
            if m < 20.0 {
                if m > dist {
                    lat_new -= dist;
                    dist = 0.0;
                } else {
                    lat_new -= m;
                    dist -= m;
                }
            } else if 40.0 - m > dist {
                lat_new += dist;
                dist = 0.0;
            } else {
                lat_new += 40.0 - m;
                dist -= 40.0 - m;
            }
            if dir == Dir::Down {
                // the original recomputes `dy = getDxDy()` after the alignment
                dist = step;
            }
            let p = &mut self.players[pi];
            if horizontal {
                p.y = lat_new;
                p.x = (p.x + vx * dist).clamp(LEFT_BORDER, RIGHT_BORDER);
            } else {
                p.x = lat_new;
                p.y = (p.y + vy * dist).clamp(UPPER_BORDER, LOWER_BORDER);
            }
        } else {
            // setMitArrayX / setMitArrayY: snap to the nearest multiple of 40
            let p = &mut self.players[pi];
            if vx != 0.0 {
                p.x = 40.0 * ((p.x + 20.0) / 40.0).floor();
            } else {
                p.y = 40.0 * ((p.y + 20.0) / 40.0).floor();
            }
            if let Some((bx, by, cell)) = probe {
                if cell.kind == Kind::ArmedBomb && self.players[pi].kick && self.authority(pi) {
                    let (sx, sy) = (self.players[pi].x, self.players[pi].y);
                    let own_is_bomb = matches!(self.stone_at_px(sx, sy), Some((_, _, c)) if c.kind == Kind::ArmedBomb);
                    if !own_is_bomb {
                        self.kick_bomb(bx, by, pi, vx * KICK_SPEED, vy * KICK_SPEED);
                    }
                }
            }
        }
    }

    // ---- bombs -----------------------------------------------------------------------------------------------

    /// `Bomb.kick`: a player can kick a bomb only if they were not the last one to kick it.
    pub fn kick_bomb(&mut self, cx: i32, cy: i32, by: usize, dx: f64, dy: f64) {
        let Some(i) = self.bomb_index_at(cx, cy) else { return };
        let b = &mut self.bombs[i];
        if (dx < 0.0 && b.cx <= 0) || (dy < 0.0 && b.cy <= 0) || (dx > 0.0 && b.cx >= 12) || (dy > 0.0 && b.cy >= 10) {
            return;
        }
        if b.kicked_by != Some(by) {
            b.kicked_by = Some(by);
            b.remote_kicked = false;
            b.moving = true;
            b.kicked = true;
            b.dx = dx;
            b.dy = dy;
            let id = match b.id {
                Some(id) => id,
                None => {
                    let id = self.next_bomb_id;
                    self.next_bomb_id += 1;
                    self.bombs[i].id = Some(id);
                    id
                }
            };
            let b = &self.bombs[i];
            let (bcx, bcy, t) = (b.cx, b.cy, b.start + BOMB_FUSE_MS - self.now);
            self.events.push(Event::Kick);
            self.notify(Outgoing::Kick { cx: bcx, cy: bcy, id, dx, dy, t });
        }
    }

    fn snap_bomb(&mut self, i: usize) {
        let b = &mut self.bombs[i];
        b.x = cell_center(b.cx);
        b.y = cell_center(b.cy);
    }

    fn update_bomb(&mut self, i: usize) {
        if self.bombs[i].exploded {
            return;
        }
        if self.now > BOMB_FUSE_MS + self.bombs[i].start && !self.bombs[i].remote_kicked {
            self.explode(i);
            return;
        }
        if !self.bombs[i].moving {
            return;
        }
        let diff = DT_MS as f64;
        let (mut x, mut y, dx, dy, kicked_by) = {
            let b = &self.bombs[i];
            (b.x + b.dx * diff, b.y + b.dy * diff, b.dx, b.dy, b.kicked_by)
        };
        x = x.clamp(20.0, 500.0);
        y = y.clamp(20.0, 420.0);
        self.bombs[i].x = x;
        self.bombs[i].y = y;
        let ox = if dx > 0.0 { 20.0 } else if dx < 0.0 { -20.0 } else { 0.0 };
        let oy = if dy > 0.0 { 20.0 } else if dy < 0.0 { -20.0 } else { 0.0 };

        // a player other than the kicker stops the bomb and is pushed one cell on
        let hit = (0..2).find(|&pi| {
            Some(pi) != kicked_by
                && self.authority(pi)
                && self.players[pi].hit_test(x + ox + BOARD_X, y + oy + BOARD_Y)
        });
        if let Some(pi) = hit {
            self.snap_bomb(i);
            let (bx, by) = (self.bombs[i].x, self.bombs[i].y);
            self.players[pi].x = bx + ox * 2.0 + BOARD_X;
            self.players[pi].y = by + oy * 2.0 + BOARD_Y;
            self.bombs[i].moving = false;
            self.bombs[i].remote_kicked = false;
            let b = &self.bombs[i];
            let (cx, cy, t) = (b.cx, b.cy, b.start + BOMB_FUSE_MS - self.now);
            if let Some(id) = b.id {
                self.notify(Outgoing::Stop { cx, cy, id, t });
            }
            return;
        }

        let ncx = grid_to_cell(x + ox);
        let ncy = grid_to_cell(y + oy);
        let (cx, cy) = (self.bombs[i].cx, self.bombs[i].cy);
        let target = self.get_stone(ncx, ncy);
        if (ncx, ncy) != (cx, cy) && !target.can_enter() {
            self.bombs[i].moving = false;
            self.snap_bomb(i);
            return;
        }
        if (ncx, ncy) != (cx, cy) {
            self.move_bomb_to(i, ncx, ncy);
        }
    }

    /// Moves the grid marker of a bomb to another cell (the cell it leaves gets its old stone back).
    fn move_bomb_to(&mut self, i: usize, ncx: i32, ncy: i32) {
        let (cx, cy) = (self.bombs[i].cx, self.bombs[i].cy);
        let restored = self.bombs[i].old.take().unwrap_or(Cell::FREE);
        self.cells[cy as usize][cx as usize] = restored;
        self.bombs[i].old = Some(self.cells[ncy as usize][ncx as usize]);
        self.cells[ncy as usize][ncx as usize] = Cell { kind: Kind::ArmedBomb, open: false };
        self.bombs[i].cx = ncx;
        self.bombs[i].cy = ncy;
    }

    /// `Bomb.explode`: centre flame plus four arms of `numBliz` tiles.
    pub fn explode(&mut self, i: usize) {
        if self.bombs[i].exploded {
            return;
        }
        self.events.push(Event::Explosion);
        if self.bombs[i].kicked && !self.bombs[i].remote_kicked {
            if let Some(id) = self.bombs[i].id {
                let (cx, cy) = (self.bombs[i].cx, self.bombs[i].cy);
                self.notify(Outgoing::Detonate { cx, cy, id });
            }
        }
        if self.bombs[i].kicked {
            self.bombs[i].kicked = false;
            self.bombs[i].moving = false;
            self.snap_bomb(i);
        }
        self.bombs[i].start = 0;
        self.bombs[i].exploded = true;
        self.bombs[i].old = None; // an item the bomb was sitting on is destroyed with the cell below
        let (bx, by, owner) = (self.bombs[i].cx, self.bombs[i].cy, self.bombs[i].owner);
        self.spawn_flame(bx, by, owner, 0, true);
        let arms = self.players[owner].num_blast as i32;
        for (vx, vy, rot) in [(1, 0, 90u16), (-1, 0, 90), (0, 1, 0), (0, -1, 0)] {
            for k in 1..=arms {
                let (cx, cy) = (bx + vx * k, by + vy * k);
                if !in_bounds(cx, cy) {
                    continue;
                }
                let cell = self.cells[cy as usize][cx as usize];
                match cell.kind {
                    Kind::Free => self.spawn_flame(cx, cy, owner, rot, false),
                    Kind::ItemBomb | Kind::ItemSpeed | Kind::ItemBlast | Kind::ItemKick => {
                        self.spawn_flame(cx, cy, owner, rot, false);
                        let c = &mut self.cells[cy as usize][cx as usize];
                        if !c.open {
                            c.open = true;
                        } else {
                            *c = Cell::FREE;
                        }
                        break;
                    }
                    Kind::Crate => {
                        self.spawn_flame(cx, cy, owner, rot, false);
                        self.cells[cy as usize][cx as usize] = Cell::FREE;
                        break;
                    }
                    Kind::Solid => break,
                    Kind::ArmedBomb => {
                        if let Some(j) = self.bomb_index_at(cx, cy) {
                            self.explode(j);
                        }
                        break;
                    }
                }
            }
        }
        let p = &mut self.players[owner];
        p.bombs_set = p.bombs_set.saturating_sub(1);
        self.cells[by as usize][bx as usize] = Cell::FREE;
    }

    // ---- flames ----------------------------------------------------------------------------------------------

    // ---- messages from the other client --------------------------------------------------------------------------

    fn bomb_by_id(&self, id: i64) -> Option<usize> {
        self.bombs.iter().position(|b| !b.exploded && b.id == Some(id))
    }

    /// Message 17: `Grid.setBomb` for the remote player (its `bombsSet` is not touched, like the original).
    pub fn remote_place_bomb(&mut self, cx: i32, cy: i32, owner: usize) {
        if in_bounds(cx, cy) {
            self.events.push(Event::BombPlaced);
            self.set_bomb(cx, cy, owner);
        }
    }

    /// Message 18: the remote player took an item; its new speed and blast range are adopted.
    pub fn remote_pickup(&mut self, cx: i32, cy: i32, owner: usize, speed: f64, blast: u32) {
        self.players[owner].speed = speed;
        self.players[owner].num_blast = blast;
        self.events.push(Event::PickUp);
        if in_bounds(cx, cy) && self.cells[cy as usize][cx as usize].is_item() {
            self.cells[cy as usize][cx as usize] = Cell::FREE;
        }
    }

    /// Message 19: `Bomb.remoteKick`.
    pub fn remote_kick(&mut self, cx: i32, cy: i32, owner: usize, dx: f64, dy: f64, t: i64, id: i64) {
        let Some(i) = self.bomb_index_at(cx, cy) else { return };
        let now = self.now;
        let b = &mut self.bombs[i];
        b.kicked_by = Some(owner);
        b.remote_kicked = true;
        b.start = now - BOMB_FUSE_MS + t;
        b.moving = true;
        b.kicked = true;
        b.dx = dx;
        b.dy = dy;
        if b.id.is_none() {
            b.id = Some(id);
        }
        self.events.push(Event::Kick);
    }

    /// Message 11: `BombManager.remoteStopBomb`; the sender keeps responsibility for the detonation.
    pub fn remote_stop(&mut self, id: i64, cx: i32, cy: i32, t: i64) {
        let Some(i) = self.bomb_by_id(id) else { return };
        if !in_bounds(cx, cy) {
            return;
        }
        if (self.bombs[i].cx, self.bombs[i].cy) != (cx, cy) {
            self.move_bomb_to(i, cx, cy);
        }
        let now = self.now;
        let b = &mut self.bombs[i];
        b.start = now - BOMB_FUSE_MS + t;
        b.moving = false;
        b.remote_kicked = true;
        self.snap_bomb(i);
    }

    /// Message 10: `BombManager.remoteExplode`.
    pub fn remote_detonate(&mut self, id: i64, cx: i32, cy: i32) {
        let Some(i) = self.bomb_by_id(id) else { return };
        if !in_bounds(cx, cy) {
            return;
        }
        if (self.bombs[i].cx, self.bombs[i].cy) != (cx, cy) {
            self.move_bomb_to(i, cx, cy);
        }
        self.explode(i);
    }

    /// Message `die`: the remote player is dead, the game is locked (`Game.remotePlayerDie`).
    pub fn remote_die(&mut self, pi: usize, x: f64, y: f64) {
        self.players[pi].x = x;
        self.players[pi].y = y;
        self.players[pi].die();
        self.locked = true;
        self.lock_now = -1;
    }

    /// `Grid.setBliz`
    fn spawn_flame(&mut self, cx: i32, cy: i32, owner: usize, rot: u16, center: bool) {
        let uid = self.uid();
        self.flames.push(Flame { uid, cx, cy, owner, start: self.now, center, rot });
        for pi in 0..2 {
            if self.players[pi].cx == cx && self.players[pi].cy == cy {
                self.kill(pi);
            }
        }
        self.flame_map[cy as usize][cx as usize] = Some(uid);
    }

    fn update_flames(&mut self) {
        let now = self.now;
        let mut expired = Vec::new();
        self.flames.retain(|f| {
            if now > FLAME_MS + f.start {
                expired.push((f.cx, f.cy, f.uid));
                false
            } else {
                true
            }
        });
        for (cx, cy, uid) in expired {
            // an older flame must not clear a newer one in the same cell
            if self.flame_map[cy as usize][cx as usize] == Some(uid) {
                self.flame_map[cy as usize][cx as usize] = None;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_world() -> World {
        World::with_cells(base_level())
    }

    fn hold(w: &mut World, pi: usize, d: Dir) {
        w.players[pi].key_down(d);
    }

    #[test]
    fn moves_speed_times_25ms_per_tick() {
        let mut w = empty_world();
        hold(&mut w, 0, Dir::Right);
        w.tick();
        assert!((w.players[0].x - 123.75).abs() < 1e-9); // 0.15 px/ms * 25 ms
        for _ in 0..39 {
            w.tick();
        }
        assert!((w.players[0].x - 270.0).abs() < 1e-9); // 150 px/s for one second
        assert_eq!(w.players[0].y, 40.0);
    }

    #[test]
    fn stops_at_right_border_and_never_leaves_board() {
        let mut w = empty_world();
        hold(&mut w, 0, Dir::Right);
        for _ in 0..200 {
            w.tick();
        }
        assert_eq!(w.players[0].x, 600.0);
    }

    #[test]
    fn solid_stone_blocks_and_snaps_to_cell_centre() {
        let mut w = empty_world();
        // row 1 centre y = 80; stone at (1,1) = stage x 140..180
        w.players[0].x = 130.0;
        w.players[0].y = 80.0;
        hold(&mut w, 0, Dir::Right);
        w.tick();
        assert_eq!(w.players[0].x, 120.0);
        assert_eq!(w.players[0].y, 80.0);
    }

    #[test]
    fn corner_assist_consumes_movement_for_right_left_up() {
        let mut w = empty_world();
        w.players[0].y = 50.0; // 10 px below row-0 centre
        hold(&mut w, 0, Dir::Right);
        w.tick();
        // m = 10 > 3.75: all movement goes into aligning y, x unchanged
        assert!((w.players[0].y - 46.25).abs() < 1e-9);
        assert_eq!(w.players[0].x, 120.0);

        let mut w = empty_world();
        w.players[0].y = 42.0;
        hold(&mut w, 0, Dir::Right);
        w.tick();
        assert_eq!(w.players[0].y, 40.0);
        assert!((w.players[0].x - 121.75).abs() < 1e-9); // 3.75 - 2 left over
    }

    #[test]
    fn down_ignores_alignment_cost_quirk() {
        let mut w = empty_world();
        w.players[0].x = 125.0;
        hold(&mut w, 0, Dir::Down);
        w.tick();
        assert!((w.players[0].x - 121.25).abs() < 1e-9);
        assert!((w.players[0].y - 43.75).abs() < 1e-9); // full step although x was aligned

        let mut w = empty_world();
        w.players[0].x = 125.0;
        w.players[0].y = 100.0;
        hold(&mut w, 0, Dir::Up);
        w.tick();
        assert!((w.players[0].x - 121.25).abs() < 1e-9);
        assert_eq!(w.players[0].y, 100.0);
    }

    #[test]
    fn boxes_and_bombs_block_but_open_items_do_not() {
        let mut cells = base_level();
        cells[0][2] = Cell { kind: Kind::Crate, open: false };
        let mut w = World::with_cells(cells);
        w.players[0].x = 150.0; // cell 1; the crate sits in cell 2 (probe 21 px ahead)
        hold(&mut w, 0, Dir::Right);
        for _ in 0..30 {
            w.tick();
        }
        // stops with its centre on the cell before the crate: x = 160
        assert_eq!(w.players[0].x, 160.0);

        let mut cells = base_level();
        cells[0][2] = Cell { kind: Kind::ItemSpeed, open: true };
        let mut w = World::with_cells(cells);
        hold(&mut w, 0, Dir::Right);
        for _ in 0..30 {
            w.tick();
        }
        assert!(w.players[0].x > 200.0);
        assert!(w.players[0].speed > 0.15, "picked up the open item on the way");
    }

    #[test]
    fn pickups_apply_effects_and_speed_is_capped() {
        let mut w = empty_world();
        for (i, kind) in [Kind::ItemBomb, Kind::ItemBlast, Kind::ItemKick].into_iter().enumerate() {
            w.cells[0][i] = Cell { kind, open: true };
            w.players[0].x = 120.0 + 40.0 * i as f64;
            w.tick();
        }
        assert_eq!(w.players[0].num_bombs, 2);
        assert_eq!(w.players[0].num_blast, 3);
        assert!(w.players[0].kick);
        assert!(w.events.iter().filter(|e| **e == Event::PickUp).count() == 3);

        let mut w = empty_world();
        for n in 0..6 {
            w.cells[0][0] = Cell { kind: Kind::ItemSpeed, open: true };
            w.tick();
            let s = w.players[0].speed;
            if n == 0 {
                assert!((s - 0.16).abs() < 1e-12);
            }
            // `if speed < MAX then speed += 0.01 else speed = MAX` can overshoot by one step when the sum lands just below 0.18
            assert!((0.16 - 1e-12..=0.19 + 1e-12).contains(&s));
        }
        let s = w.players[0].speed;
        assert!((s - 0.18).abs() < 1e-12 || (s - 0.19).abs() < 1e-12);
    }

    fn place_bomb_by_key(w: &mut World, pi: usize) {
        w.players[pi].key_space_down();
        w.tick();
    }

    #[test]
    fn bomb_is_placed_once_per_press_and_limited_by_num_bombs() {
        let mut w = empty_world();
        place_bomb_by_key(&mut w, 0);
        assert_eq!(w.bombs.len(), 1);
        assert_eq!(w.cells[0][0].kind, Kind::ArmedBomb);
        // held key: no second bomb
        w.tick();
        assert_eq!(w.bombs.len(), 1);
        // release and press again: limit is numBombs = 1
        w.players[0].keys_released([false; 4], false);
        place_bomb_by_key(&mut w, 0);
        assert_eq!(w.bombs.len(), 1);
        assert_eq!(w.players[0].bombs_set, 1);
    }

    #[test]
    fn fuse_is_3500_ms_strictly_greater() {
        let mut w = empty_world();
        w.players[0].x = 400.0; // keep the player away from the blast
        w.set_bomb(6, 4, 0);
        w.players[0].bombs_set = 1;
        let mut ticks = 0;
        while !w.bombs.is_empty() {
            w.tick();
            ticks += 1;
            assert!(ticks < 1000);
        }
        // placed at now = 0; explodes in the first tick with now > 3500, i.e. now = 3525 (tick 142 counting from 1)
        assert_eq!(w.now, 3525 + DT_MS);
        assert_eq!(w.players[0].bombs_set, 0);
    }

    fn flame_cells(w: &World) -> Vec<(i32, i32)> {
        let mut v: Vec<_> = w.flames.iter().map(|f| (f.cx, f.cy)).collect();
        v.sort();
        v
    }

    fn explode_now(w: &mut World, cx: i32, cy: i32) {
        w.set_bomb(cx, cy, 0);
        w.players[0].bombs_set += 1;
        let i = w.bomb_index_at(cx, cy).unwrap();
        w.explode(i);
    }

    #[test]
    fn blast_is_a_plus_of_range_two_by_default() {
        let mut w = empty_world();
        w.players[0].x = 600.0; // out of the way
        w.players[0].y = 440.0;
        explode_now(&mut w, 6, 4);
        assert_eq!(
            flame_cells(&w),
            vec![(4, 4), (5, 4), (6, 2), (6, 3), (6, 4), (6, 5), (6, 6), (7, 4), (8, 4)]
        );
        assert_eq!(w.flames.iter().filter(|f| f.center).count(), 1);
        assert_eq!(w.cells[4][6], Cell::FREE);
    }

    #[test]
    fn range_grows_with_blast_items() {
        let mut w = empty_world();
        w.players[0].num_blast = 3;
        explode_now(&mut w, 6, 4);
        assert_eq!(w.flames.len(), 1 + 4 * 3);
    }

    #[test]
    fn solid_stone_stops_blast_without_flame() {
        let mut w = empty_world();
        explode_now(&mut w, 0, 1); // (1,1) is solid, left and up/down edges
        assert_eq!(flame_cells(&w), vec![(0, 0), (0, 1), (0, 2), (0, 3)]);
    }

    #[test]
    fn crate_is_destroyed_and_stops_blast() {
        let mut w = empty_world();
        w.cells[4][7] = Cell { kind: Kind::Crate, open: false };
        explode_now(&mut w, 6, 4);
        assert!(w.flame_at(7, 4));
        assert!(!w.flame_at(8, 4));
        assert_eq!(w.cells[4][7], Cell::FREE);
    }

    #[test]
    fn hidden_item_is_revealed_then_destroyed_by_second_blast() {
        let mut w = empty_world();
        w.cells[4][7] = Cell { kind: Kind::ItemBlast, open: false };
        explode_now(&mut w, 6, 4);
        assert!(w.flame_at(7, 4) && !w.flame_at(8, 4));
        assert_eq!(w.cells[4][7], Cell { kind: Kind::ItemBlast, open: true });
        explode_now(&mut w, 6, 4);
        assert_eq!(w.cells[4][7], Cell::FREE);
    }

    #[test]
    fn blast_chains_through_bombs() {
        let mut w = empty_world();
        w.players[0].x = 600.0;
        w.players[0].y = 440.0;
        w.set_bomb(8, 4, 1);
        w.players[1].bombs_set = 1;
        explode_now(&mut w, 6, 4);
        assert!(w.bombs.iter().all(|b| b.exploded));
        // second bomb (player 2) blasts (9,4),(10,4)
        assert!(w.flame_at(9, 4) && w.flame_at(10, 4));
        // first bomb stops at the second one: no flame at (8,4) from it, but the second bomb's centre is there
        assert!(w.flame_at(8, 4));
        assert_eq!(w.players[1].bombs_set, 0);
    }

    #[test]
    fn flames_last_500_ms() {
        let mut w = empty_world();
        w.players[0].x = 600.0;
        w.players[0].y = 440.0;
        explode_now(&mut w, 6, 4);
        let mut ticks = 0;
        while !w.flames.is_empty() {
            w.tick();
            ticks += 1;
        }
        // created at now = 0; removed in the first update with now > 500, i.e. now = 525 (22nd tick)
        assert_eq!(ticks, 22);
        assert!(!w.flame_at(6, 4));
    }

    #[test]
    fn older_flame_does_not_clear_newer_one() {
        let mut w = empty_world();
        w.players[0].x = 600.0;
        w.players[0].y = 440.0;
        explode_now(&mut w, 6, 4);
        for _ in 0..10 {
            w.tick();
        }
        explode_now(&mut w, 6, 4); // new flame on the same cells at now = 250
        for _ in 0..13 {
            w.tick(); // now = 575: first set expired
        }
        assert!(w.flame_at(6, 4));
    }

    #[test]
    fn flame_spawned_on_player_kills_and_both_in_same_tick_is_draw() {
        let mut w = empty_world();
        w.players[0].cx = 6;
        w.players[0].cy = 4;
        w.players[1].cx = 7;
        w.players[1].cy = 4;
        explode_now(&mut w, 6, 4);
        assert!(w.players[0].dead && w.players[1].dead);
        assert!(w.locked);

        let mut w = empty_world();
        w.players[0].cx = 6;
        w.players[0].cy = 4;
        w.players[1].cx = 12;
        w.players[1].cy = 10;
        explode_now(&mut w, 6, 4);
        assert!(w.players[0].dead && !w.players[1].dead);
        // later deaths are ignored (Game.locked)
        w.now += DT_MS;
        w.kill(1);
        assert!(!w.players[1].dead);
    }

    #[test]
    fn walking_into_a_flame_kills_with_18px_lookahead() {
        let mut w = empty_world();
        w.players[1].x = 600.0;
        w.players[1].y = 440.0;
        w.flame_map[0][1] = Some(99);
        w.players[0].x = 121.0; // 18 px ahead = 139 -> still cell 0
        hold(&mut w, 0, Dir::Right);
        w.tick();
        assert!(!w.players[0].dead);
        w.players[0].x = 123.0; // 123 + 18 = 141 -> cell 1
        w.tick();
        assert!(w.players[0].dead);
    }

    #[test]
    fn kicking_requires_the_item_and_moves_at_100_px_per_s() {
        let mut w = empty_world();
        w.players[0].x = 120.0;
        w.players[0].kick = true;
        w.set_bomb(1, 0, 1);
        w.players[1].bombs_set = 1;
        hold(&mut w, 0, Dir::Right);
        w.tick(); // blocked by the bomb at cell 1 -> kick; the bomb already moves in the same tick
        assert!(w.bombs[0].moving);
        assert_eq!(w.players[0].x, 120.0);
        // 0.1 px/ms * 25 ms = 2.5 px per tick; the cell changes as soon as the leading edge enters cell 2
        assert!((w.bombs[0].x - 62.5).abs() < 1e-9);
        assert_eq!(w.bombs[0].cx, 2);
        assert_eq!(w.cells[0][1], Cell::FREE);
        assert_eq!(w.cells[0][2].kind, Kind::ArmedBomb);
        w.tick();
        assert!((w.bombs[0].x - 65.0).abs() < 1e-9);

        let mut w = empty_world();
        w.players[0].x = 120.0;
        w.set_bomb(1, 0, 1);
        hold(&mut w, 0, Dir::Right);
        w.tick();
        assert!(!w.bombs[0].moving, "no kick without the kick item");
    }

    #[test]
    fn same_player_cannot_kick_the_same_bomb_twice() {
        let mut w = empty_world();
        w.set_bomb(5, 4, 0);
        w.kick_bomb(5, 4, 0, 0.1, 0.0);
        assert!(w.bombs[0].moving);
        w.bombs[0].moving = false;
        w.kick_bomb(5, 4, 0, 0.1, 0.0);
        assert!(!w.bombs[0].moving);
        w.kick_bomb(5, 4, 1, 0.1, 0.0);
        assert!(w.bombs[0].moving);
    }

    #[test]
    fn kicked_bomb_stops_in_front_of_a_stone() {
        let mut w = empty_world();
        w.players[0].x = 600.0;
        w.players[0].y = 440.0;
        w.players[1].x = 600.0;
        w.players[1].y = 440.0;
        w.set_bomb(0, 1, 0); // row 1: stone at (1,1)
        w.kick_bomb(0, 1, 1, 0.1, 0.0);
        for _ in 0..10 {
            w.tick();
        }
        assert!(!w.bombs[0].moving);
        assert_eq!((w.bombs[0].cx, w.bombs[0].x), (0, 20.0));
    }

    #[test]
    fn kicked_bomb_slides_over_open_items_and_restores_them() {
        let mut w = empty_world();
        w.players[0].x = 600.0;
        w.players[0].y = 440.0;
        w.players[1].x = 600.0;
        w.players[1].y = 440.0;
        w.cells[0][3] = Cell { kind: Kind::ItemBomb, open: true };
        w.set_bomb(2, 0, 0);
        w.kick_bomb(2, 0, 1, 0.1, 0.0);
        for _ in 0..4 {
            w.tick();
        }
        // the leading edge (x + 20) is in cell 3 from the first tick on; the item is remembered
        assert_eq!(w.bombs[0].cx, 3);
        assert_eq!(w.bombs[0].old, Some(Cell { kind: Kind::ItemBomb, open: true }));
        assert_eq!(w.cells[0][3].kind, Kind::ArmedBomb);
        for _ in 0..16 {
            w.tick();
        }
        assert_eq!(w.bombs[0].cx, 4);
        assert_eq!(w.cells[0][3], Cell { kind: Kind::ItemBomb, open: true });
    }
}
