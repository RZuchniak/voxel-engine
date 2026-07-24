//! Assembling the overworld terrain density (`NoiseRouterData.overworld` +
//! `registerTerrainNoises`), seeded from a real world seed.
//!
//! This ties the pieces together — shifted climate noises, the offset/factor/jaggedness
//! splines, `BlendedNoise` — into the `slopedCheese` density and the slided/squeezed
//! terrain density that decides solid-vs-air.
//!
//! ## What's included vs deferred
//!
//! Included: the full pre-cave terrain column — climate coordinates, `offset`/`factor`/
//! `depth`/`jaggedness`, `slopedCheese = initialDensity + base_3d_noise`, the overworld
//! y-slide, and the `squeeze` post-process. `finalDensity > 0 ⇒ solid`.
//!
//! Deferred (documented, not yet ported): the cave carving that `NoiseRouterData.overworld`
//! layers on via `rangeChoice(slopedCheese, …, underground(…))` + `NOODLE` + `entrances`,
//! the aquifer fluid picker, and surface rules (grass/dirt/sand vs stone). So this predicts
//! the terrain *envelope* (surface altitude and solid body) but not cave voids or the exact
//! surface block. Surface altitude is the first thing to validate against the oracle.

use std::sync::Arc;

use super::blended_noise::BlendedNoise;
use super::noise_params::{seed_factory, Noise};
use super::normal_noise::NormalNoise;
use super::spline::{self, Spline, SplineInput};

/// `constant(-0.50375F)` — the float literal is promoted to double in Java, so keep the
/// exact widened value rather than the f64 literal `-0.50375`.
const GLOBAL_OFFSET: f64 = -0.50375f32 as f64;

pub struct Overworld {
    shift: Arc<NormalNoise>,
    continentalness: Arc<NormalNoise>,
    erosion: Arc<NormalNoise>,
    ridge: Arc<NormalNoise>,
    jagged: Arc<NormalNoise>,
    base_3d: BlendedNoise,
    offset_spline: Spline,
    factor_spline: Spline,
    jaggedness_spline: Spline,
}

impl Overworld {
    pub fn new(seed: i64) -> Self {
        let factory = seed_factory(seed);
        // base_3d_noise is re-seeded from fromHashOf("minecraft:terrain") (RandomState).
        let mut terrain_random = factory.from_hash_of("minecraft:terrain");
        Self {
            shift: Arc::new(Noise::Shift.instantiate(&factory)),
            continentalness: Arc::new(Noise::Continentalness.instantiate(&factory)),
            erosion: Arc::new(Noise::Erosion.instantiate(&factory)),
            ridge: Arc::new(Noise::Ridge.instantiate(&factory)),
            jagged: Arc::new(Noise::Jagged.instantiate(&factory)),
            base_3d: BlendedNoise::overworld(&mut terrain_random),
            offset_spline: spline::overworld_offset(),
            factor_spline: spline::overworld_factor(),
            jaggedness_spline: spline::overworld_jaggedness(),
        }
    }

    // ---- shift (the coordinate warp shared by every 2d climate noise) ----

    #[inline]
    fn shift_x(&self, x: f64, z: f64) -> f64 {
        // ShiftA: offsetNoise.getValue(x·0.25, 0, z·0.25) · 4.
        self.shift.get_value(x * 0.25, 0.0, z * 0.25) * 4.0
    }
    #[inline]
    fn shift_z(&self, x: f64, z: f64) -> f64 {
        // ShiftB: offsetNoise.getValue(z·0.25, x·0.25, 0) · 4.
        self.shift.get_value(z * 0.25, x * 0.25, 0.0) * 4.0
    }

    /// `shiftedNoise2d(shiftX, shiftZ, 0.25, noise)` at (x, z) — the shared 2d sampling.
    #[inline]
    fn shifted_2d(&self, noise: &NormalNoise, x: f64, z: f64) -> f64 {
        let nx = x * 0.25 + self.shift_x(x, z);
        let nz = z * 0.25 + self.shift_z(x, z);
        noise.get_value(nx, 0.0, nz)
    }

    pub fn continents(&self, x: f64, z: f64) -> f64 {
        self.shifted_2d(&self.continentalness, x, z)
    }
    pub fn erosion(&self, x: f64, z: f64) -> f64 {
        self.shifted_2d(&self.erosion, x, z)
    }
    /// The raw ridge noise (Java's "weirdness" coordinate).
    pub fn ridge(&self, x: f64, z: f64) -> f64 {
        self.shifted_2d(&self.ridge, x, z)
    }
    /// `RIDGES_FOLDED` = the density-function `peaksAndValleys(ridge)` (computed in f64).
    pub fn ridges_folded(&self, x: f64, z: f64) -> f64 {
        let w = self.ridge(x, z);
        // mul(add(add(|w|, -2/3).abs(), -1/3), -3)
        ((w.abs() - 0.666_666_666_666_666_6).abs() - 0.333_333_333_333_333_3) * -3.0
    }

    fn spline_input(&self, x: f64, z: f64) -> SplineInput {
        SplineInput {
            continents: self.continents(x, z) as f32,
            erosion: self.erosion(x, z) as f32,
            ridges: self.ridge(x, z) as f32,
            ridges_folded: self.ridges_folded(x, z) as f32,
        }
    }

    // ---- terrain shaping ----

    /// `offset = GLOBAL_OFFSET + overworldOffset_spline` (blend is identity in a fresh world).
    pub fn offset(&self, x: f64, z: f64) -> f64 {
        GLOBAL_OFFSET + self.offset_spline.sample(&self.spline_input(x, z)) as f64
    }
    /// `factor = overworldFactor_spline` (blend target folds away).
    pub fn factor(&self, x: f64, z: f64) -> f64 {
        self.factor_spline.sample(&self.spline_input(x, z)) as f64
    }
    /// `depth = yClampedGradient(-64, 320, 1.5, -1.5) + offset`.
    pub fn depth(&self, x: f64, y: f64, z: f64) -> f64 {
        clamped_map(y, -64.0, 320.0, 1.5, -1.5) + self.offset(x, z)
    }

    /// `jaggedness = unscaledJaggedness · halfNegative(jaggedNoise)` where
    /// `jaggedNoise = noise(JAGGED, xzScale=1500, yScale=0)`.
    fn jaggedness(&self, x: f64, y: f64, z: f64, input: &SplineInput) -> f64 {
        let unscaled = self.jaggedness_spline.sample(input) as f64;
        let jagged_noise = self.jagged.get_value(x * 1500.0, y * 0.0, z * 1500.0);
        let half_neg = if jagged_noise > 0.0 { jagged_noise } else { jagged_noise * 0.5 };
        unscaled * half_neg
    }

    /// `slopedCheese = 4·quarterNegative((depth + jaggedness)·factor) + base_3d_noise`.
    pub fn sloped_cheese(&self, x: f64, y: f64, z: f64) -> f64 {
        let input = self.spline_input(x, z);
        let offset = GLOBAL_OFFSET + self.offset_spline.sample(&input) as f64;
        let factor = self.factor_spline.sample(&input) as f64;
        let depth = clamped_map(y, -64.0, 320.0, 1.5, -1.5) + offset;
        let depth_with_jaggedness = depth + self.jaggedness(x, y, z, &input);

        let gradient_unscaled = depth_with_jaggedness * factor;
        let quarter_neg = if gradient_unscaled > 0.0 { gradient_unscaled } else { gradient_unscaled * 0.25 };
        let initial_density = 4.0 * quarter_neg;

        initial_density + self.base_3d.compute(x, y, z)
    }

    /// The terrain density **without cave carving / aquifers**: the overworld y-slide
    /// applied to `slopedCheese`, then the `squeeze` post-process. `> 0 ⇒ solid`.
    pub fn density_no_caves(&self, x: f64, y: f64, z: f64) -> f64 {
        let caves = self.sloped_cheese(x, y, z);
        let slid = slide_overworld(y, caves);
        // postProcess: squeeze(0.64 · slide)  (blendDensity + interpolated are identity).
        squeeze(0.64 * slid)
    }

    /// Topmost `y` in `[min_y, max_y]` whose pre-cave density is solid, or `None` if the
    /// column is entirely air in range. This is the predicted surface altitude to compare
    /// against the oracle (subject to the deferred cave/aquifer/surface-rule caveats).
    pub fn surface_y(&self, x: f64, z: f64, min_y: i32, max_y: i32) -> Option<i32> {
        (min_y..=max_y).rev().find(|&y| self.density_no_caves(x, y as f64, z as f64) > 0.0)
    }
}

/// `slideOverworld(amplified=false, caves)` — the top/bottom y taper.
fn slide_overworld(y: f64, caves: f64) -> f64 {
    // slide(caves, minY=-64, height=384, 80, 64, -0.078125, 0, 24, 0.1171875)
    // topFactor over [minY+height-topStartY, minY+height-topEndY] = [240, 256] : 1 → 0.
    let top_factor = clamped_map(y, 240.0, 256.0, 1.0, 0.0);
    let noise_value = lerp(top_factor, -0.078125, caves);
    // bottomFactor over [minY+bottomStartY, minY+bottomEndY] = [-64, -40] : 0 → 1.
    let bottom_factor = clamped_map(y, -64.0, -40.0, 0.0, 1.0);
    lerp(bottom_factor, 0.1171875, noise_value)
}

/// `squeeze` node — clamp to ±1, then `c/2 - c³/24`.
fn squeeze(v: f64) -> f64 {
    let c = v.clamp(-1.0, 1.0);
    c / 2.0 - c * c * c / 24.0
}

#[inline]
fn lerp(t: f64, a: f64, b: f64) -> f64 {
    a + t * (b - a)
}

/// `Mth.clampedMap` (double).
#[inline]
fn clamped_map(value: f64, from_min: f64, from_max: f64, to_min: f64, to_max: f64) -> f64 {
    let factor = (value - from_min) / (from_max - from_min);
    if factor < 0.0 {
        to_min
    } else if factor > 1.0 {
        to_max
    } else {
        to_min + factor * (to_max - to_min)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SEED: i64 = 6954908675375307936;

    #[test]
    fn density_profile_is_solid_low_and_air_high() {
        let ow = Overworld::new(SEED);
        // Deep underground must be solid; high in the sky must be air — at any column.
        for (x, z) in [(0.0, 0.0), (37.0, 37.0), (-8.0, 24.0)] {
            assert!(ow.density_no_caves(x, -50.0, z) > 0.0, "expected solid at y=-50 ({x},{z})");
            assert!(ow.density_no_caves(x, 300.0, z) < 0.0, "expected air at y=300 ({x},{z})");
        }
    }

    #[test]
    fn spawn_surface_altitude_matches_oracle() {
        // Ground truth from the Voxel save (examples/oracle): grass at y=65 at block (0,0),
        // i.e. solid up to ~65. Without caves/aquifers/surface-rules the pre-cave density
        // crossing should land within a few blocks of that.
        let ow = Overworld::new(SEED);
        let surface = ow.surface_y(0.0, 0.0, -64, 320).expect("column has a surface");
        assert!(
            (60..=70).contains(&surface),
            "predicted spawn surface y={surface}, expected near oracle's 65"
        );
    }

    #[test]
    fn deterministic() {
        let ow = Overworld::new(SEED);
        assert_eq!(ow.sloped_cheese(10.0, 40.0, -20.0), ow.sloped_cheese(10.0, 40.0, -20.0));
    }
}
