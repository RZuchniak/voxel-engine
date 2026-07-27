//! `WorldgenRandom` — the RNG that drives feature decoration (trees, ores, plants).
//!
//! This is **not** the same random source the terrain uses, and the difference is easy to get
//! wrong in a way that produces plausible-but-incorrect output. `mc::rng::XoroshiroRandom` is
//! sampled directly by the density functions. Features instead go through Java's
//! `WorldgenRandom`, which is a *legacy-shaped* interface wrapped around a xoroshiro source:
//!
//! ```java
//! public class WorldgenRandom extends LegacyRandomSource {         // ← BitRandomSource semantics
//!    public int next(final int bits) {
//!       return this.randomSource instanceof LegacyRandomSource legacy
//!          ? legacy.next(bits)
//!          : (int)(this.randomSource.nextLong() >>> 64 - bits);    // ← xoroshiro: TOP bits
//!    }
//! }
//! ```
//!
//! So every `next(bits)` **consumes a whole `xoroshiro.nextLong()` and keeps only the top
//! `bits`**, and all the higher-level draws (`nextInt(bound)`, `nextFloat`, `nextLong`) are
//! `BitRandomSource`'s defaults layered on that. Two consequences worth stating plainly, because
//! both are counter-intuitive:
//!
//! - [`Self::next_long`] costs **two** xoroshiro draws, not one — it is `next(32) << 32 | next(32)`.
//! - [`Self::next_int_bound`] is Java's classic rejection loop over `next(31)`, *not*
//!   `XoroshiroRandom::next_int_bound`. Using the xoroshiro one here would give a different
//!   value for the same state, and the tree would land somewhere else.
//!
//! Validated against decompiled 26.2 `WorldgenRandom.java`, `LegacyRandomSource.java` and
//! `BitRandomSource.java`.

use super::rng::XoroshiroRandom;

/// `BitRandomSource.FLOAT_MULTIPLIER`.
const FLOAT_MULTIPLIER: f32 = 5.9604645E-8;
/// `BitRandomSource.DOUBLE_MULTIPLIER`. Note Java declares this as a **float** literal
/// (`1.110223E-16F`) and widens it, so it is *not* the same as the `double` 2^-53 used by
/// `XoroshiroRandom::next_double`. Transcribed as the widened float to match.
const DOUBLE_MULTIPLIER: f64 = 1.110223E-16f32 as f64;

/// Java's `WorldgenRandom` wrapping an `XoroshiroRandomSource`.
pub struct WorldgenRandom {
    source: XoroshiroRandom,
}

impl WorldgenRandom {
    pub fn from_seed(seed: i64) -> Self {
        Self { source: XoroshiroRandom::from_seed(seed) }
    }

    /// `WorldgenRandom.setSeed` — delegates to the wrapped source, which re-runs
    /// `upgradeSeedTo128bit`.
    pub fn set_seed(&mut self, seed: i64) {
        self.source = XoroshiroRandom::from_seed(seed);
    }

    /// `next(bits)` — one full xoroshiro draw, keeping the **top** `bits`.
    #[inline]
    fn next(&mut self, bits: u32) -> i32 {
        debug_assert!((1..=32).contains(&bits));
        ((self.source.next_long() as u64) >> (64 - bits)) as i32
    }

    /// `BitRandomSource.nextInt()`.
    pub fn next_int(&mut self) -> i32 {
        self.next(32)
    }

    /// `BitRandomSource.nextInt(bound)` — the power-of-two shortcut plus Java's rejection loop.
    ///
    /// The loop's exit test is written the way Java writes it (`sample - modulo + (bound - 1)`
    /// overflowing to negative), because that overflow *is* the condition; rewriting it as a
    /// comparison changes which samples are rejected.
    pub fn next_int_bound(&mut self, bound: i32) -> i32 {
        assert!(bound > 0, "bound must be positive");
        if (bound & (bound - 1)) == 0 {
            // Power of two: take the high bits of a 31-bit sample.
            return ((bound as i64).wrapping_mul(self.next(31) as i64) >> 31) as i32;
        }
        loop {
            let sample = self.next(31);
            let modulo = sample % bound;
            if sample.wrapping_sub(modulo).wrapping_add(bound - 1) >= 0 {
                return modulo;
            }
        }
    }

    /// `BitRandomSource.nextLong()` — **two** `next(32)` draws, so two xoroshiro steps.
    pub fn next_long(&mut self) -> i64 {
        let upper = self.next(32) as i64;
        let lower = self.next(32) as i64;
        (upper << 32).wrapping_add(lower)
    }

    /// `BitRandomSource.nextBoolean()`.
    pub fn next_bool(&mut self) -> bool {
        self.next(1) != 0
    }

    /// `BitRandomSource.nextFloat()`.
    pub fn next_float(&mut self) -> f32 {
        self.next(24) as f32 * FLOAT_MULTIPLIER
    }

    /// `BitRandomSource.nextDouble()`.
    pub fn next_double(&mut self) -> f64 {
        let upper = self.next(26) as i64;
        let lower = self.next(27) as i64;
        (((upper << 27) + lower) as f64) * DOUBLE_MULTIPLIER
    }

    /// `WorldgenRandom.setDecorationSeed(seed, chunkMinBlockX, chunkMinBlockZ)`.
    ///
    /// Seeds this generator for one chunk's whole decoration pass and returns the value that
    /// [`Self::set_feature_seed`] is then salted from. ⚠️ The two arguments are **block**
    /// coordinates of the chunk origin, not chunk coordinates — vanilla passes
    /// `SectionPos.origin()`, so `chunkX * 16`. Passing chunk coords silently produces a
    /// completely different, still plausible-looking world.
    pub fn set_decoration_seed(&mut self, seed: i64, origin_x: i32, origin_z: i32) -> i64 {
        self.set_seed(seed);
        let x_scale = self.next_long() | 1;
        let z_scale = self.next_long() | 1;
        let result = (origin_x as i64)
            .wrapping_mul(x_scale)
            .wrapping_add((origin_z as i64).wrapping_mul(z_scale))
            ^ seed;
        self.set_seed(result);
        result
    }

    /// `WorldgenRandom.setFeatureSeed(decorationSeed, index, step)`.
    ///
    /// `index` is the feature's **global index within its generation step**, assigned by
    /// `FeatureSorter.buildFeaturesPerStep` — a topological sort over every possible biome's
    /// ordered feature list. See `mc::feature_order` for how far that is reproduced here; it is
    /// the single thing standing between this port and bit-exact feature placement.
    pub fn set_feature_seed(&mut self, decoration_seed: i64, index: i32, step: i32) {
        let result = decoration_seed
            .wrapping_add(index as i64)
            .wrapping_add(10_000i64.wrapping_mul(step as i64));
        self.set_seed(result);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `next(bits)` must consume a whole xoroshiro draw and keep the **top** bits.
    ///
    /// The tempting reading is that it takes the low bits (that is what
    /// `XoroshiroRandom::next_int` does). Getting this backwards still yields well-distributed
    /// numbers, so nothing looks broken — the trees simply grow somewhere else.
    #[test]
    fn next_takes_the_high_bits_of_a_full_draw() {
        let seed = 6954908675375307936i64;
        let mut expected = XoroshiroRandom::from_seed(seed);
        let raw = expected.next_long() as u64;

        let mut wg = WorldgenRandom::from_seed(seed);
        assert_eq!(wg.next(32), (raw >> 32) as i32, "next(32) must be the top 32 bits");

        let mut wg = WorldgenRandom::from_seed(seed);
        assert_eq!(wg.next(24), (raw >> 40) as i32, "next(24) must be the top 24 bits");
    }

    /// `nextLong` costs two draws — the property that decides whether `setDecorationSeed`
    /// lands on the right value, since it draws two longs before mixing.
    #[test]
    fn next_long_consumes_two_xoroshiro_draws() {
        let seed = -42i64;
        let mut reference = XoroshiroRandom::from_seed(seed);
        let a = (reference.next_long() as u64) >> 32;
        let b = (reference.next_long() as u64) >> 32;
        let expected = (((a as u32 as i32) as i64) << 32).wrapping_add((b as u32 as i32) as i64);

        let mut wg = WorldgenRandom::from_seed(seed);
        assert_eq!(wg.next_long(), expected);
    }

    /// A power-of-two bound must take the shortcut branch, not the rejection loop — they
    /// consume different numbers of draws, so a mix-up desynchronises everything after it.
    #[test]
    fn power_of_two_bounds_take_the_shortcut() {
        let mut shortcut = WorldgenRandom::from_seed(7);
        let value = shortcut.next_int_bound(16);
        assert!((0..16).contains(&value));

        // The shortcut consumes exactly one draw; check by replaying the state.
        let mut manual = WorldgenRandom::from_seed(7);
        let sample = manual.next(31) as i64;
        assert_eq!(value, ((16i64 * sample) >> 31) as i32);
    }

    /// Non-power-of-two bounds stay in range across many draws.
    #[test]
    fn bounded_ints_stay_in_range() {
        let mut random = WorldgenRandom::from_seed(12345);
        for bound in [3, 5, 6, 7, 10, 11, 100] {
            for _ in 0..200 {
                let value = random.next_int_bound(bound);
                assert!((0..bound).contains(&value), "{value} out of range for bound {bound}");
            }
        }
    }

    /// `setDecorationSeed` must depend on the chunk's **block** origin.
    ///
    /// Chunk coords vs block coords is the classic way to get a whole, self-consistent, wrong
    /// world — every tree is somewhere plausible, just not where Minecraft puts it.
    #[test]
    fn the_decoration_seed_varies_with_the_block_origin() {
        let seed = 6954908675375307936i64;
        let mut a = WorldgenRandom::from_seed(0);
        let mut b = WorldgenRandom::from_seed(0);
        let mut c = WorldgenRandom::from_seed(0);
        let at_origin = a.set_decoration_seed(seed, 0, 0);
        let one_chunk_east = b.set_decoration_seed(seed, 16, 0);
        // What passing chunk coords instead of block coords would have produced.
        let wrong_units = c.set_decoration_seed(seed, 1, 0);

        assert_ne!(at_origin, one_chunk_east);
        assert_ne!(one_chunk_east, wrong_units, "block vs chunk coords must not coincide");
    }

    /// Feature seeds must separate both by feature index and by generation step.
    #[test]
    fn feature_seeds_separate_by_index_and_step() {
        let decoration = 123456789i64;
        let mut random = WorldgenRandom::from_seed(0);

        let draw = |random: &mut WorldgenRandom, index, step| {
            random.set_feature_seed(decoration, index, step);
            random.next_int()
        };

        let a = draw(&mut random, 0, 9);
        let b = draw(&mut random, 1, 9);
        let c = draw(&mut random, 0, 10);
        assert_ne!(a, b, "adjacent feature indices must not share a seed");
        assert_ne!(a, c, "adjacent steps must not share a seed");

        // `seed + index + 10000 * step` means index 10000 of step 0 lands on exactly the same
        // seed as index 0 of step 1. That collision is vanilla's, not a transcription slip, and
        // it is harmless because no step has 10000 features. Pinned so the arithmetic does not
        // get "corrected" into something that no longer matches Minecraft.
        assert_eq!(
            draw(&mut random, 10_000, 0),
            draw(&mut random, 0, 1),
            "the 10000-per-step stride is vanilla's; do not change it"
        );
    }
}
