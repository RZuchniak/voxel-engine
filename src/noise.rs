//! Minecraft-style gradient Perlin noise (ImprovedNoise + octave summation).

/// Java `Random` (48-bit LCG) used to shuffle Perlin permutation tables.
struct JavaRandom {
    seed: i64,
}

impl JavaRandom {
    fn new(seed: i64) -> Self {
        const MULTIPLIER: i64 = 0x5DEECE66D;
        const MASK: i64 = (1 << 48) - 1;
        Self {
            seed: (seed ^ MULTIPLIER) & MASK,
        }
    }

    fn next_bits(&mut self, bits: u32) -> i32 {
        const MULTIPLIER: i64 = 0x5DEECE66D;
        const ADDEND: i64 = 0xB;
        const MASK: i64 = (1 << 48) - 1;
        self.seed = (self.seed.wrapping_mul(MULTIPLIER).wrapping_add(ADDEND)) & MASK;
        (self.seed >> (48 - bits)) as i32
    }

    fn next_int(&mut self, bound: i32) -> i32 {
        if bound <= 0 {
            return 0;
        }
        let bits = self.next_bits(31);
        let m = bound - 1;
        if (bound & m) == 0 {
            return ((bound as i64 * bits as i64) >> 31) as i32;
        }
        let mut r = bits;
        loop {
            let u = r % bound;
            r = self.next_bits(31);
            if r.wrapping_sub(u).wrapping_add(m) >= 0 {
                return u;
            }
        }
    }
}

/// Single octave of Ken Perlin's improved 3D gradient noise (Minecraft `ImprovedNoise`).
struct ImprovedNoise {
    p: [u8; 512],
}

impl ImprovedNoise {
    fn new(random: &mut JavaRandom) -> Self {
        let mut perm = [0u8; 256];
        for i in 0..256 {
            perm[i] = i as u8;
        }
        for i in 0..256 {
            let j = random.next_int(256 - i as i32) + i as i32;
            perm.swap(i, j as usize);
        }
        let mut p = [0u8; 512];
        for i in 0..256 {
            p[i] = perm[i];
            p[i + 256] = perm[i];
        }
        Self { p }
    }

    #[inline]
    fn fade(t: f64) -> f64 {
        t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
    }

    #[inline]
    fn lerp(t: f64, a: f64, b: f64) -> f64 {
        a + t * (b - a)
    }

    #[inline]
    fn grad(hash: u8, x: f64, y: f64, z: f64) -> f64 {
        let h = hash & 15;
        let u = if h < 8 { x } else { y };
        let v = if h < 4 {
            y
        } else if h == 12 || h == 14 {
            x
        } else {
            z
        };
        let u = if (h & 1) == 0 { u } else { -u };
        let v = if (h & 2) == 0 { v } else { -v };
        u + v
    }

    fn noise(&self, x: f64, y: f64, z: f64) -> f64 {
        let xi = x.floor() as i32 & 255;
        let yi = y.floor() as i32 & 255;
        let zi = z.floor() as i32 & 255;
        let xf = x - x.floor();
        let yf = y - y.floor();
        let zf = z - z.floor();

        let u = Self::fade(xf);
        let v = Self::fade(yf);
        let w = Self::fade(zf);

        let p = &self.p;
        let a = p[xi as usize].wrapping_add(yi as u8) as usize;
        let aa = p[a].wrapping_add(zi as u8) as usize;
        let ab = p[a + 1].wrapping_add(zi as u8) as usize;
        let b = p[(xi + 1) as usize].wrapping_add(yi as u8) as usize;
        let ba = p[b].wrapping_add(zi as u8) as usize;
        let bb = p[b + 1].wrapping_add(zi as u8) as usize;

        Self::lerp(
            w,
            Self::lerp(
                v,
                Self::lerp(
                    u,
                    Self::grad(p[aa], xf, yf, zf),
                    Self::grad(p[ba], xf - 1.0, yf, zf),
                ),
                Self::lerp(
                    u,
                    Self::grad(p[ab], xf, yf - 1.0, zf),
                    Self::grad(p[bb], xf - 1.0, yf - 1.0, zf),
                ),
            ),
            Self::lerp(
                v,
                Self::lerp(
                    u,
                    Self::grad(p[aa + 1], xf, yf, zf - 1.0),
                    Self::grad(p[ba + 1], xf - 1.0, yf, zf - 1.0),
                ),
                Self::lerp(
                    u,
                    Self::grad(p[ab + 1], xf, yf - 1.0, zf - 1.0),
                    Self::grad(p[bb + 1], xf - 1.0, yf - 1.0, zf - 1.0),
                ),
            ),
        )
    }
}

/// Fractal Perlin noise — multiple octaves with decreasing amplitude (Minecraft `PerlinNoise`).
pub struct OctavePerlinNoise {
    octaves: Vec<ImprovedNoise>,
    amplitudes: Vec<f64>,
}

impl OctavePerlinNoise {
    /// Build octaves with Minecraft-style amplitude falloff (1, 1/2, 1/4, …).
    pub fn create(seed: i64, first_octave: i32, octave_count: u32) -> Self {
        let mut random = JavaRandom::new(seed);
        let mut octaves = Vec::with_capacity(octave_count as usize);
        let mut amplitudes = Vec::with_capacity(octave_count as usize);
        let mut amp = 1.0;
        for _ in 0..octave_count {
            octaves.push(ImprovedNoise::new(&mut random));
            amplitudes.push(amp);
            amp *= 0.5;
        }
        let _ = first_octave;
        Self { octaves, amplitudes }
    }

    pub fn sample_2d(&self, x: f64, z: f64) -> f64 {
        self.sample_3d(x, 0.0, z)
    }

    pub fn sample_3d(&self, x: f64, y: f64, z: f64) -> f64 {
        let mut value = 0.0;
        let mut freq = 1.0;
        let mut amp_sum = 0.0;
        for (noise, amp) in self.octaves.iter().zip(self.amplitudes.iter()) {
            value += *amp * noise.noise(x * freq, y * freq, z * freq);
            amp_sum += amp;
            freq *= 2.0;
        }
        if amp_sum > 0.0 {
            value / amp_sum
        } else {
            0.0
        }
    }
}
