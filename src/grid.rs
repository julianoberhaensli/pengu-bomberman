//! Tile grid, coordinate conversions and level generation (ports of `Grid`, `BaseStone`).

use crate::rng::MyMath;

pub const COLS: usize = 13;
pub const ROWS: usize = 11;
pub const TILE: f64 = 40.0;
/// Stage position of the grid clip.
pub const BOARD_X: f64 = 100.0;
pub const BOARD_Y: f64 = 20.0;

pub const NUM_CRATES: usize = 65;
pub const NUM_BOMB_ITEMS: usize = 10;
pub const NUM_SPEED_ITEMS: usize = 6;
pub const NUM_BLAST_ITEMS: usize = 6;
pub const NUM_KICK_ITEMS: usize = 6;

/// `BaseStone.sType` values that can live in the grid. `ARMED_BLIZ` (flames) lives in a separate map.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Free,
    Solid,
    Crate,
    ItemBomb,
    ItemSpeed,
    ItemBlast,
    ItemKick,
    ArmedBomb,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Cell {
    pub kind: Kind,
    /// `isOpen`: a box whose hidden item has been revealed by a blast.
    pub open: bool,
}

impl Cell {
    pub const FREE: Cell = Cell { kind: Kind::Free, open: false };
    pub const SOLID: Cell = Cell { kind: Kind::Solid, open: false };

    pub fn is_item(&self) -> bool {
        matches!(self.kind, Kind::ItemBomb | Kind::ItemSpeed | Kind::ItemBlast | Kind::ItemKick)
    }

    /// `BaseStone.canEnter`
    pub fn can_enter(&self) -> bool {
        match self.kind {
            Kind::Free => true,
            _ => self.open && self.is_item(),
        }
    }
}

/// `Grid.arrayPosToX/Y`: grid-local centre of a cell.
pub fn cell_center(i: i32) -> f64 {
    i as f64 * TILE + 20.0
}

/// `Grid.xToArrayPos` / `yToArrayPos` (grid-local px -> cell). Quirk: exactly 40 maps to 0.
pub fn grid_to_cell(v: f64) -> i32 {
    if v <= 40.0 {
        return 0;
    }
    let m = v % 40.0;
    ((v + 40.0 - m) / 40.0 - 1.0) as i32
}

/// `MyPlayer.xToArrayPos` (stage px -> cell column).
pub fn stage_x_to_cell(x: f64) -> i32 {
    let m = (x - 20.0) % 40.0;
    ((x - 20.0 - m) / 40.0 - 2.0) as i32
}

/// `MyPlayer.yToArrayPos` (stage px -> cell row).
pub fn stage_y_to_cell(y: f64) -> i32 {
    let m = (y + 20.0) % 40.0;
    ((y + 20.0 - m) / 40.0 - 1.0) as i32
}

/// Stage-space centre of a cell.
pub fn stage_center(cx: i32, cy: i32) -> (f64, f64) {
    (BOARD_X + cell_center(cx), BOARD_Y + cell_center(cy))
}

pub type Cells = [[Cell; COLS]; ROWS];

pub fn in_bounds(x: i32, y: i32) -> bool {
    x >= 0 && y >= 0 && (x as usize) < COLS && (y as usize) < ROWS
}

/// `Grid.init` template: 0 free, 1 solid stone, 9 banned (start zones, never receive boxes).
const TEMPLATE: [[u8; COLS]; ROWS] = [
    [9, 9, 9, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
    [9, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0],
    [9, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
    [0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0],
    [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
    [0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0],
    [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
    [0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0],
    [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 9],
    [0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 9],
    [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 9, 9, 9],
];

/// The static stones only (before boxes are placed).
pub fn base_level() -> Cells {
    let mut cells = [[Cell::FREE; COLS]; ROWS];
    for (y, row) in TEMPLATE.iter().enumerate() {
        for (x, t) in row.iter().enumerate() {
            if *t == 1 {
                cells[y][x] = Cell::SOLID;
            }
        }
    }
    cells
}

/// `Grid.init` + `setStones`: deterministic level from `seed`.
pub fn generate(seed: i64) -> Cells {
    let mut cells = base_level();
    let mut occupied = TEMPLATE;
    let mut rng = MyMath::new(seed);
    let (mut bombs, mut speed, mut blast, mut kick) = (0, 0, 0, 0);
    let mut placed = 0;
    while placed < NUM_CRATES {
        let x = rng.random_min_max(0, 12) as usize;
        let y = rng.random_min_max(0, 10) as usize;
        if occupied[y][x] != 0 {
            continue;
        }
        let kind = if bombs < NUM_BOMB_ITEMS {
            bombs += 1;
            Kind::ItemBomb
        } else if speed < NUM_SPEED_ITEMS {
            speed += 1;
            Kind::ItemSpeed
        } else if blast < NUM_BLAST_ITEMS {
            blast += 1;
            Kind::ItemBlast
        } else if kick < NUM_KICK_ITEMS {
            kick += 1;
            Kind::ItemKick
        } else {
            Kind::Crate
        };
        placed += 1;
        occupied[y][x] = 2;
        cells[y][x] = Cell { kind, open: false };
    }
    cells
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grid_to_cell_matches_original_quirk() {
        assert_eq!(grid_to_cell(0.0), 0);
        assert_eq!(grid_to_cell(20.0), 0);
        assert_eq!(grid_to_cell(40.0), 0); // `x <= 40 -> 0`
        assert_eq!(grid_to_cell(41.0), 1);
        assert_eq!(grid_to_cell(79.0), 1);
        assert_eq!(grid_to_cell(80.0), 2); // (80 + 40 - 0) / 40 - 1
        assert_eq!(grid_to_cell(500.0), 12);
        assert_eq!(grid_to_cell(520.0), 13);
    }

    #[test]
    fn player_cell_spans() {
        // cell i covers stage x in [100 + 40 i, 140 + 40 i)
        assert_eq!(stage_x_to_cell(100.0), 0);
        assert_eq!(stage_x_to_cell(99.0), -1);
        assert_eq!(stage_x_to_cell(120.0), 0);
        assert_eq!(stage_x_to_cell(139.9), 0);
        assert_eq!(stage_x_to_cell(140.0), 1);
        assert_eq!(stage_x_to_cell(600.0), 12);
        // cell j covers stage y in [20 + 40 j, 60 + 40 j)
        assert_eq!(stage_y_to_cell(20.0), 0);
        assert_eq!(stage_y_to_cell(19.0), -1);
        assert_eq!(stage_y_to_cell(40.0), 0);
        assert_eq!(stage_y_to_cell(60.0), 1);
        assert_eq!(stage_y_to_cell(440.0), 10);
    }

    #[test]
    fn centres() {
        assert_eq!(cell_center(0), 20.0);
        assert_eq!(cell_center(12), 500.0);
        assert_eq!(stage_center(0, 0), (120.0, 40.0));
        assert_eq!(stage_center(12, 10), (600.0, 440.0));
    }

    #[test]
    fn static_level_has_stones_at_odd_odd() {
        let l = base_level();
        let mut solid = 0;
        for y in 0..ROWS {
            for x in 0..COLS {
                if l[y][x].kind == Kind::Solid {
                    assert!(x % 2 == 1 && y % 2 == 1, "stone at {x},{y}");
                    solid += 1;
                }
            }
        }
        assert_eq!(solid, 5 * 6);
    }

    #[test]
    fn generated_level_counts_and_start_zones() {
        for seed in [0, 1, 777, 20000] {
            let l = generate(seed);
            let count = |k: Kind| l.iter().flatten().filter(|c| c.kind == k).count();
            assert_eq!(count(Kind::Crate), 65 - 28);
            assert_eq!(count(Kind::ItemBomb), 10);
            assert_eq!(count(Kind::ItemSpeed), 6);
            assert_eq!(count(Kind::ItemBlast), 6);
            assert_eq!(count(Kind::ItemKick), 6);
            assert_eq!(count(Kind::Solid), 30);
            for (x, y) in [(0, 0), (1, 0), (2, 0), (0, 1), (0, 2), (12, 10), (11, 10), (10, 10), (12, 9), (12, 8)] {
                assert_eq!(l[y][x], Cell::FREE, "start zone {x},{y} seed {seed}");
            }
        }
    }

    #[test]
    fn generation_is_deterministic_and_seed_dependent() {
        assert_eq!(generate(42), generate(42));
        assert_ne!(generate(42), generate(43));
    }

    #[test]
    fn first_box_position_follows_rng_order() {
        // seed 1: first random = 5912/21870 -> x = floor(0.2703*13) = 3; second = (1291*5912+4621)%21870 -> y
        let s2 = (1291i64 * 5912 + 4621) % 21870;
        let y = ((s2 as f64 / 21870.0) * 11.0).floor() as usize;
        let l = generate(1);
        assert_eq!(l[y][3].kind, Kind::ItemBomb, "first placed box is a bomb item at (3,{y})");
    }

    #[test]
    fn can_enter_rules() {
        assert!(Cell::FREE.can_enter());
        assert!(!Cell::SOLID.can_enter());
        assert!(!Cell { kind: Kind::Crate, open: false }.can_enter());
        assert!(!Cell { kind: Kind::ItemBomb, open: false }.can_enter());
        assert!(Cell { kind: Kind::ItemBomb, open: true }.can_enter());
        assert!(!Cell { kind: Kind::ArmedBomb, open: false }.can_enter());
    }
}
