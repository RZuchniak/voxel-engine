//! `BlendedNoise` — the `base_3d_noise` density function
//! (`net.minecraft.world.level.levelgen.synth.BlendedNoise`).
//!
//! This is the old "min/max/main" interpolated noise that still supplies the 3D
//! roughness under the density-function terrain. It is built from three *legacy*
//! `PerlinNoise` stacks (16 + 16 + 8 octaves) seeded sequentially from one random source,
//! and its per-octave sampling uses the y-smearing `ImprovedNoise::noise_smeared`.
//!
//! Seeding (per `RandomState`): the overworld `base_3d_noise` is `createUnseeded` at
//! registry time, then re-seeded via `withNewRandom(random.fromHashOf("minecraft:terrain"))`
//! where `random = XoroshiroRandomSource(seed).forkPositional()`. So the terrain random is
//! `seed_factory(seed).from_hash_of("minecraft:terrain")`; the three Perlin stacks consume
//! it in order (min, then max, then main). See [`BlendedNoise::overworld`].

use super::perlin::{wrap, ImprovedNoise};
use super::rng::XoroshiroRandom;

/// A legacy octave stack: `ImprovedNoise`s in *construction order*, which is exactly what
/// `PerlinNoise.getOctaveNoise(i)` returns for the full (all-amplitude-1) octave sets that
/// `createLegacyForBlendedNoise` builds. Octave `i` is used at input factor `2^-i`.
struct LegacyOctaves {
    octaves: Vec<ImprovedNoise>,
}

impl LegacyOctaves {
    /// `PerlinNoise.createLegacyForBlendedNoise(random, rangeClosed(first_octave, 0))`.
    ///
    /// The octave set spans `first_octave..=0` with every amplitude 1.0, so the legacy
    /// initialisation constructs `count = -first_octave + 1` `ImprovedNoise`s straight off
    /// `random` (no skipped/absent octaves). `getOctaveNoise(i)` then reverses the storage,
    /// which cancels out to "the i-th one constructed" — so we keep construction order and
    /// index directly.
    fn create(random: &mut XoroshiroRandom, first_octave: i32) -> Self {
        let count = (-first_octave + 1) as usize;
        let octaves = (0..count).map(|_| ImprovedNoise::new(random)).collect();
        Self { octaves }
    }

    #[inline]
    fn octave(&self, i: usize) -> &ImprovedNoise {
        &self.octaves[i]
    }
}

pub struct BlendedNoise {
    min_limit: LegacyOctaves,
    max_limit: LegacyOctaves,
    main: LegacyOctaves,
    xz_multiplier: f64,
    y_multiplier: f64,
    xz_factor: f64,
    y_factor: f64,
    smear_scale_multiplier: f64,
}

impl BlendedNoise {
    /// Construct with an explicit random and the five shape parameters
    /// (`xzScale, yScale, xzFactor, yFactor, smearScaleMultiplier`), matching the
    /// `BlendedNoise(RandomSource, …)` constructor.
    pub fn new(
        random: &mut XoroshiroRandom,
        xz_scale: f64,
        y_scale: f64,
        xz_factor: f64,
        y_factor: f64,
        smear_scale_multiplier: f64,
    ) -> Self {
        // Order matters: min, then max, then main all draw from the same source.
        let min_limit = LegacyOctaves::create(random, -15);
        let max_limit = LegacyOctaves::create(random, -15);
        let main = LegacyOctaves::create(random, -7);
        Self {
            min_limit,
            max_limit,
            main,
            xz_multiplier: 684.412 * xz_scale,
            y_multiplier: 684.412 * y_scale,
            xz_factor,
            y_factor,
            smear_scale_multiplier,
        }
    }

    /// The overworld `base_3d_noise`: `createUnseeded(0.25, 0.125, 80.0, 160.0, 8.0)`
    /// re-seeded with `fromHashOf("minecraft:terrain")` (`RandomState`).
    pub fn overworld(terrain_random: &mut XoroshiroRandom) -> Self {
        Self::new(terrain_random, 0.25, 0.125, 80.0, 160.0, 8.0)
    }

    /// `BlendedNoise.compute` — the min/max/main interpolated sample at a block position.
    pub fn compute(&self, x: f64, y: f64, z: f64) -> f64 {
        let limit_x = x * self.xz_multiplier;
        let limit_y = y * self.y_multiplier;
        let limit_z = z * self.xz_multiplier;
        let main_x = limit_x / self.xz_factor;
        let main_y = limit_y / self.y_factor;
        let main_z = limit_z / self.xz_factor;
        let limit_smear = self.y_multiplier * self.smear_scale_multiplier;
        let main_smear = limit_smear / self.y_factor;

        // Main noise: 8 octaves, pow starts at 1 and halves.
        let mut main_value = 0.0;
        let mut pow = 1.0;
        for i in 0..8 {
            let noise = self.main.octave(i);
            main_value += noise.noise_smeared(
                wrap(main_x * pow),
                wrap(main_y * pow),
                wrap(main_z * pow),
                main_smear * pow,
                main_y * pow,
            ) / pow;
            pow /= 2.0;
        }

        let factor = (main_value / 10.0 + 1.0) / 2.0;
        let is_max = factor >= 1.0;
        let is_min = factor <= 0.0;

        // Min / max limit noises: 16 octaves each; skip whichever the factor makes irrelevant.
        let mut blend_min = 0.0;
        let mut blend_max = 0.0;
        pow = 1.0;
        for i in 0..16 {
            let wx = wrap(limit_x * pow);
            let wy = wrap(limit_y * pow);
            let wz = wrap(limit_z * pow);
            let y_scale_pow = limit_smear * pow;
            if !is_max {
                blend_min += self.min_limit.octave(i).noise_smeared(wx, wy, wz, y_scale_pow, limit_y * pow) / pow;
            }
            if !is_min {
                blend_max += self.max_limit.octave(i).noise_smeared(wx, wy, wz, y_scale_pow, limit_y * pow) / pow;
            }
            pow /= 2.0;
        }

        // clampedLerp(factor, blendMin/512, blendMax/512) / 128
        clamped_lerp(factor, blend_min / 512.0, blend_max / 512.0) / 128.0
    }
}

/// `Mth.clampedLerp`.
#[inline]
fn clamped_lerp(factor: f64, min: f64, max: f64) -> f64 {
    if factor < 0.0 {
        min
    } else if factor > 1.0 {
        max
    } else {
        min + factor * (max - min)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mc::noise_params::seed_factory;

    const SEED: i64 = 6954908675375307936;

    fn overworld_for_seed(seed: i64) -> BlendedNoise {
        let mut terrain = seed_factory(seed).from_hash_of("minecraft:terrain");
        BlendedNoise::overworld(&mut terrain)
    }

    #[test]
    fn deterministic_and_bounded() {
        let bn = overworld_for_seed(SEED);
        // base_3d_noise output is small (~±1 after the /128); sanity-bound it.
        for (x, y, z) in [(0.0, 64.0, 0.0), (100.0, 0.0, -50.0), (-2000.0, 200.0, 1500.0)] {
            let a = bn.compute(x, y, z);
            let b = bn.compute(x, y, z);
            assert_eq!(a, b, "must be deterministic");
            assert!(a.abs() < 5.0, "base_3d_noise {a} unexpectedly large at ({x},{y},{z})");
        }
    }

    #[test]
    fn octave_counts() {
        let bn = overworld_for_seed(SEED);
        assert_eq!(bn.min_limit.octaves.len(), 16);
        assert_eq!(bn.max_limit.octaves.len(), 16);
        assert_eq!(bn.main.octaves.len(), 8);
    }

    #[test]
    fn smear_reduces_to_plain_noise_when_yscale_zero() {
        // With yScale=0 the smeared sampler must equal the plain 3-arg noise.
        let mut r = seed_factory(SEED).from_hash_of("minecraft:terrain");
        let n = ImprovedNoise::new(&mut r);
        for (x, y, z) in [(1.5, 2.5, 3.5), (-10.25, 4.0, 7.75)] {
            assert_eq!(n.noise(x, y, z), n.noise_smeared(x, y, z, 0.0, 0.0));
        }
    }
}
