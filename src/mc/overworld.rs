//! Assembling the overworld terrain density (`NoiseRouterData.overworld` +
//! `registerTerrainNoises`), seeded from a real world seed.
//!
//! This ties the pieces together — shifted climate noises, the offset/factor/jaggedness
//! splines, `BlendedNoise` — into the `slopedCheese` density and the slided/squeezed
//! terrain density that decides solid-vs-air.
//!
//! ## What's included vs deferred
//!
//! Included: the full terrain column — climate coordinates, `offset`/`factor`/`depth`/
//! `jaggedness`, `slopedCheese = initialDensity + base_3d_noise`, the overworld y-slide,
//! the `squeeze` post-process, cave carving (`super::caves`), and the cell interpolation
//! that vanilla applies on top ([`CellSampler`]). `finalDensity > 0 ⇒ solid`.
//!
//! What a `finalDensity <= 0` position actually *becomes* — air, water or lava — is
//! `super::aquifer`'s job. Still deferred: surface rules (grass/dirt/sand vs stone), so
//! this places the right *shape* but not yet the right surface block.

use std::collections::HashMap;
use std::sync::Arc;

use super::aquifer::{AquiferNoises, ChunkAquifer};
use super::blended_noise::BlendedNoise;
use super::caves::{self, Caves, NoodleNodes};
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
    caves: Caves,
    aquifer: AquiferNoises,
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
            caves: Caves::new(&factory),
            aquifer: AquiferNoises::new(&factory),
        }
    }

    /// The shared aquifer noises + positional factory (`RandomState`-level state).
    pub(super) fn aquifer(&self) -> &AquiferNoises {
        &self.aquifer
    }

    /// Build the [`ChunkAquifer`] for chunk `(chunk_x, chunk_z)`. The aquifer is per-chunk
    /// in vanilla (its sampling cutoff depends on the chunk's highest surface), so a
    /// column's substance must be queried through the aquifer of its own chunk.
    pub fn aquifer_for_chunk(&self, chunk_x: i32, chunk_z: i32) -> ChunkAquifer<'_> {
        ChunkAquifer::new(self, chunk_x, chunk_z)
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
    /// The router wraps every 2d climate noise in `flatCache`, so it is sampled **once per
    /// quart column** and reused across that quart's 4×4 blocks; [`quart`] reproduces that.
    #[inline]
    fn shifted_2d(&self, noise: &NormalNoise, x: f64, z: f64) -> f64 {
        let (x, z) = (quart(x), quart(z));
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
        // Also `flatCache`d in the router — quart-resolution, and always at y = 0 (which is
        // moot here since the jagged noise's y scale is 0 anyway).
        let jagged_noise = self.jagged.get_value(quart(x) * 1500.0, y * 0.0, quart(z) * 1500.0);
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
    /// Faster than [`Self::final_density`] and good enough for a surface heightmap.
    pub fn density_no_caves(&self, x: f64, y: f64, z: f64) -> f64 {
        let caves = self.sloped_cheese(x, y, z);
        let slid = slide_overworld(y, caves);
        // postProcess: squeeze(0.64 · slide)  (blendDensity + interpolated are identity).
        squeeze(0.64 * slid)
    }

    /// The full overworld `finalDensity` **with cave carving**:
    /// `min( squeeze(0.64 · slide(caves)), NOODLE )`. `> 0 ⇒ solid`.
    ///
    /// Both halves are `interpolated` in the router, so this is a trilinear blend of the
    /// 4×8×4 cell lattice, not a per-block evaluation — see [`CellSampler`]. Each call
    /// evaluates all 8 corners; use a [`CellSampler`] directly to reuse them across a chunk.
    pub fn final_density(&self, x: i32, y: i32, z: i32) -> f64 {
        CellSampler::new(self).final_density(x, y, z)
    }

    /// The `interpolated`-wrapped nodes at one cell-lattice corner.
    fn cell_nodes(&self, x: i32, y: i32, z: i32) -> CellNodes {
        let (xf, yf, zf) = (x as f64, y as f64, z as f64);
        let sloped_cheese = self.sloped_cheese(xf, yf, zf);
        let caves = self.caves.caves(xf, yf, zf, sloped_cheese);
        CellNodes {
            // `postProcess` interpolates `0.64 · slide(caves)`, then squeezes per block.
            density: 0.64 * slide_overworld(yf, caves),
            noodle: self.caves.noodle_nodes(xf, yf, zf),
        }
    }

    /// `preliminarySurfaceLevel(x, z)` — a cheap surface estimate the **aquifer** samples
    /// (`NoiseRouterData.preliminarySurfaceLevel` → `findTopSurface`). It scans a simplified
    /// density (offset/factor only — no 3D noise, no jaggedness) by `cellHeight = 8` from a
    /// factor-derived ceiling and returns the first solid Y (or −64). Deliberately coarser
    /// than [`Self::surface_y`]; it exists to seed aquifer cells, not to place blocks.
    pub fn preliminary_surface_level(&self, x: f64, z: f64) -> i32 {
        let factor = self.factor(x, z);
        let offset = self.offset(x, z);
        // upperBound = remap(0.2734375/factor − offset, 1.5,−1.5 → −64,320) clamped to [−40,320].
        // remap(input, 1.5,−1.5,−64,320) = input·(−128) + 128.
        let upper_raw = (0.2734375 * (1.0 / factor) - offset) * -128.0 + 128.0;
        let upper = upper_raw.clamp(-40.0, 320.0);

        const CELL_HEIGHT: i32 = 8;
        const LOWER: i32 = -64;
        let top_y = (upper / CELL_HEIGHT as f64).floor() as i32 * CELL_HEIGHT;
        if top_y <= LOWER {
            return LOWER;
        }
        let mut y = top_y;
        while y >= LOWER {
            if self.preliminary_density(y as f64, offset, factor) > 0.0 {
                return y;
            }
            y -= CELL_HEIGHT;
        }
        LOWER
    }

    /// The simplified density `findTopSurface` scans: `slide(clamp(4·quarterNeg(depth·factor)
    /// − 0.703125, ±64)) − 0.390625` (no 3D noise / jaggedness). `offset`/`factor` are the
    /// column values, passed in to avoid recomputing the splines per Y step.
    fn preliminary_density(&self, y: f64, offset: f64, factor: f64) -> f64 {
        let depth = clamped_map(y, -64.0, 320.0, 1.5, -1.5) + offset;
        let gradient_unscaled = depth * factor;
        let quarter_neg = if gradient_unscaled > 0.0 { gradient_unscaled } else { gradient_unscaled * 0.25 };
        let inner = (4.0 * quarter_neg - 0.703125).clamp(-64.0, 64.0);
        slide_overworld(y, inner) - 0.390625
    }

    /// Topmost `y` in `[min_y, max_y]` whose pre-cave density is solid, or `None` if the
    /// column is entirely air in range. This is the predicted surface altitude to compare
    /// against the oracle (subject to the deferred cave/aquifer/surface-rule caveats).
    pub fn surface_y(&self, x: f64, z: f64, min_y: i32, max_y: i32) -> Option<i32> {
        (min_y..=max_y).rev().find(|&y| self.density_no_caves(x, y as f64, z as f64) > 0.0)
    }
}

/// The cell size the router's `interpolated` nodes are sampled on
/// (`NoiseSettings.OVERWORLD_NOISE_SETTINGS.getCell{Width,Height}()`).
const CELL_WIDTH: i32 = 4;
const CELL_HEIGHT: i32 = 8;

/// The density-function nodes vanilla wraps in `interpolated`, at one lattice corner.
#[derive(Clone, Copy, Default)]
struct CellNodes {
    /// `0.64 · slide(caves)` — the argument of `postProcess`'s `interpolated`.
    density: f64,
    noodle: NoodleNodes,
}

/// Samples the terrain the way `NoiseChunk` does: the expensive density nodes are
/// evaluated **only on the 4×8×4 cell lattice** and trilinearly blended between corners.
/// That smoothing is part of the world's shape, not an optimisation — caves and overhangs
/// come out visibly different without it.
///
/// The corner cache makes bulk sampling cheap: a cell's 8 corners serve all 128 blocks in
/// it, and neighbouring cells share them. Keep one sampler per chunk.
pub struct CellSampler<'a> {
    ow: &'a Overworld,
    corners: HashMap<(i32, i32, i32), CellNodes>,
}

impl<'a> CellSampler<'a> {
    pub fn new(ow: &'a Overworld) -> Self {
        Self { ow, corners: HashMap::new() }
    }

    /// See [`Overworld::final_density`]. `> 0 ⇒ solid`.
    pub fn final_density(&mut self, x: i32, y: i32, z: i32) -> f64 {
        let x0 = x.div_euclid(CELL_WIDTH) * CELL_WIDTH;
        let y0 = y.div_euclid(CELL_HEIGHT) * CELL_HEIGHT;
        let z0 = z.div_euclid(CELL_WIDTH) * CELL_WIDTH;
        let ax = (x - x0) as f64 / CELL_WIDTH as f64;
        let ay = (y - y0) as f64 / CELL_HEIGHT as f64;
        let az = (z - z0) as f64 / CELL_WIDTH as f64;

        // Corner order matches `Mth.lerp3`'s argument order: x fastest, then y, then z.
        let mut n = [CellNodes::default(); 8];
        let mut i = 0;
        for dz in [0, CELL_WIDTH] {
            for dy in [0, CELL_HEIGHT] {
                for dx in [0, CELL_WIDTH] {
                    n[i] = self.corner(x0 + dx, y0 + dy, z0 + dz);
                    i += 1;
                }
            }
        }

        let density = squeeze(lerp3(ax, ay, az, n.map(|c| c.density)));
        let noodle = caves::noodle(&NoodleNodes {
            toggle: lerp3(ax, ay, az, n.map(|c| c.noodle.toggle)),
            thickness: lerp3(ax, ay, az, n.map(|c| c.noodle.thickness)),
            ridge_a: lerp3(ax, ay, az, n.map(|c| c.noodle.ridge_a)),
            ridge_b: lerp3(ax, ay, az, n.map(|c| c.noodle.ridge_b)),
        });
        density.min(noodle)
    }

    fn corner(&mut self, x: i32, y: i32, z: i32) -> CellNodes {
        if let Some(&c) = self.corners.get(&(x, y, z)) {
            return c;
        }
        let c = self.ow.cell_nodes(x, y, z);
        self.corners.insert((x, y, z), c);
        c
    }
}

/// `Mth.lerp3` — trilinear blend over corners ordered `x000, x100, x010, x110, x001, …`.
fn lerp3(ax: f64, ay: f64, az: f64, v: [f64; 8]) -> f64 {
    let x00 = lerp(ax, v[0], v[1]);
    let x10 = lerp(ax, v[2], v[3]);
    let x01 = lerp(ax, v[4], v[5]);
    let x11 = lerp(ax, v[6], v[7]);
    lerp(az, lerp(ay, x00, x10), lerp(ay, x01, x11))
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

/// Snap a block coordinate down to its quart (4-block) origin — `QuartPos.toBlock(
/// QuartPos.fromBlock(v))`. This is the resolution `flatCache` pins the 2d climate and
/// spline layer to, so terrain shaping changes only every 4 blocks in x/z.
#[inline]
fn quart(v: f64) -> f64 {
    ((v.floor() as i32) & !3) as f64
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
    fn preliminary_surface_level_is_plausible() {
        // The aquifer's cheap surface estimate: a multiple of the cell height (8) or the
        // floor (−64), and near the real surface at spawn (oracle grass y65).
        let ow = Overworld::new(SEED);
        let psl = ow.preliminary_surface_level(0.0, 0.0);
        assert!(psl == -64 || psl % 8 == 0, "psl {psl} not on the cell grid");
        assert!((40..=96).contains(&psl), "psl {psl} implausibly far from spawn surface");
    }

    #[test]
    fn deterministic() {
        let ow = Overworld::new(SEED);
        assert_eq!(ow.sloped_cheese(10.0, 40.0, -20.0), ow.sloped_cheese(10.0, 40.0, -20.0));
        assert_eq!(ow.final_density(10, 40, -20), ow.final_density(10, 40, -20));
    }

    #[test]
    fn cell_sampler_matches_uncached_final_density() {
        // The corner cache must be pure memoisation.
        let ow = Overworld::new(SEED);
        let mut sampler = CellSampler::new(&ow);
        for (x, y, z) in [(0, 30, 0), (7, -12, 5), (-3, 64, 11), (13, 100, -9)] {
            assert_eq!(sampler.final_density(x, y, z), ow.final_density(x, y, z), "at {x},{y},{z}");
        }
    }

    #[test]
    fn density_is_continuous_across_a_cell() {
        // Interpolation means the density varies smoothly inside a 4×8×4 cell rather than
        // jumping: consecutive blocks along x must not differ wildly.
        let ow = Overworld::new(SEED);
        let mut sampler = CellSampler::new(&ow);
        let mut prev = sampler.final_density(0, 20, 0);
        for x in 1..8 {
            let v = sampler.final_density(x, 20, 0);
            assert!((v - prev).abs() < 0.5, "density jumped from {prev} to {v} at x={x}");
            prev = v;
        }
    }

    #[test]
    fn caves_carve_voids_underground() {
        // With cave carving on, some below-surface blocks that the no-cave density calls
        // solid must become air (finalDensity <= 0). Scan a small underground volume.
        let ow = Overworld::new(SEED);
        // A region the oracle comparison flagged as cavey (near spawn's low ground).
        // Stop at the first carved block to keep the test cheap.
        let mut sampler = CellSampler::new(&ow);
        let carved = (16..48).flat_map(|x| (-80..-48).map(move |z| (x, z))).any(|(x, z)| {
            (-30..40).any(|y| {
                ow.density_no_caves(x as f64, y as f64, z as f64) > 0.0
                    && sampler.final_density(x, y, z) <= 0.0
            })
        });
        assert!(carved, "expected cave carving to open some underground voids");
    }
}
