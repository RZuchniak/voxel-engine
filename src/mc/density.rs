//! A scalar density-function interpreter — the Rust equivalent of Minecraft's
//! `DensityFunction` tree (`net.minecraft.world.level.levelgen.DensityFunctions`).
//!
//! Minecraft's overworld generator is a DAG of typed density-function nodes
//! (`NoiseRouterData.overworld`). Each node computes one `f64` at a block position; the
//! root `finalDensity` decides solid-vs-air (`> 0 ⇒ solid`, before aquifers). This module
//! is the node vocabulary and a single-position `compute(x, y, z)` evaluator.
//!
//! ## Scope / what's deliberately simplified
//!
//! The wrapper nodes that exist purely for Minecraft's *array* fill path — `FlatCache`,
//! `Cache2D`, `CacheOnce`, `CacheAllInCell`, `Interpolated` — are **identity** for a
//! single scalar sample, so they are not represented: callers just inline the child. The
//! blending nodes (`BlendDensity`, `BlendAlpha`, `BlendOffset`) only do work when an
//! adjacent pre-1.18 region is being blended in; a freshly generated single-noise world
//! has no blending, so `BlendDensity` is identity, `blendAlpha ≡ 1.0`, `blendOffset ≡ 0.0`.
//! Those constants get folded in at tree-assembly time (a later module), not here.
//!
//! Two nodes are **not yet** implemented and will arrive with their own reference passes:
//! `BlendedNoise` (`base_3d_noise`) and `Spline` (`TerrainProvider` + `CubicSpline`). Terrain
//! height needs both, so this module alone can't yet produce a full column — it's the
//! backbone the remaining pieces plug into.
//!
//! Every arithmetic here is transcribed from decompiled 26.2 `DensityFunctions.java` /
//! `Mth.java`; see the per-node comments for the source method.

use std::sync::Arc;

use super::normal_noise::NormalNoise;

/// `Mth.lerp(t, a, b)`.
#[inline]
fn lerp(t: f64, a: f64, b: f64) -> f64 {
    a + t * (b - a)
}

/// `Mth.clampedLerp` — lerp with the factor clamped to `[0, 1]` (no extrapolation).
#[inline]
fn clamped_lerp(factor: f64, min: f64, max: f64) -> f64 {
    if factor < 0.0 {
        min
    } else if factor > 1.0 {
        max
    } else {
        lerp(factor, min, max)
    }
}

/// `Mth.clampedMap` — remap `value` from `[from_min, from_max]` to `[to_min, to_max]`,
/// clamped at the ends. Used by `YClampedGradient`.
#[inline]
fn clamped_map(value: f64, from_min: f64, from_max: f64, to_min: f64, to_max: f64) -> f64 {
    // Mth.inverseLerp then clampedLerp.
    clamped_lerp((value - from_min) / (from_max - from_min), to_min, to_max)
}

/// One node of the density-function tree. Children are boxed so the tree is a plain owned
/// value; noise samplers are shared via `Arc` (the same `NormalNoise` feeds many nodes).
#[derive(Clone)]
pub enum Df {
    /// `DensityFunctions.constant` — a fixed value.
    Constant(f64),

    /// `YClampedGradient` — `clampedMap(y, from_y, to_y, from_value, to_value)`.
    YClampedGradient { from_y: f64, to_y: f64, from_value: f64, to_value: f64 },

    /// `Ap2 ADD` — `a + b`.
    Add(Box<Df>, Box<Df>),
    /// `Ap2 MUL` — `a * b` (Java short-circuits `a==0 → 0`; same value for finite inputs).
    Mul(Box<Df>, Box<Df>),
    /// `Ap2 MIN` — `min(a, b)`.
    Min(Box<Df>, Box<Df>),
    /// `Ap2 MAX` — `max(a, b)`.
    Max(Box<Df>, Box<Df>),

    /// `Mapped ABS` — `|a|`.
    Abs(Box<Df>),
    /// `Mapped SQUARE` — `a²`.
    Square(Box<Df>),
    /// `Mapped CUBE` — `a³`.
    Cube(Box<Df>),
    /// `Mapped HALF_NEGATIVE` — `a>0 ? a : a*0.5`.
    HalfNegative(Box<Df>),
    /// `Mapped QUARTER_NEGATIVE` — `a>0 ? a : a*0.25`.
    QuarterNegative(Box<Df>),

    /// `Clamp` — `clamp(a, min, max)`.
    Clamp { input: Box<Df>, min: f64, max: f64 },

    /// `Squeeze` — clamp to ±1 then `c/2 - c³/24`.
    Squeeze(Box<Df>),

    /// `RangeChoice` — if `input ∈ [min_inclusive, max_exclusive)` use `in_range`, else `out_of_range`.
    RangeChoice {
        input: Box<Df>,
        min_inclusive: f64,
        max_exclusive: f64,
        in_range: Box<Df>,
        out_of_range: Box<Df>,
    },

    /// `Noise` — `noise.getValue(x·xz_scale, y·y_scale, z·xz_scale)`.
    Noise { noise: Arc<NormalNoise>, xz_scale: f64, y_scale: f64 },

    /// `ShiftedNoise` — `noise.getValue(x·xz_scale + shift_x, y·y_scale + shift_y, z·xz_scale + shift_z)`.
    /// (`shiftedNoise2d` sets `shift_y = zero` and `y_scale = 0`.)
    ShiftedNoise {
        shift_x: Box<Df>,
        shift_y: Box<Df>,
        shift_z: Box<Df>,
        xz_scale: f64,
        y_scale: f64,
        noise: Arc<NormalNoise>,
    },

    /// `ShiftA` — `noise.getValue(x·0.25, 0, z·0.25) · 4` (see `ShiftNoise.compute`).
    ShiftA(Arc<NormalNoise>),
    /// `ShiftB` — `noise.getValue(z·0.25, x·0.25, 0) · 4`.
    ShiftB(Arc<NormalNoise>),
}

impl Df {
    // ---- ergonomic constructors so tree assembly reads like the Java builder ----

    pub fn constant(v: f64) -> Df {
        Df::Constant(v)
    }
    pub fn zero() -> Df {
        Df::Constant(0.0)
    }
    pub fn add(a: Df, b: Df) -> Df {
        Df::Add(Box::new(a), Box::new(b))
    }
    pub fn mul(a: Df, b: Df) -> Df {
        Df::Mul(Box::new(a), Box::new(b))
    }
    pub fn min(a: Df, b: Df) -> Df {
        Df::Min(Box::new(a), Box::new(b))
    }
    pub fn max(a: Df, b: Df) -> Df {
        Df::Max(Box::new(a), Box::new(b))
    }
    pub fn abs(self) -> Df {
        Df::Abs(Box::new(self))
    }
    pub fn square(self) -> Df {
        Df::Square(Box::new(self))
    }
    pub fn cube(self) -> Df {
        Df::Cube(Box::new(self))
    }
    pub fn half_negative(self) -> Df {
        Df::HalfNegative(Box::new(self))
    }
    pub fn quarter_negative(self) -> Df {
        Df::QuarterNegative(Box::new(self))
    }
    pub fn clamp(self, min: f64, max: f64) -> Df {
        Df::Clamp { input: Box::new(self), min, max }
    }
    pub fn squeeze(self) -> Df {
        Df::Squeeze(Box::new(self))
    }
    pub fn y_clamped_gradient(from_y: f64, to_y: f64, from_value: f64, to_value: f64) -> Df {
        Df::YClampedGradient { from_y, to_y, from_value, to_value }
    }
    pub fn range_choice(input: Df, min_inclusive: f64, max_exclusive: f64, in_range: Df, out_of_range: Df) -> Df {
        Df::RangeChoice {
            input: Box::new(input),
            min_inclusive,
            max_exclusive,
            in_range: Box::new(in_range),
            out_of_range: Box::new(out_of_range),
        }
    }
    pub fn noise(noise: Arc<NormalNoise>, xz_scale: f64, y_scale: f64) -> Df {
        Df::Noise { noise, xz_scale, y_scale }
    }
    /// `DensityFunctions.shiftedNoise2d(shift_x, shift_z, xz_scale, noise)`.
    pub fn shifted_noise_2d(shift_x: Df, shift_z: Df, xz_scale: f64, noise: Arc<NormalNoise>) -> Df {
        Df::ShiftedNoise {
            shift_x: Box::new(shift_x),
            shift_y: Box::new(Df::zero()),
            shift_z: Box::new(shift_z),
            xz_scale,
            y_scale: 0.0,
            noise,
        }
    }
    /// `DensityFunctions.mappedNoise(noise, xz_scale, y_scale, min_target, max_target)` —
    /// a `Noise` node linearly remapped from unit range to `[min_target, max_target]`.
    pub fn mapped_noise(noise: Arc<NormalNoise>, xz_scale: f64, y_scale: f64, min_target: f64, max_target: f64) -> Df {
        let middle = (min_target + max_target) * 0.5;
        let factor = (max_target - min_target) * 0.5;
        Df::add(Df::constant(middle), Df::mul(Df::constant(factor), Df::noise(noise, xz_scale, y_scale)))
    }

    /// Evaluate this node at a block position. `x`/`y`/`z` are block coordinates as `f64`
    /// (Java's `FunctionContext.blockX/Y/Z()` are ints promoted to double at each use).
    pub fn compute(&self, x: f64, y: f64, z: f64) -> f64 {
        match self {
            Df::Constant(v) => *v,

            Df::YClampedGradient { from_y, to_y, from_value, to_value } => {
                clamped_map(y, *from_y, *to_y, *from_value, *to_value)
            }

            Df::Add(a, b) => a.compute(x, y, z) + b.compute(x, y, z),
            Df::Mul(a, b) => {
                let v = a.compute(x, y, z);
                if v == 0.0 { 0.0 } else { v * b.compute(x, y, z) }
            }
            Df::Min(a, b) => a.compute(x, y, z).min(b.compute(x, y, z)),
            Df::Max(a, b) => a.compute(x, y, z).max(b.compute(x, y, z)),

            Df::Abs(a) => a.compute(x, y, z).abs(),
            Df::Square(a) => {
                let v = a.compute(x, y, z);
                v * v
            }
            Df::Cube(a) => {
                let v = a.compute(x, y, z);
                v * v * v
            }
            Df::HalfNegative(a) => {
                let v = a.compute(x, y, z);
                if v > 0.0 { v } else { v * 0.5 }
            }
            Df::QuarterNegative(a) => {
                let v = a.compute(x, y, z);
                if v > 0.0 { v } else { v * 0.25 }
            }

            Df::Clamp { input, min, max } => input.compute(x, y, z).clamp(*min, *max),

            Df::Squeeze(a) => {
                let c = a.compute(x, y, z).clamp(-1.0, 1.0);
                c / 2.0 - c * c * c / 24.0
            }

            Df::RangeChoice { input, min_inclusive, max_exclusive, in_range, out_of_range } => {
                let v = input.compute(x, y, z);
                if v >= *min_inclusive && v < *max_exclusive {
                    in_range.compute(x, y, z)
                } else {
                    out_of_range.compute(x, y, z)
                }
            }

            Df::Noise { noise, xz_scale, y_scale } => {
                noise.get_value(x * xz_scale, y * y_scale, z * xz_scale)
            }

            Df::ShiftedNoise { shift_x, shift_y, shift_z, xz_scale, y_scale, noise } => {
                let nx = x * xz_scale + shift_x.compute(x, y, z);
                let ny = y * y_scale + shift_y.compute(x, y, z);
                let nz = z * xz_scale + shift_z.compute(x, y, z);
                noise.get_value(nx, ny, nz)
            }

            // ShiftNoise.compute(localX, localY, localZ) = noise.getValue(l*0.25...) * 4.
            // ShiftA feeds (x, 0, z); ShiftB feeds (z, x, 0).
            Df::ShiftA(noise) => noise.get_value(x * 0.25, 0.0, z * 0.25) * 4.0,
            Df::ShiftB(noise) => noise.get_value(z * 0.25, x * 0.25, 0.0) * 4.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mc::noise_params::{seed_factory, Noise};

    const SEED: i64 = 6954908675375307936;

    #[test]
    fn arithmetic_and_transforms() {
        // (2 + 3) * 4 = 20
        let f = Df::mul(Df::add(Df::constant(2.0), Df::constant(3.0)), Df::constant(4.0));
        assert_eq!(f.compute(0.0, 0.0, 0.0), 20.0);

        assert_eq!(Df::constant(-5.0).abs().compute(0.0, 0.0, 0.0), 5.0);
        assert_eq!(Df::constant(3.0).square().compute(0.0, 0.0, 0.0), 9.0);
        assert_eq!(Df::constant(-2.0).cube().compute(0.0, 0.0, 0.0), -8.0);
        assert_eq!(Df::constant(-4.0).half_negative().compute(0.0, 0.0, 0.0), -2.0);
        assert_eq!(Df::constant(4.0).half_negative().compute(0.0, 0.0, 0.0), 4.0);
        assert_eq!(Df::constant(-4.0).quarter_negative().compute(0.0, 0.0, 0.0), -1.0);
        assert_eq!(Df::constant(10.0).clamp(-1.0, 1.0).compute(0.0, 0.0, 0.0), 1.0);
        assert_eq!(Df::min(Df::constant(1.0), Df::constant(2.0)).compute(0.0, 0.0, 0.0), 1.0);
        assert_eq!(Df::max(Df::constant(1.0), Df::constant(2.0)).compute(0.0, 0.0, 0.0), 2.0);
    }

    #[test]
    fn squeeze_matches_formula() {
        // At c=1: 1/2 - 1/24 = 0.4583333...
        let got = Df::constant(1.0).squeeze().compute(0.0, 0.0, 0.0);
        assert!((got - (0.5 - 1.0 / 24.0)).abs() < 1e-12);
        // Clamps first: c=2 behaves like c=1.
        let got2 = Df::constant(2.0).squeeze().compute(0.0, 0.0, 0.0);
        assert_eq!(got, got2);
        // Odd function: squeeze(-1) = -squeeze(1).
        let neg = Df::constant(-1.0).squeeze().compute(0.0, 0.0, 0.0);
        assert!((neg + got).abs() < 1e-12);
    }

    #[test]
    fn y_clamped_gradient_is_clamped_linear() {
        // depth's y term: yClampedGradient(-64, 320, 1.5, -1.5).
        let g = Df::y_clamped_gradient(-64.0, 320.0, 1.5, -1.5);
        assert_eq!(g.compute(0.0, -64.0, 0.0), 1.5); // at/below from_y
        assert_eq!(g.compute(0.0, 320.0, 0.0), -1.5); // at/above to_y
        assert_eq!(g.compute(0.0, -1000.0, 0.0), 1.5); // clamped below
        assert_eq!(g.compute(0.0, 1000.0, 0.0), -1.5); // clamped above
        // midpoint y=128 → halfway → 0.0
        let mid = g.compute(0.0, 128.0, 0.0);
        assert!((mid - 0.0).abs() < 1e-9, "mid={mid}");
    }

    #[test]
    fn range_choice_selects_branch() {
        let f = Df::range_choice(
            Df::constant(0.5),
            0.0,
            1.0,
            Df::constant(100.0),
            Df::constant(-100.0),
        );
        assert_eq!(f.compute(0.0, 0.0, 0.0), 100.0);
        // max is exclusive: input == max_exclusive → out of range.
        let f2 = Df::range_choice(Df::constant(1.0), 0.0, 1.0, Df::constant(100.0), Df::constant(-100.0));
        assert_eq!(f2.compute(0.0, 0.0, 0.0), -100.0);
    }

    #[test]
    fn noise_node_delegates_to_sampler() {
        // A `Noise` node with unit scales must equal the raw sampler at the same point.
        let factory = seed_factory(SEED);
        let cont = Arc::new(Noise::Continentalness.instantiate(&factory));
        let node = Df::noise(cont.clone(), 1.0, 1.0);
        for (x, y, z) in [(0.0, 0.0, 0.0), (12.0, -5.0, 30.0), (1000.0, 60.0, -700.0)] {
            assert_eq!(node.compute(x, y, z), cont.get_value(x, y, z));
        }
    }

    #[test]
    fn shift_a_b_use_offset_noise_scaled() {
        // ShiftA/ShiftB sample the SHIFT ("offset") noise at quarter coords, times 4.
        let factory = seed_factory(SEED);
        let shift = Arc::new(Noise::Shift.instantiate(&factory));
        let (x, z) = (25.0, -40.0);
        let a = Df::ShiftA(shift.clone()).compute(x, 0.0, z);
        assert_eq!(a, shift.get_value(x * 0.25, 0.0, z * 0.25) * 4.0);
        let b = Df::ShiftB(shift.clone()).compute(x, 0.0, z);
        assert_eq!(b, shift.get_value(z * 0.25, x * 0.25, 0.0) * 4.0);
    }
}
