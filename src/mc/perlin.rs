//! `ImprovedNoise` and `PerlinNoise` (octave sum) — Minecraft 1.18+ Java Edition.
//!
//! Mirrors:
//!   - `net.minecraft.world.level.levelgen.synth.ImprovedNoise`
//!   - `net.minecraft.world.level.levelgen.synth.PerlinNoise` (the non-legacy path,
//!     i.e. `PerlinNoise.create`, which seeds each octave from a positional factory)
//!
//! Two things differ from the engine's older `crate::noise` and are required for parity:
//! the `xo/yo/zo` origin offsets drawn before the permutation shuffle, and the 16-entry
//! gradient *lookup table* (Minecraft uses `SimplexNoise.GRADIENT` dot products, not the
//! branching `grad(hash, …)` of Perlin's reference code).

use super::rng::XoroshiroRandom;

/// `SimplexNoise.GRADIENT` — 12 distinct edge gradients padded to 16 for `& 15` indexing.
const GRADIENT: [[i32; 3]; 16] = [
    [1, 1, 0], [-1, 1, 0], [1, -1, 0], [-1, -1, 0],
    [1, 0, 1], [-1, 0, 1], [1, 0, -1], [-1, 0, -1],
    [0, 1, 1], [0, -1, 1], [0, 1, -1], [0, -1, -1],
    [1, 1, 0], [0, -1, 1], [-1, 1, 0], [0, -1, -1],
];

#[inline]
fn grad_dot(hash: i32, x: f64, y: f64, z: f64) -> f64 {
    let g = GRADIENT[(hash & 15) as usize];
    g[0] as f64 * x + g[1] as f64 * y + g[2] as f64 * z
}

/// `Mth.smoothstep` — the quintic fade `6t^5 - 15t^4 + 10t^3`.
#[inline]
fn smoothstep(t: f64) -> f64 {
    t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
}

#[inline]
fn lerp(t: f64, a: f64, b: f64) -> f64 {
    a + t * (b - a)
}

/// One octave of Minecraft's improved 3D gradient noise.
pub struct ImprovedNoise {
    pub xo: f64,
    pub yo: f64,
    pub zo: f64,
    p: [u8; 256],
}

impl ImprovedNoise {
    /// `new ImprovedNoise(RandomSource)` — draw the three origin offsets, then
    /// Fisher–Yates shuffle the identity permutation with `nextInt(256 - i)`.
    pub fn new(random: &mut XoroshiroRandom) -> Self {
        let xo = random.next_double() * 256.0;
        let yo = random.next_double() * 256.0;
        let zo = random.next_double() * 256.0;
        let mut p = [0u8; 256];
        for (i, slot) in p.iter_mut().enumerate() {
            *slot = i as u8;
        }
        for i in 0..256 {
            let j = random.next_int_bound(256 - i as i32) as usize;
            p.swap(i, i + j);
        }
        Self { xo, yo, zo, p }
    }

    /// `p(int)` — permutation lookup, wrapping the index into `0..256` exactly as Java's
    /// `int & 255` (which folds negatives via two's complement, matching Rust `i32 & 255`).
    #[inline]
    fn perm(&self, i: i32) -> i32 {
        self.p[(i & 255) as usize] as i32
    }

    /// `noise(x, y, z)` (the `yScale`/`yMax` = 0 case used by `NormalNoise`).
    pub fn noise(&self, x: f64, y: f64, z: f64) -> f64 {
        let ix = x + self.xo;
        let iy = y + self.yo;
        let iz = z + self.zo;
        let l = ix.floor() as i32;
        let m = iy.floor() as i32;
        let n = iz.floor() as i32;
        let o = ix - l as f64;
        let pp = iy - m as f64;
        let q = iz - n as f64;

        let a = self.perm(l).wrapping_add(m);
        let b = self.perm(l.wrapping_add(1)).wrapping_add(m);
        let aa = self.perm(a);
        let ab = self.perm(a.wrapping_add(1));
        let ba = self.perm(b);
        let bb = self.perm(b.wrapping_add(1));

        let g0 = grad_dot(self.perm(aa.wrapping_add(n)), o, pp, q);
        let g1 = grad_dot(self.perm(ba.wrapping_add(n)), o - 1.0, pp, q);
        let g2 = grad_dot(self.perm(ab.wrapping_add(n)), o, pp - 1.0, q);
        let g3 = grad_dot(self.perm(bb.wrapping_add(n)), o - 1.0, pp - 1.0, q);
        let g4 = grad_dot(self.perm(aa.wrapping_add(n).wrapping_add(1)), o, pp, q - 1.0);
        let g5 = grad_dot(self.perm(ba.wrapping_add(n).wrapping_add(1)), o - 1.0, pp, q - 1.0);
        let g6 = grad_dot(self.perm(ab.wrapping_add(n).wrapping_add(1)), o, pp - 1.0, q - 1.0);
        let g7 = grad_dot(self.perm(bb.wrapping_add(n).wrapping_add(1)), o - 1.0, pp - 1.0, q - 1.0);

        let u = smoothstep(o);
        let v = smoothstep(pp);
        let w = smoothstep(q);
        lerp(
            w,
            lerp(v, lerp(u, g0, g1), lerp(u, g2, g3)),
            lerp(v, lerp(u, g4, g5), lerp(u, g6, g7)),
        )
    }
}

/// Large period used by `PerlinNoise.wrap` to keep sample coordinates in a range where
/// `f64` precision stays high (`2^25`).
const WRAP_PERIOD: f64 = 3.355_443_2E7;

#[inline]
fn wrap(d: f64) -> f64 {
    d - (d / WRAP_PERIOD + 0.5).floor() * WRAP_PERIOD
}

/// Sum of `ImprovedNoise` octaves with `firstOctave`-derived frequency/amplitude scaling.
pub struct PerlinNoise {
    levels: Vec<Option<ImprovedNoise>>,
    amplitudes: Vec<f64>,
    lowest_freq_input_factor: f64,
    lowest_freq_value_factor: f64,
}

impl PerlinNoise {
    /// `PerlinNoise.create` — each nonzero-amplitude octave gets its own `ImprovedNoise`
    /// seeded from `factory.fromHashOf("octave_" + (firstOctave + i))`.
    pub fn create(random: &mut XoroshiroRandom, first_octave: i32, amplitudes: Vec<f64>) -> Self {
        let size = amplitudes.len();
        let factory = random.fork_positional();
        let mut levels: Vec<Option<ImprovedNoise>> = Vec::with_capacity(size);
        for (i, &amp) in amplitudes.iter().enumerate() {
            if amp != 0.0 {
                let octave = first_octave + i as i32;
                let mut r = factory.from_hash_of(&format!("octave_{octave}"));
                levels.push(Some(ImprovedNoise::new(&mut r)));
            } else {
                levels.push(None);
            }
        }
        let lowest_freq_input_factor = 2f64.powi(first_octave);
        let lowest_freq_value_factor =
            2f64.powi(size as i32 - 1) / (2f64.powi(size as i32) - 1.0);
        Self {
            levels,
            amplitudes,
            lowest_freq_input_factor,
            lowest_freq_value_factor,
        }
    }

    /// `getValue(x, y, z)` — the `yScale`/`yMax` = 0 case.
    pub fn get_value(&self, x: f64, y: f64, z: f64) -> f64 {
        let mut sum = 0.0;
        let mut input_factor = self.lowest_freq_input_factor;
        let mut value_factor = self.lowest_freq_value_factor;
        for (level, &amp) in self.levels.iter().zip(self.amplitudes.iter()) {
            if let Some(noise) = level {
                let m = noise.noise(
                    wrap(x * input_factor),
                    wrap(y * input_factor),
                    wrap(z * input_factor),
                );
                sum += amp * m * value_factor;
            }
            input_factor *= 2.0;
            value_factor /= 2.0;
        }
        sum
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EPS: f64 = 1e-11;

    #[test]
    fn improved_noise_matches_reference() {
        let mut r = XoroshiroRandom::from_seed(12345);
        let n = ImprovedNoise::new(&mut r);
        // Origin offsets, drawn before the shuffle.
        assert!((n.xo - 143.333_384_037_811_4).abs() < EPS, "xo={}", n.xo);
        assert!((n.yo - 114.374_589_611_211_4).abs() < EPS, "yo={}", n.yo);
        assert!((n.zo - 57.506_153_099_363_644).abs() < EPS, "zo={}", n.zo);
        // Permutation head (validates the shuffle indexing).
        assert_eq!(&n.p[0..8], &[4, 142, 177, 55, 130, 78, 251, 190]);
        // Sample points.
        let pts = [(0.0, 0.0, 0.0), (10.5, 20.25, -30.75), (100.1, 64.0, -200.9)];
        let expect = [
            -0.274_231_626_550_387_8,
            -0.148_125_303_843_768_87,
            0.359_310_344_864_551_16,
        ];
        for ((x, y, z), want) in pts.into_iter().zip(expect) {
            let got = n.noise(x, y, z);
            assert!((got - want).abs() < EPS, "noise({x},{y},{z})={got} want {want}");
        }
    }

    #[test]
    fn perlin_noise_matches_reference() {
        let mut r = XoroshiroRandom::from_seed(12345);
        let p = PerlinNoise::create(&mut r, -7, vec![1.0; 8]);
        let pts = [(0.0, 0.0, 0.0), (10.5, 20.25, -30.75), (1000.5, 50.0, 2000.5)];
        let expect = [
            0.102_237_823_639_447_33,
            0.141_543_885_886_613_82,
            0.177_524_426_023_511_82,
        ];
        for ((x, y, z), want) in pts.into_iter().zip(expect) {
            let got = p.get_value(x, y, z);
            assert!((got - want).abs() < EPS, "perlin({x},{y},{z})={got} want {want}");
        }
    }
}
