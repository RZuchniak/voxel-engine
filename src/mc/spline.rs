//! Cubic splines — the `offset`/`factor`/`jaggedness` terrain shaping
//! (`net.minecraft.util.CubicSpline` + `net.minecraft.data.worldgen.TerrainProvider`).
//!
//! In Minecraft's density-function terrain, three quantities are derived from the climate
//! coordinates (continentalness, erosion, weirdness/ridges) via nested cubic (Hermite)
//! splines: the terrain `offset`, the `factor`, and the `jaggedness`. These feed the
//! `depth`/`initialDensity` that ultimately decides surface height.
//!
//! ## Precision note
//!
//! `CubicSpline` math is **`f32`** in Java (control points, derivatives, and the Hermite
//! evaluation are all `float`). Bit-exact parity requires the same, so this module computes
//! in `f32`. Coordinate density-function values arrive as `f64` and are cast to `f32` at the
//! boundary (matching Java's `(float) function.compute(context)` in `Spline.Coordinate`).
//!
//! ## Scope
//!
//! Only the `sample` path is ported — the `minValue`/`maxValue` bounds that `Multipoint`
//! also tracks exist purely for the density-DAG min/max short-circuit optimization, which a
//! scalar evaluator doesn't need. This module builds the **non-amplified** overworld splines
//! (every `valueTransformer` is identity there, so the transform is omitted).

/// Which climate coordinate a spline branches on. `Ridges` is the raw ridge noise
/// (Java's "weirdness" coordinate); `RidgesFolded` is `peaksAndValleys(ridge)` (Java's
/// "ridges" coordinate). This mirrors how `registerTerrainNoises` wires the coordinates.
#[derive(Clone, Copy)]
pub enum Coord {
    Continents,
    Erosion,
    /// Raw ridge noise — Java calls this coordinate "weirdness".
    Ridges,
    /// `peaksAndValleys(ridge)` — Java calls this coordinate "ridges".
    RidgesFolded,
}

/// The four coordinate values at one position (already cast to `f32`).
#[derive(Clone, Copy)]
pub struct SplineInput {
    pub continents: f32,
    pub erosion: f32,
    pub ridges: f32,
    pub ridges_folded: f32,
}

impl Coord {
    #[inline]
    fn value(self, input: &SplineInput) -> f32 {
        match self {
            Coord::Continents => input.continents,
            Coord::Erosion => input.erosion,
            Coord::Ridges => input.ridges,
            Coord::RidgesFolded => input.ridges_folded,
        }
    }
}

/// A cubic spline: either a constant, or a multipoint Hermite spline over one coordinate.
#[derive(Clone)]
pub enum Spline {
    Constant(f32),
    Multipoint { coord: Coord, locations: Vec<f32>, values: Vec<Spline>, derivatives: Vec<f32> },
}

/// `Mth.lerp(float, float, float)`.
#[inline]
fn lerp(t: f32, a: f32, b: f32) -> f32 {
    a + t * (b - a)
}

impl Spline {
    /// `CubicSpline.Multipoint.sample` — evaluate the spline at a position.
    pub fn sample(&self, input: &SplineInput) -> f32 {
        match self {
            Spline::Constant(v) => *v,
            Spline::Multipoint { coord, locations, values, derivatives } => {
                let x = coord.value(input);
                let last = locations.len() as i32 - 1;
                // findIntervalStart = binarySearch(first i where x < locations[i]) - 1.
                let first_gt = locations.iter().position(|&l| x < l).unwrap_or(locations.len());
                let start = first_gt as i32 - 1;

                if start < 0 {
                    linear_extend(x, locations, values[0].sample(input), derivatives, 0)
                } else if start == last {
                    let li = last as usize;
                    linear_extend(x, locations, values[li].sample(input), derivatives, li)
                } else {
                    let s = start as usize;
                    let x1 = locations[s];
                    let x2 = locations[s + 1];
                    let t = (x - x1) / (x2 - x1);
                    let y1 = values[s].sample(input);
                    let y2 = values[s + 1].sample(input);
                    let d1 = derivatives[s];
                    let d2 = derivatives[s + 1];
                    let a = d1 * (x2 - x1) - (y2 - y1);
                    let b = -d2 * (x2 - x1) + (y2 - y1);
                    lerp(t, y1, y2) + t * (1.0 - t) * lerp(t, a, b)
                }
            }
        }
    }
}

/// `CubicSpline.Multipoint.linearExtend` — flat past the ends unless the edge derivative is nonzero.
#[inline]
fn linear_extend(x: f32, locations: &[f32], value: f32, derivatives: &[f32], index: usize) -> f32 {
    let d = derivatives[index];
    if d == 0.0 { value } else { value + d * (x - locations[index]) }
}

// ---------------------------------------------------------------------------------------
// Builder + TerrainProvider port (non-amplified overworld). Transcribed line-for-line from
// net.minecraft.data.worldgen.TerrainProvider; every value transformer there is identity.
// ---------------------------------------------------------------------------------------

/// Mirrors `CubicSpline.Builder`, accumulating (location, value, derivative) triples.
struct Builder {
    coord: Coord,
    locations: Vec<f32>,
    values: Vec<Spline>,
    derivatives: Vec<f32>,
}

impl Builder {
    fn new(coord: Coord) -> Self {
        Self { coord, locations: Vec::new(), values: Vec::new(), derivatives: Vec::new() }
    }
    /// `addPoint(location, float value)` — constant value, derivative 0.
    fn point(mut self, location: f32, value: f32) -> Self {
        self.locations.push(location);
        self.values.push(Spline::Constant(value));
        self.derivatives.push(0.0);
        self
    }
    /// `addPoint(location, float value, float derivative)`.
    fn point_d(mut self, location: f32, value: f32, derivative: f32) -> Self {
        self.locations.push(location);
        self.values.push(Spline::Constant(value));
        self.derivatives.push(derivative);
        self
    }
    /// `addPoint(location, CubicSpline sampler)` — sub-spline value, derivative 0.
    fn spline(mut self, location: f32, sampler: Spline) -> Self {
        self.locations.push(location);
        self.values.push(sampler);
        self.derivatives.push(0.0);
        self
    }
    fn build(self) -> Spline {
        Spline::Multipoint {
            coord: self.coord,
            locations: self.locations,
            values: self.values,
            derivatives: self.derivatives,
        }
    }
}

/// `TerrainProvider.peaksAndValleys(float)`.
pub fn peaks_and_valleys(weirdness: f32) -> f32 {
    -((weirdness.abs() - 0.6666667).abs() - 0.33333334) * 3.0
}

fn mountain_continentalness(ridge: f32, modulation: f32, allow_rivers_below: f32) -> f32 {
    let ridge_slope = 1.0 - (1.0 - modulation) * 0.5;
    let ridge_intersect = 0.5 * (1.0 - modulation);
    let adjusted_ridge_height = (ridge + 1.17) * 0.46082947;
    let continentalness = adjusted_ridge_height * ridge_slope - ridge_intersect;
    if ridge < allow_rivers_below {
        continentalness.max(-0.2222)
    } else {
        continentalness.max(0.0)
    }
}

fn calculate_mountain_ridge_zero_continentalness_point(modulation: f32) -> f32 {
    let ridge_slope = 1.0 - (1.0 - modulation) * 0.5;
    let ridge_intersect = 0.5 * (1.0 - modulation);
    ridge_intersect / (0.46082947 * ridge_slope) - 1.17
}

fn calculate_slope(y1: f32, y2: f32, x1: f32, x2: f32) -> f32 {
    (y2 - y1) / (x2 - x1)
}

fn build_mountain_ridge_spline_with_points(modulation: f32, saddle: bool) -> Spline {
    let mut build = Builder::new(Coord::RidgesFolded);
    let min_point_cont = mountain_continentalness(-1.0, modulation, -0.7);
    let max_point_cont = mountain_continentalness(1.0, modulation, -0.7);
    let ridge_zero_point = calculate_mountain_ridge_zero_continentalness_point(modulation);
    if -0.65 < ridge_zero_point && ridge_zero_point < 1.0 {
        let after_river_threshold = mountain_continentalness(-0.65, modulation, -0.7);
        let before_river_threshold = mountain_continentalness(-0.75, modulation, -0.7);
        let min_point_derivative = calculate_slope(min_point_cont, before_river_threshold, -1.0, -0.75);
        build = build.point_d(-1.0, min_point_cont, min_point_derivative);
        build = build.point(-0.75, before_river_threshold);
        build = build.point(-0.65, after_river_threshold);
        let ridge_zero_point_cont = mountain_continentalness(ridge_zero_point, modulation, -0.7);
        let max_point_derivative = calculate_slope(ridge_zero_point_cont, max_point_cont, ridge_zero_point, 1.0);
        build = build.point(ridge_zero_point - 0.01, ridge_zero_point_cont);
        build = build.point_d(ridge_zero_point, ridge_zero_point_cont, max_point_derivative);
        build = build.point_d(1.0, max_point_cont, max_point_derivative);
    } else {
        let simple_derivative = calculate_slope(min_point_cont, max_point_cont, -1.0, 1.0);
        if saddle {
            build = build.point(-1.0, min_point_cont.max(0.2));
            build = build.point_d(0.0, lerp(0.5, min_point_cont, max_point_cont), simple_derivative);
        } else {
            build = build.point_d(-1.0, min_point_cont, simple_derivative);
        }
        build = build.point_d(1.0, max_point_cont, simple_derivative);
    }
    build.build()
}

/// `TerrainProvider.ridgeSpline` — the valley→low→mid→high→peaks ridge shape.
#[allow(clippy::too_many_arguments)]
fn ridge_spline(valley: f32, low: f32, mid: f32, high: f32, peaks: f32, min_valley_steepness: f32) -> Spline {
    let d1 = (0.5 * (low - valley)).max(min_valley_steepness);
    let d2 = 5.0 * (mid - low);
    Builder::new(Coord::RidgesFolded)
        .point_d(-1.0, valley, d1)
        .point_d(-0.4, low, d1.min(d2))
        .point_d(0.0, mid, d2)
        .point_d(0.4, high, 2.0 * (high - mid))
        .point_d(1.0, peaks, 0.7 * (peaks - high))
        .build()
}

#[allow(clippy::too_many_arguments)]
fn build_erosion_offset_spline(
    low_valley: f32,
    hill: f32,
    tall_hill: f32,
    mountain_factor: f32,
    plain: f32,
    swamp: f32,
    include_extreme_hills: bool,
    saddle: bool,
) -> Spline {
    let very_low_erosion_mountains = build_mountain_ridge_spline_with_points(lerp(mountain_factor, 0.6, 1.5), saddle);
    let low_erosion_mountains = build_mountain_ridge_spline_with_points(lerp(mountain_factor, 0.6, 1.0), saddle);
    let mountains = build_mountain_ridge_spline_with_points(mountain_factor, saddle);
    let wide_plateau = ridge_spline(
        low_valley - 0.15,
        0.5 * mountain_factor,
        lerp(0.5, 0.5, 0.5) * mountain_factor,
        0.5 * mountain_factor,
        0.6 * mountain_factor,
        0.5,
    );
    let narrow_plateau = ridge_spline(
        low_valley,
        plain * mountain_factor,
        hill * mountain_factor,
        0.5 * mountain_factor,
        0.6 * mountain_factor,
        0.5,
    );
    let plains = ridge_spline(low_valley, plain, plain, hill, tall_hill, 0.5);
    let plains_far_inland = ridge_spline(low_valley, plain, plain, hill, tall_hill, 0.5);
    let extreme_hills = Builder::new(Coord::RidgesFolded)
        .point(-1.0, low_valley)
        .spline(-0.4, plains.clone())
        .point(0.0, tall_hill + 0.07)
        .build();
    let swamps = ridge_spline(-0.02, swamp, swamp, hill, tall_hill, 0.0);

    let mut builder = Builder::new(Coord::Erosion)
        .spline(-0.85, very_low_erosion_mountains)
        .spline(-0.7, low_erosion_mountains)
        .spline(-0.4, mountains)
        .spline(-0.35, wide_plateau)
        .spline(-0.1, narrow_plateau)
        .spline(0.2, plains);
    if include_extreme_hills {
        // plainsFarInland is reused at 0.4 and 0.58; extremeHills at 0.45 and 0.55.
        builder = builder
            .spline(0.4, plains_far_inland.clone())
            .spline(0.45, extreme_hills.clone())
            .spline(0.55, extreme_hills)
            .spline(0.58, plains_far_inland);
    }
    builder = builder.spline(0.7, swamps);
    builder.build()
}

/// `overworldOffset(continents, erosion, ridges, amplified=false)`.
pub fn overworld_offset() -> Spline {
    let beach = build_erosion_offset_spline(-0.15, 0.0, 0.0, 0.1, 0.0, -0.03, false, false);
    let low = build_erosion_offset_spline(-0.1, 0.03, 0.1, 0.1, 0.01, -0.03, false, false);
    let mid = build_erosion_offset_spline(-0.1, 0.03, 0.1, 0.7, 0.01, -0.03, true, true);
    let high = build_erosion_offset_spline(-0.05, 0.03, 0.1, 1.0, 0.01, 0.01, true, true);
    Builder::new(Coord::Continents)
        .point(-1.1, 0.044)
        .point(-1.02, -0.2222)
        .point(-0.51, -0.2222)
        .point(-0.44, -0.12)
        .point(-0.18, -0.12)
        .spline(-0.16, beach.clone())
        .spline(-0.15, beach)
        .spline(-0.1, low)
        .spline(0.25, mid)
        .spline(1.0, high)
        .build()
}

fn build_weirdness_jaggedness_spline(jaggedness_factor: f32) -> Spline {
    let max_neg = 0.63 * jaggedness_factor;
    let max_pos = 0.3 * jaggedness_factor;
    Builder::new(Coord::Ridges).point(-0.01, max_neg).point(0.01, max_pos).build()
}

fn build_ridge_jaggedness_spline(jaggedness_at_peak_ridge: f32, jaggedness_at_high_ridge: f32) -> Spline {
    let high_slice_start = peaks_and_valleys(0.4);
    let high_slice_end = peaks_and_valleys(0.56666666);
    let high_slice_middle = (high_slice_start + high_slice_end) / 2.0;
    let mut b = Builder::new(Coord::RidgesFolded).point(high_slice_start, 0.0);
    if jaggedness_at_high_ridge > 0.0 {
        b = b.spline(high_slice_middle, build_weirdness_jaggedness_spline(jaggedness_at_high_ridge));
    } else {
        b = b.point(high_slice_middle, 0.0);
    }
    if jaggedness_at_peak_ridge > 0.0 {
        b = b.spline(1.0, build_weirdness_jaggedness_spline(jaggedness_at_peak_ridge));
    } else {
        b = b.point(1.0, 0.0);
    }
    b.build()
}

fn build_erosion_jaggedness_spline(
    peak_e0: f32,
    peak_e1: f32,
    high_e0: f32,
    high_e1: f32,
) -> Spline {
    let ridge_at_e0 = build_ridge_jaggedness_spline(peak_e0, high_e0);
    let ridge_at_e1 = build_ridge_jaggedness_spline(peak_e1, high_e1);
    Builder::new(Coord::Erosion)
        .spline(-1.0, ridge_at_e0)
        .spline(-0.78, ridge_at_e1.clone())
        .spline(-0.5775, ridge_at_e1)
        .point(-0.375, 0.0)
        .build()
}

/// `overworldJaggedness(continents, erosion, weirdness, ridges, amplified=false)`.
pub fn overworld_jaggedness() -> Spline {
    Builder::new(Coord::Continents)
        .point(-0.11, 0.0)
        .spline(0.03, build_erosion_jaggedness_spline(1.0, 0.5, 0.0, 0.0))
        .spline(0.65, build_erosion_jaggedness_spline(1.0, 1.0, 1.0, 0.0))
        .build()
}

fn get_erosion_factor(base_value: f32, shattered_terrain: bool) -> Spline {
    let base_spline = Builder::new(Coord::Ridges).point(-0.2, 6.3).point(0.2, base_value).build();
    let mut erosion_points = Builder::new(Coord::Erosion)
        .spline(-0.6, base_spline.clone())
        .spline(-0.5, Builder::new(Coord::Ridges).point(-0.05, 6.3).point(0.05, 2.67).build())
        .spline(-0.35, base_spline.clone())
        .spline(-0.25, base_spline.clone())
        .spline(-0.1, Builder::new(Coord::Ridges).point(-0.05, 2.67).point(0.05, 6.3).build())
        .spline(0.03, base_spline.clone());
    if shattered_terrain {
        let weirdness_shattered = Builder::new(Coord::Ridges).point(0.0, base_value).point(0.1, 0.625).build();
        let ridges_shattered = Builder::new(Coord::RidgesFolded)
            .point(-0.9, base_value)
            .spline(-0.69, weirdness_shattered)
            .build();
        erosion_points = erosion_points
            .point(0.35, base_value)
            .spline(0.45, ridges_shattered.clone())
            .spline(0.55, ridges_shattered)
            .point(0.62, base_value);
    } else {
        let extreme_hills = Builder::new(Coord::RidgesFolded)
            .spline(-0.7, base_spline.clone())
            .point(-0.15, 1.37)
            .build();
        let extra_3d = Builder::new(Coord::RidgesFolded).spline(0.45, base_spline).point(0.7, 1.56).build();
        erosion_points = erosion_points
            .spline(0.05, extra_3d.clone())
            .spline(0.4, extra_3d)
            .spline(0.45, extreme_hills.clone())
            .spline(0.55, extreme_hills)
            .point(0.58, base_value);
    }
    erosion_points.build()
}

/// `overworldFactor(continents, erosion, weirdness, ridges, amplified=false)`.
pub fn overworld_factor() -> Spline {
    Builder::new(Coord::Continents)
        .point(-0.19, 3.95)
        .spline(-0.15, get_erosion_factor(6.25, true))
        .spline(-0.1, get_erosion_factor(5.47, true))
        .spline(0.03, get_erosion_factor(5.08, true))
        .spline(0.06, get_erosion_factor(4.69, false))
        .build()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(continents: f32, erosion: f32, ridge: f32) -> SplineInput {
        SplineInput { continents, erosion, ridges: ridge, ridges_folded: peaks_and_valleys(ridge) }
    }

    #[test]
    fn peaks_and_valleys_matches_formula() {
        // -(|(|w|-2/3)|-1/3)*3
        assert!((peaks_and_valleys(0.0) - (-((0.6666667f32).abs() - 0.33333334) * 3.0)).abs() < 1e-6);
        // At w where |w|=2/3, inner = -1/3, result = 1.0
        assert!((peaks_and_valleys(0.6666667) - 1.0).abs() < 1e-4);
    }

    #[test]
    fn constant_spline_returns_value() {
        let s = Spline::Constant(1.25);
        assert_eq!(s.sample(&input(0.0, 0.0, 0.0)), 1.25);
    }

    #[test]
    fn simple_hermite_endpoints_and_midpoint() {
        // Two-point spline on continents: (-1 → 0), (1 → 10), zero derivatives.
        let s = Builder::new(Coord::Continents).point(-1.0, 0.0).point(1.0, 10.0).build();
        assert_eq!(s.sample(&input(-1.0, 0.0, 0.0)), 0.0);
        assert_eq!(s.sample(&input(1.0, 0.0, 0.0)), 10.0);
        // Midpoint with zero derivatives: a=-(y2-y1)=-10, b=+(y2-y1)=10, so lerp(.5,a,b)=0;
        // result = lerp(.5,0,10) + .25*0 = 5.0.
        assert!((s.sample(&input(0.0, 0.0, 0.0)) - 5.0).abs() < 1e-5);
        // Below/above range → clamped flat (zero edge derivative).
        assert_eq!(s.sample(&input(-5.0, 0.0, 0.0)), 0.0);
        assert_eq!(s.sample(&input(5.0, 0.0, 0.0)), 10.0);
    }

    #[test]
    fn overworld_splines_build_and_sample_finite() {
        let off = overworld_offset();
        let fac = overworld_factor();
        let jag = overworld_jaggedness();
        // Sample across a spread of climate inputs; every result must be finite.
        for &c in &[-1.0f32, -0.3, 0.0, 0.5, 1.0] {
            for &e in &[-0.9f32, -0.2, 0.2, 0.7] {
                for &r in &[-0.9f32, 0.0, 0.9] {
                    let inp = input(c, e, r);
                    assert!(off.sample(&inp).is_finite(), "offset NaN at c={c} e={e} r={r}");
                    assert!(fac.sample(&inp).is_finite(), "factor NaN at c={c} e={e} r={r}");
                    assert!(jag.sample(&inp).is_finite(), "jagged NaN at c={c} e={e} r={r}");
                }
            }
        }
        // Factor stays in a sane band (Java factor ~ [0.6, 6.3]).
        let f = fac.sample(&input(0.5, 0.2, 0.0));
        assert!(f > 0.0 && f < 10.0, "factor {f} out of band");
    }
}
