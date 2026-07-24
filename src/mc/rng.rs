//! Minecraft 1.18+ random-number generation: `Xoroshiro128PlusPlus` and the
//! `XoroshiroRandomSource` / positional-factory seeding used by worldgen.
//!
//! Mirrors, method for method:
//!   - `net.minecraft.world.level.levelgen.Xoroshiro128PlusPlus`
//!   - `net.minecraft.util.RandomSupport` (`upgradeSeedTo128bit`, `mixStafford13`)
//!   - `net.minecraft.world.level.levelgen.XoroshiroRandomSource` and its
//!     `XoroshiroPositionalRandomFactory` (`forkPositional`, `fromHashOf`)
//!
//! Java `long` is signed 64-bit; here state is kept as `u64` and reinterpreted as `i64`
//! at the API boundary, which reproduces Java's wrapping arithmetic exactly.

use md5::{Digest, Md5};

// Named to match Mojang's `RandomSupport` constants (verified against decompiled 26.2):
//   GOLDEN_RATIO_64 = -7046029254386353131, SILVER_RATIO_64 = 7640891576956012809.
const GOLDEN_RATIO_64: u64 = 0x9E37_79B9_7F4A_7C15;
const SILVER_RATIO_64: u64 = 0x6A09_E667_F3BC_C909;

/// SplitMix64 finalizer, "Stafford variant 13" — `RandomSupport.mixStafford13`.
fn mix_stafford13(mut z: u64) -> u64 {
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// `RandomSupport.upgradeSeedTo128bit` — expand a 64-bit world seed to the (lo, hi) pair.
fn upgrade_seed_to_128(seed: i64) -> (u64, u64) {
    let lo = (seed as u64) ^ SILVER_RATIO_64;
    let hi = lo.wrapping_add(GOLDEN_RATIO_64);
    (mix_stafford13(lo), mix_stafford13(hi))
}

/// Minecraft's worldgen PRNG (`Xoroshiro128PlusPlus`) plus the `RandomSource` surface
/// (`nextInt`, `nextDouble`, …) that `XoroshiroRandomSource` layers on top.
#[derive(Clone)]
pub struct XoroshiroRandom {
    lo: u64,
    hi: u64,
}

impl XoroshiroRandom {
    /// Construct from a raw 128-bit state. Matches the Java constructor's guard: an
    /// all-zero state is replaced by a fixed nonzero seed.
    pub fn from_state(mut lo: u64, mut hi: u64) -> Self {
        if (lo | hi) == 0 {
            lo = GOLDEN_RATIO_64;
            hi = SILVER_RATIO_64;
        }
        Self { lo, hi }
    }

    /// `XoroshiroRandomSource(long seed)` — the seed-initialised source.
    pub fn from_seed(seed: i64) -> Self {
        let (lo, hi) = upgrade_seed_to_128(seed);
        Self::from_state(lo, hi)
    }

    /// One xoroshiro128++ step. Equivalent to Java `Xoroshiro128PlusPlus.nextLong()`.
    pub fn next_long(&mut self) -> i64 {
        let l = self.lo;
        let m = self.hi;
        let n = l.wrapping_add(m).rotate_left(17).wrapping_add(l);
        let m = m ^ l;
        self.lo = l.rotate_left(49) ^ m ^ (m << 21);
        self.hi = m.rotate_left(28);
        n as i64
    }

    /// `nextInt()` = low 32 bits of `nextLong()`.
    pub fn next_int(&mut self) -> i32 {
        self.next_long() as i32
    }

    /// `nextDouble()` — 53 high bits scaled to `[0, 1)`.
    pub fn next_double(&mut self) -> f64 {
        // Java: `(nextLong() >>> 11) * 0x1.0p-53`.
        ((self.next_long() as u64) >> 11) as f64 * 1.110_223_024_625_156_5E-16
    }

    /// `nextBits(int)` — top `bits` bits of `nextLong()`, unsigned.
    pub fn next_bits(&mut self, bits: u32) -> u64 {
        (self.next_long() as u64) >> (64 - bits)
    }

    /// `nextInt(int bound)` — Lemire's bounded reduction, matching Java exactly.
    /// `bound` must be positive (as in vanilla worldgen).
    pub fn next_int_bound(&mut self, bound: i32) -> i32 {
        debug_assert!(bound > 0, "bound must be positive");
        let b = bound as u64;
        let mut l = (self.next_int() as u32) as u64; // Integer.toUnsignedLong
        let mut m = l * b;
        let mut n = m & 0xFFFF_FFFF;
        if n < b {
            // Integer.remainderUnsigned(-bound, bound)
            let threshold = ((bound as u32).wrapping_neg() % bound as u32) as u64;
            while n < threshold {
                l = (self.next_int() as u32) as u64;
                m = l * b;
                n = m & 0xFFFF_FFFF;
            }
        }
        (m >> 32) as i32
    }

    /// `forkPositional()` — a factory whose seed is two fresh `nextLong()` draws.
    pub fn fork_positional(&mut self) -> PositionalFactory {
        let lo = self.next_long() as u64;
        let hi = self.next_long() as u64;
        PositionalFactory { lo, hi }
    }
}

/// `XoroshiroPositionalRandomFactory` — seeds independent noises by name.
#[derive(Clone, Copy)]
pub struct PositionalFactory {
    lo: u64,
    hi: u64,
}

impl PositionalFactory {
    /// `fromHashOf(String)` — MD5 the name, split into two big-endian longs, XOR with
    /// the factory seed. This is how each worldgen noise (continentalness, erosion, …)
    /// gets its own reproducible stream from the world seed.
    pub fn from_hash_of(&self, name: &str) -> XoroshiroRandom {
        let digest = Md5::digest(name.as_bytes());
        let l = u64::from_be_bytes(digest[0..8].try_into().unwrap());
        let m = u64::from_be_bytes(digest[8..16].try_into().unwrap());
        XoroshiroRandom::from_state(l ^ self.lo, m ^ self.hi)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // All expected values below were produced by an independent BigInt reference
    // implementation of the same Java algorithms (see scratchpad `mc-rng-ref.mjs`).
    // They pin this Rust port to the spec; validation against a *real* exported world
    // is the separate ground-truth-oracle step.

    #[test]
    fn stafford_mix_known_values() {
        assert_eq!(mix_stafford13(0) as i64, 0);
        assert_eq!(mix_stafford13(1) as i64, 6_238_072_747_940_578_789);
    }

    #[test]
    fn upgrade_seed_known_values() {
        let cases: [(i64, i64, i64); 4] = [
            (0, 3_847_398_142_028_685_078, 7_192_185_014_346_937_746),
            (1, 5_272_463_233_947_570_727, 1_927_618_558_350_093_866),
            (12345, 733_019_005_196_230_046, -3_494_074_583_369_400_597),
            (-1, -110_783_831_392_733_308, 2_932_223_646_667_407_290),
        ];
        for (seed, lo, hi) in cases {
            let (glo, ghi) = upgrade_seed_to_128(seed);
            assert_eq!(glo as i64, lo, "lo for seed {seed}");
            assert_eq!(ghi as i64, hi, "hi for seed {seed}");
        }
    }

    #[test]
    fn next_long_sequences() {
        let expect_0: [i64; 5] = [
            3_038_984_756_725_240_190,
            -3_694_039_286_755_638_414,
            4_633_751_808_701_151_732,
            2_160_572_957_309_072_155,
            1_839_370_574_944_072_389,
        ];
        let expect_12345: [i64; 5] = [
            -8_118_485_274_630_516_485,
            8_241_557_746_459_281_790,
            4_143_755_034_716_878_659,
            1_226_499_899_398_695_337,
            -8_052_343_703_659_247_148,
        ];
        let expect_neg1: [i64; 5] = [
            -8_676_505_878_415_342_125,
            -868_585_888_688_873_692,
            -6_331_679_347_063_163_302,
            -2_068_491_455_652_362_927,
            -5_626_054_917_968_568_837,
        ];
        for (seed, expect) in [(0i64, expect_0), (12345, expect_12345), (-1, expect_neg1)] {
            let mut r = XoroshiroRandom::from_seed(seed);
            for (i, want) in expect.iter().enumerate() {
                assert_eq!(r.next_long(), *want, "next_long[{i}] for seed {seed}");
            }
        }
    }

    #[test]
    fn next_int_double_and_bounded() {
        let mut r = XoroshiroRandom::from_seed(12345);
        let ints: [i32; 5] = [57_184_507, -778_892_930, -527_878_333, 79_047_081, -1_911_977_516];
        for (i, want) in ints.iter().enumerate() {
            assert_eq!(r.next_int(), *want, "next_int[{i}]");
        }

        let mut r = XoroshiroRandom::from_seed(12345);
        let doubles = [
            0.559_896_031_397_700_8,
            0.446_775_740_668_794_55,
            0.224_633_410_544_389_23,
            0.066_488_692_774_065_88,
            0.563_481_573_144_633_5,
        ];
        for (i, want) in doubles.iter().enumerate() {
            assert_eq!(r.next_double(), *want, "next_double[{i}]");
        }

        let mut r = XoroshiroRandom::from_seed(12345);
        let b100: [i32; 8] = [1, 81, 87, 1, 55, 69, 20, 50];
        for (i, want) in b100.iter().enumerate() {
            assert_eq!(r.next_int_bound(100), *want, "next_int_bound(100)[{i}]");
        }

        let mut r = XoroshiroRandom::from_seed(12345);
        let b256: [i32; 8] = [3, 209, 224, 4, 142, 177, 52, 128];
        for (i, want) in b256.iter().enumerate() {
            assert_eq!(r.next_int_bound(256), *want, "next_int_bound(256)[{i}]");
        }
    }

    #[test]
    fn named_noise_seeding_matches_forkpositional_chain() {
        // World seed 12345 → forkPositional() → fromHashOf(noise name).
        let mut src = XoroshiroRandom::from_seed(12345);
        let factory = src.fork_positional();
        // forkPositional draws the first two longs of the seed's stream.
        assert_eq!(factory.lo as i64, -8_118_485_274_630_516_485);
        assert_eq!(factory.hi as i64, 8_241_557_746_459_281_790);

        let cases: [(&str, [i64; 3]); 3] = [
            (
                "minecraft:continentalness",
                [3_513_050_629_383_982_151, 2_227_727_584_598_759_204, 3_142_180_571_832_942_672],
            ),
            (
                "minecraft:erosion",
                [6_519_630_900_402_792_865, 6_332_896_244_760_641_902, -8_751_844_340_344_567_402],
            ),
            (
                "minecraft:temperature",
                [5_634_266_678_086_618_857, -6_651_648_227_214_885_851, -1_116_085_796_420_308_947],
            ),
        ];
        for (name, expect) in cases {
            let mut r = factory.from_hash_of(name);
            for (i, want) in expect.iter().enumerate() {
                assert_eq!(r.next_long(), *want, "{name} next_long[{i}]");
            }
        }
    }
}
