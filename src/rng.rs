//! Port of `MyMath`: the seeded linear congruential generator that makes both clients build the same level.

const A: i64 = 1291;
const B: i64 = 4621;
const M: i64 = 21870;

/// Seeds the original accepts for the deterministic path are `0..M`; anything else falls back to `Math.random()`.
#[derive(Clone, Debug)]
pub struct MyMath {
    seed: i64,
    fallback: u64,
}

impl MyMath {
    pub fn new(seed: i64) -> Self {
        Self { seed, fallback: (seed as u64).wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407) | 1 }
    }

    /// Next value in `[0, 1)`.
    pub fn random(&mut self) -> f64 {
        if self.seed < 0 || self.seed > M - 1 {
            // xorshift64*; stands in for Math.random()
            let mut x = self.fallback;
            x ^= x >> 12;
            x ^= x << 25;
            x ^= x >> 27;
            self.fallback = x;
            return (x.wrapping_mul(0x2545F4914F6CDD1D) >> 11) as f64 / (1u64 << 53) as f64;
        }
        let next = (A * self.seed + B) % M;
        self.seed = next;
        next as f64 / M as f64
    }

    /// `min + floor(random() * (max + 1 - min))`
    pub fn random_min_max(&mut self, min: i32, max: i32) -> i32 {
        min + (self.random() * (max + 1 - min) as f64).floor() as i32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lcg_matches_formula() {
        let mut r = MyMath::new(0);
        assert_eq!(r.random(), 4621.0 / 21870.0);
        // (1291*4621 + 4621) = 5_970_332 = 272*21870 + 21692
        assert_eq!(r.random(), 21692.0 / 21870.0);
    }

    #[test]
    fn seed_one_first_value() {
        let mut r = MyMath::new(1);
        assert_eq!(r.random(), (1291 + 4621) as f64 / 21870.0); // 5912/21870
    }

    #[test]
    fn min_max_is_inclusive_floor() {
        let mut r = MyMath::new(1);
        // 5912/21870 = 0.27034...; floor(0.27034 * 13) = 3
        assert_eq!(r.random_min_max(0, 12), 3);
    }
}
