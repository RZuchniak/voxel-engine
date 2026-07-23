//! `NormalNoise` — Minecraft 1.18+ Java Edition
//! (`net.minecraft.world.level.levelgen.synth.NormalNoise`, the non-legacy `create`).
//!
//! The primitive that every overworld climate/terrain noise is built from: two
//! `PerlinNoise` samplers (the second sampled at a slightly shifted frequency to
//! decorrelate them), summed and scaled by a `valueFactor` derived from the octave span.

use super::perlin::PerlinNoise;
use super::rng::XoroshiroRandom;

/// The second Perlin is sampled at `INPUT_FACTOR` × the coordinates to break up the
/// visible grid alignment between the two octave stacks.
const INPUT_FACTOR: f64 = 1.018_126_888_217_522_7;

/// `NormalNoise.expectedDeviation` — normalises output roughly into `[-1, 1]`.
fn expected_deviation(octave_span: i32) -> f64 {
    0.1 * (1.0 + 1.0 / (octave_span as f64 + 1.0))
}

pub struct NormalNoise {
    first: PerlinNoise,
    second: PerlinNoise,
    value_factor: f64,
}

impl NormalNoise {
    /// `NormalNoise.create(RandomSource, firstOctave, amplitudes)`.
    pub fn create(random: &mut XoroshiroRandom, first_octave: i32, amplitudes: Vec<f64>) -> Self {
        // The two Perlin stacks consume the source in sequence.
        let first = PerlinNoise::create(random, first_octave, amplitudes.clone());
        let second = PerlinNoise::create(random, first_octave, amplitudes.clone());

        // Octave span = index distance between the lowest and highest nonzero amplitude.
        let mut min_idx = i32::MAX;
        let mut max_idx = i32::MIN;
        for (i, &amp) in amplitudes.iter().enumerate() {
            if amp != 0.0 {
                min_idx = min_idx.min(i as i32);
                max_idx = max_idx.max(i as i32);
            }
        }
        let value_factor = (1.0 / 6.0) / expected_deviation(max_idx - min_idx);

        Self { first, second, value_factor }
    }

    pub fn get_value(&self, x: f64, y: f64, z: f64) -> f64 {
        let a = self.first.get_value(x, y, z);
        let b = self
            .second
            .get_value(x * INPUT_FACTOR, y * INPUT_FACTOR, z * INPUT_FACTOR);
        (a + b) * self.value_factor
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EPS: f64 = 1e-11;

    #[test]
    fn normal_noise_matches_reference() {
        // Real usage: seed a named noise off the world seed's positional factory.
        let mut src = XoroshiroRandom::from_seed(12345);
        let factory = src.fork_positional();
        let mut r = factory.from_hash_of("minecraft:continentalness");
        let n = NormalNoise::create(&mut r, -7, vec![1.0; 8]);

        assert!((n.value_factor - 1.481_481_481_481_481_4).abs() < EPS);

        let pts = [(0.0, 0.0, 0.0), (10.5, 20.25, -30.75), (1000.5, 50.0, 2000.5)];
        let expect = [
            -0.261_090_533_253_045_76,
            -0.532_132_734_612_517_4,
            0.700_962_572_809_221_1,
        ];
        for ((x, y, z), want) in pts.into_iter().zip(expect) {
            let got = n.get_value(x, y, z);
            assert!((got - want).abs() < EPS, "normal({x},{y},{z})={got} want {want}");
        }
    }
}
