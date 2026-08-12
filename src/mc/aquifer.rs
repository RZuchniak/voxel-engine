//! Aquifers — `Aquifer.NoiseBasedAquifer`, the last piece that decides what a
//! `finalDensity <= 0` position actually *is*: air, water, or lava, and which can turn a
//! would-be-air position back into stone via a **barrier**.
//!
//! Minecraft scatters aquifer "cells" on a jittered 16×12×16 grid. Each cell owns a
//! [`FluidStatus`] (a fluid level + a fluid type). For a block, the four nearest cell
//! centres are found; if the two closest are far enough apart in distance, a pressure
//! barrier is computed between their fluid levels and — where the barrier wins — the block
//! becomes stone instead of fluid. That is what seals underground lakes off from each other.
//!
//! ## Chunk scope
//!
//! The real aquifer is constructed per chunk, and it is not purely positional: the
//! `skipSamplingAboveY` cutoff comes from the *maximum* preliminary surface over the whole
//! chunk. So this mirrors Java and is built per chunk via
//! [`Overworld::aquifer_for_chunk`](super::overworld::Overworld::aquifer_for_chunk).
//!
//! Java's `shouldScheduleFluidUpdate` bookkeeping (which cells "may flow") is omitted: it
//! only drives fluid ticking, never the block that gets placed.

use std::collections::HashMap;
use std::sync::Arc;

use super::noise_params::Noise;
use super::normal_noise::NormalNoise;
use super::overworld::Overworld;
use super::rng::PositionalFactory;

/// `NoiseGeneratorSettings.overworld(...)` sea level.
pub const SEA_LEVEL: i32 = 63;
/// The global lava table below `y = -54` (`createFluidPicker`).
const LAVA_LEVEL: i32 = -54;
/// `DimensionType.WAY_BELOW_MIN_Y` = `MIN_Y << 4` = `-2032 << 4`. Used as the "this cell
/// holds no fluid at all" sentinel fluid level.
const WAY_BELOW_MIN_Y: i32 = -32512;

/// The overworld noise settings' vertical extent (`NoiseSettings.OVERWORLD_NOISE_SETTINGS`).
const MIN_BLOCK_Y: i32 = -64;
const Y_BLOCK_SIZE: i32 = 384;

/// Cell grid spacing (`X_SPACING` / `Y_SPACING` / `Z_SPACING`) and jitter ranges.
const Y_SPACING: i32 = 12;
const X_RANGE: i32 = 10;
const Y_RANGE: i32 = 9;
const Z_RANGE: i32 = 10;

/// What the aquifer decides a position is. `Solid` means "leave the terrain block".
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Substance {
    Solid,
    Air,
    Water,
    Lava,
}

/// The two fluids worldgen can place (`FluidStatus.fluidType`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Fluid {
    Water,
    Lava,
}

/// `Aquifer.FluidStatus` — a fluid surface level plus the fluid that fills below it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct FluidStatus {
    pub level: i32,
    pub fluid: Fluid,
}

impl FluidStatus {
    /// `at(blockY)` — the fluid below `level`, air at or above it.
    #[inline]
    fn at(self, y: i32) -> Option<Fluid> {
        if y < self.level {
            Some(self.fluid)
        } else {
            None
        }
    }
}

/// The global fluid picker built by `NoiseBasedChunkGenerator.createFluidPicker` for the
/// overworld: a lava table under `y = -54`, the sea above it. Independent of x/z.
#[inline]
fn global_fluid(y: i32) -> FluidStatus {
    if y < LAVA_LEVEL.min(SEA_LEVEL) {
        FluidStatus { level: LAVA_LEVEL, fluid: Fluid::Lava }
    } else {
        FluidStatus { level: SEA_LEVEL, fluid: Fluid::Water }
    }
}

/// The four named noises the aquifer samples, plus its positional random factory.
/// Built once per world (`RandomState`) and shared by every chunk's aquifer.
pub struct AquiferNoises {
    barrier: Arc<NormalNoise>,
    floodedness: Arc<NormalNoise>,
    spread: Arc<NormalNoise>,
    lava: Arc<NormalNoise>,
    /// `randomState.aquiferRandom()` = `random.fromHashOf("minecraft:aquifer").forkPositional()`.
    random: PositionalFactory,
}

impl AquiferNoises {
    pub fn new(factory: &PositionalFactory) -> Self {
        let mut aquifer_source = factory.from_hash_of("minecraft:aquifer");
        Self {
            barrier: Arc::new(Noise::AquiferBarrier.instantiate(factory)),
            floodedness: Arc::new(Noise::AquiferFluidLevelFloodedness.instantiate(factory)),
            spread: Arc::new(Noise::AquiferFluidLevelSpread.instantiate(factory)),
            lava: Arc::new(Noise::AquiferLava.instantiate(factory)),
            random: aquifer_source.fork_positional(),
        }
    }

    /// `router.barrierNoise()` = `noise(AQUIFER_BARRIER, xzScale = 1, yScale = 0.5)`.
    fn barrier(&self, x: f64, y: f64, z: f64) -> f64 {
        self.barrier.get_value(x, y * 0.5, z)
    }
    /// `noise(AQUIFER_FLUID_LEVEL_FLOODEDNESS, 1, 0.67)`.
    fn floodedness(&self, x: f64, y: f64, z: f64) -> f64 {
        self.floodedness.get_value(x, y * 0.67, z)
    }
    /// `noise(AQUIFER_FLUID_LEVEL_SPREAD, 1, 0.7142857142857143)`.
    fn spread(&self, x: f64, y: f64, z: f64) -> f64 {
        self.spread.get_value(x, y * 0.714_285_714_285_714_3, z)
    }
    /// `noise(AQUIFER_LAVA)` — unscaled.
    fn lava(&self, x: f64, y: f64, z: f64) -> f64 {
        self.lava.get_value(x, y, z)
    }
}

/// The per-chunk aquifer. Mirrors `Aquifer.NoiseBasedAquifer`, including its grid extent
/// and caches — the caches are pure memoisation, so results are position-deterministic
/// given the same chunk.
pub struct ChunkAquifer<'a> {
    ow: &'a Overworld,
    min_grid_x: i32,
    min_grid_y: i32,
    min_grid_z: i32,
    grid_size_x: i32,
    grid_size_z: i32,
    /// Above this Y the aquifer never samples cells — the global fluid picker decides.
    skip_sampling_above_y: i32,
    /// `aquiferLocationCache` / `aquiferCache`, indexed by [`Self::index`].
    locations: Vec<Option<(i32, i32, i32)>>,
    statuses: Vec<Option<FluidStatus>>,
    /// `NoiseChunk.preliminarySurfaceLevelCache`, keyed by the quart-quantised column.
    surface_cache: HashMap<(i32, i32), i32>,
}

impl<'a> ChunkAquifer<'a> {
    /// Build the aquifer for chunk `(chunk_x, chunk_z)` — the `NoiseBasedAquifer` constructor.
    pub fn new(ow: &'a Overworld, chunk_x: i32, chunk_z: i32) -> Self {
        let min_block_x = chunk_x * 16;
        let max_block_x = min_block_x + 15;
        let min_block_z = chunk_z * 16;
        let max_block_z = min_block_z + 15;

        let min_grid_x = grid_x(min_block_x - 5);
        let max_grid_x = grid_x(max_block_x - 5) + 1;
        let grid_size_x = max_grid_x - min_grid_x + 1;
        let min_grid_y = grid_y(MIN_BLOCK_Y + 1) - 1;
        let max_grid_y = grid_y(MIN_BLOCK_Y + Y_BLOCK_SIZE + 1) + 1;
        let grid_size_y = max_grid_y - min_grid_y + 1;
        let min_grid_z = grid_z(min_block_z - 5);
        let max_grid_z = grid_z(max_block_z - 5) + 1;
        let grid_size_z = max_grid_z - min_grid_z + 1;

        let total = (grid_size_x * grid_size_y * grid_size_z) as usize;
        let mut aq = Self {
            ow,
            min_grid_x,
            min_grid_y,
            min_grid_z,
            grid_size_x,
            grid_size_z,
            skip_sampling_above_y: i32::MAX,
            locations: vec![None; total],
            statuses: vec![None; total],
            surface_cache: HashMap::new(),
        };

        // skipSamplingAboveY, from the highest preliminary surface anywhere in the grid.
        let max_adjusted = adjust_surface_level(aq.max_preliminary_surface_level(
            from_grid_x(min_grid_x, 0),
            from_grid_z(min_grid_z, 0),
            from_grid_x(max_grid_x, X_RANGE - 1),
            from_grid_z(max_grid_z, Z_RANGE - 1),
        ));
        let skip_grid_y = grid_y(max_adjusted + Y_SPACING) + 1;
        aq.skip_sampling_above_y = from_grid_y(skip_grid_y, Y_SPACING - 1) - 1;
        aq
    }

    /// `computeSubstance(context, density)` — what to place at `(x, y, z)` given the
    /// terrain density there. `Solid` = keep the terrain block (Java's `null`).
    pub fn compute_substance(&mut self, x: i32, y: i32, z: i32, density: f64) -> Substance {
        if density > 0.0 {
            return Substance::Solid;
        }
        let global = global_fluid(y);
        if y > self.skip_sampling_above_y {
            return substance(global.at(y));
        }
        if global.at(y) == Some(Fluid::Lava) {
            return Substance::Lava;
        }

        // The 2×3×2 block of grid cells around the position; keep the nearest four.
        let x_anchor = grid_x(x - 5);
        let y_anchor = grid_y(y + 1);
        let z_anchor = grid_z(z - 5);
        let mut best: [(i32, usize); 4] = [(i32::MAX, 0); 4];
        for dx in 0..=1 {
            for dy in -1..=1 {
                for dz in 0..=1 {
                    let (gx, gy, gz) = (x_anchor + dx, y_anchor + dy, z_anchor + dz);
                    let index = self.index(gx, gy, gz);
                    let cached = self.locations[index];
                    let (cx, cy, cz) = cached.unwrap_or_else(|| {
                        // Cell centre = the grid node jittered by three draws, in x/y/z order.
                        let mut random = self.ow.aquifer().random.at(gx, gy, gz);
                        let loc = (
                            from_grid_x(gx, random.next_int_bound(X_RANGE)),
                            from_grid_y(gy, random.next_int_bound(Y_RANGE)),
                            from_grid_z(gz, random.next_int_bound(Z_RANGE)),
                        );
                        loc
                    });
                    self.locations[index] = Some((cx, cy, cz));
                    let (ddx, ddy, ddz) = (cx - x, cy - y, cz - z);
                    let dist = ddx * ddx + ddy * ddy + ddz * ddz;
                    // Insertion sort into the top-4, keeping Java's `>=` tie handling.
                    for slot in 0..4 {
                        if best[slot].0 >= dist {
                            best[slot..].rotate_right(1);
                            best[slot] = (dist, index);
                            break;
                        }
                    }
                }
            }
        }

        let status1 = self.status(best[0].1);
        let similarity12 = similarity(best[0].0, best[1].0);
        let fluid_state = status1.at(y);
        if similarity12 <= 0.0 {
            return substance(fluid_state);
        }
        // Water sitting directly on the lava table is always placed (no barrier check).
        if fluid_state == Some(Fluid::Water) && global_fluid(y - 1).at(y - 1) == Some(Fluid::Lava) {
            return substance(fluid_state);
        }

        // Barrier: a pressure gradient between differing fluid levels can re-solidify.
        let mut barrier_noise = None;
        let status2 = self.status(best[1].1);
        let pressure12 = self.pressure(x, y, z, &mut barrier_noise, status1, status2);
        if density + similarity12 * pressure12 > 0.0 {
            return Substance::Solid;
        }
        let status3 = self.status(best[2].1);
        let similarity13 = similarity(best[0].0, best[2].0);
        if similarity13 > 0.0 {
            let p = self.pressure(x, y, z, &mut barrier_noise, status1, status3);
            if density + similarity12 * similarity13 * p > 0.0 {
                return Substance::Solid;
            }
        }
        let similarity23 = similarity(best[1].0, best[2].0);
        if similarity23 > 0.0 {
            let p = self.pressure(x, y, z, &mut barrier_noise, status2, status3);
            if density + similarity12 * similarity23 * p > 0.0 {
                return Substance::Solid;
            }
        }
        substance(fluid_state)
    }

    /// `getIndex(gridX, gridY, gridZ)` into the per-chunk cell caches.
    fn index(&self, gx: i32, gy: i32, gz: i32) -> usize {
        let x = gx - self.min_grid_x;
        let y = gy - self.min_grid_y;
        let z = gz - self.min_grid_z;
        ((y * self.grid_size_z + z) * self.grid_size_x + x) as usize
    }

    /// `getAquiferStatus(index)` — the memoised fluid status of a cell.
    fn status(&mut self, index: usize) -> FluidStatus {
        if let Some(s) = self.statuses[index] {
            return s;
        }
        let (x, y, z) = self.locations[index].expect("cell location computed before its status");
        let s = self.compute_fluid(x, y, z);
        self.statuses[index] = Some(s);
        s
    }

    /// `computeFluid(x, y, z)` — decide a cell's fluid level and type from the terrain
    /// around it: it probes the preliminary surface at 13 chunk offsets, bails out to the
    /// global fluid where the cell is at/above the surface, and otherwise rolls a local
    /// water table.
    fn compute_fluid(&mut self, x: i32, y: i32, z: i32) -> FluidStatus {
        let global = global_fluid(y);
        let mut lowest_preliminary_surface = i32::MAX;
        let top_of_cell = y + Y_SPACING;
        let bottom_of_cell = y - Y_SPACING;
        let mut surface_at_center_is_under_global_fluid_level = false;

        for (i, &(ox, oz)) in SURFACE_SAMPLING_OFFSETS_IN_CHUNKS.iter().enumerate() {
            let sample_x = x + ox * 16;
            let sample_z = z + oz * 16;
            let preliminary = self.preliminary_surface_level(sample_x, sample_z);
            let adjusted = adjust_surface_level(preliminary);
            let start = i == 0; // the {0, 0} entry — the cell's own column
            if start && bottom_of_cell > adjusted {
                return global;
            }
            let pokes_above_surface = top_of_cell > adjusted;
            if pokes_above_surface || start {
                let at_surface = global_fluid(adjusted);
                if at_surface.at(adjusted).is_some() {
                    if start {
                        surface_at_center_is_under_global_fluid_level = true;
                    }
                    if pokes_above_surface {
                        return at_surface;
                    }
                }
            }
            lowest_preliminary_surface = lowest_preliminary_surface.min(preliminary);
        }

        let level = self.compute_surface_level(
            x,
            y,
            z,
            global,
            lowest_preliminary_surface,
            surface_at_center_is_under_global_fluid_level,
        );
        FluidStatus { level, fluid: self.compute_fluid_type(x, y, z, global, level) }
    }

    /// `computeSurfaceLevel` — the floodedness roll: fully flooded ⇒ the global sea, partly
    /// flooded ⇒ a randomised local table, otherwise no fluid at all.
    fn compute_surface_level(
        &self,
        x: i32,
        y: i32,
        z: i32,
        global: FluidStatus,
        lowest_preliminary_surface: i32,
        surface_at_center_is_under_global_fluid_level: bool,
    ) -> i32 {
        let (xf, yf, zf) = (x as f64, y as f64, z as f64);
        let (partially, fully) = if self.is_deep_dark_region(xf, yf, zf) {
            (-1.0, -1.0)
        } else {
            let distance_below_surface = (lowest_preliminary_surface + 8 - y) as f64;
            let floodedness_factor = if surface_at_center_is_under_global_fluid_level {
                clamped_map(distance_below_surface, 0.0, 64.0, 1.0, 0.0)
            } else {
                0.0
            };
            let noise = self.ow.aquifer().floodedness(xf, yf, zf).clamp(-1.0, 1.0);
            let fully_threshold = map(floodedness_factor, 1.0, 0.0, -0.3, 0.8);
            let partially_threshold = map(floodedness_factor, 1.0, 0.0, -0.8, 0.4);
            (noise - partially_threshold, noise - fully_threshold)
        };

        if fully > 0.0 {
            global.level
        } else if partially > 0.0 {
            self.randomized_fluid_surface_level(x, y, z, lowest_preliminary_surface)
        } else {
            WAY_BELOW_MIN_Y
        }
    }

    /// `computeRandomizedFluidSurfaceLevel` — a 16×40×16 cell's water table, quantised to 3.
    fn randomized_fluid_surface_level(&self, x: i32, y: i32, z: i32, lowest_preliminary_surface: i32) -> i32 {
        let cell_x = x.div_euclid(16);
        let cell_y = y.div_euclid(40);
        let cell_z = z.div_euclid(16);
        let middle_y = cell_y * 40 + 20;
        let spread = self.ow.aquifer().spread(cell_x as f64, cell_y as f64, cell_z as f64) * 10.0;
        let target = middle_y + quantize(spread, 3);
        lowest_preliminary_surface.min(target)
    }

    /// `computeFluidType` — deep, isolated tables can be lava instead of water.
    fn compute_fluid_type(&self, x: i32, y: i32, z: i32, global: FluidStatus, level: i32) -> Fluid {
        if level <= -10 && level != WAY_BELOW_MIN_Y && global.fluid != Fluid::Lava {
            let cell_x = x.div_euclid(64);
            let cell_y = y.div_euclid(40);
            let cell_z = z.div_euclid(64);
            let v = self.ow.aquifer().lava(cell_x as f64, cell_y as f64, cell_z as f64);
            if v.abs() > 0.3 {
                return Fluid::Lava;
            }
        }
        global.fluid
    }

    /// `calculatePressure` — how strongly the barrier between two aquifers pushes back.
    /// `barrier_noise` memoises the (position-only) barrier sample across the up-to-three
    /// calls per block, exactly as Java's `MutableDouble` does.
    fn pressure(
        &self,
        x: i32,
        y: i32,
        z: i32,
        barrier_noise: &mut Option<f64>,
        status1: FluidStatus,
        status2: FluidStatus,
    ) -> f64 {
        let type1 = status1.at(y);
        let type2 = status2.at(y);
        // Lava meeting water always gets a hard barrier.
        let lava_water = (type1 == Some(Fluid::Lava) && type2 == Some(Fluid::Water))
            || (type1 == Some(Fluid::Water) && type2 == Some(Fluid::Lava));
        if lava_water {
            return 2.0;
        }
        let fluid_y_diff = (status1.level - status2.level).abs();
        if fluid_y_diff == 0 {
            return 0.0;
        }

        let average_fluid_y = 0.5 * (status1.level + status2.level) as f64;
        let how_far_above_average = y as f64 + 0.5 - average_fluid_y;
        let base_value = fluid_y_diff as f64 / 2.0;
        let distance_from_edge = base_value - how_far_above_average.abs();
        // Above the average fluid point the barrier is thin (top bias 0), below it thick (3).
        let gradient = if how_far_above_average > 0.0 {
            let center = distance_from_edge;
            if center > 0.0 { center / 1.5 } else { center / 2.5 }
        } else {
            let center = 3.0 + distance_from_edge;
            if center > 0.0 { center / 3.0 } else { center / 10.0 }
        };

        // Outside ±2 the barrier is already decided, so the noise sample is skipped.
        let noise_value = if (-2.0..=2.0).contains(&gradient) {
            *barrier_noise.get_or_insert_with(|| {
                self.ow.aquifer().barrier(x as f64, y as f64, z as f64)
            })
        } else {
            0.0
        };
        2.0 * (noise_value + gradient)
    }

    /// `OverworldBiomeBuilder.isDeepDarkRegion` — deep-dark columns get no aquifer at all.
    /// The thresholds are float literals widened to double, so keep the widened values.
    fn is_deep_dark_region(&self, x: f64, y: f64, z: f64) -> bool {
        self.ow.erosion(x, z) < -0.225f32 as f64 && self.ow.depth(x, y, z) > 0.9f32 as f64
    }

    /// `NoiseChunk.preliminarySurfaceLevel` — quart-quantised in x/z, then memoised.
    fn preliminary_surface_level(&mut self, x: i32, z: i32) -> i32 {
        let key = ((x >> 2) << 2, (z >> 2) << 2);
        if let Some(&v) = self.surface_cache.get(&key) {
            return v;
        }
        let v = self.ow.preliminary_surface_level(key.0 as f64, key.1 as f64);
        self.surface_cache.insert(key, v);
        v
    }

    /// `NoiseChunk.maxPreliminarySurfaceLevel` — the highest surface on a 4-block lattice.
    fn max_preliminary_surface_level(&mut self, min_x: i32, min_z: i32, max_x: i32, max_z: i32) -> i32 {
        let mut max_y = i32::MIN;
        let mut z = min_z;
        while z <= max_z {
            let mut x = min_x;
            while x <= max_x {
                max_y = max_y.max(self.preliminary_surface_level(x, z));
                x += 4;
            }
            z += 4;
        }
        max_y
    }
}

/// `SURFACE_SAMPLING_OFFSETS_IN_CHUNKS` — the 13 chunk offsets a cell probes for the
/// surface. The `{0, 0}` entry must stay first: `computeFluid` treats it as the centre.
const SURFACE_SAMPLING_OFFSETS_IN_CHUNKS: [(i32, i32); 13] = [
    (0, 0),
    (-2, -1),
    (-1, -1),
    (0, -1),
    (1, -1),
    (-3, 0),
    (-2, 0),
    (-1, 0),
    (1, 0),
    (-2, 1),
    (-1, 1),
    (0, 1),
    (1, 1),
];

/// `adjustSurfaceLevel` — aquifers treat the surface as 8 blocks higher than the estimate.
#[inline]
fn adjust_surface_level(preliminary: i32) -> i32 {
    preliminary + 8
}

/// `similarity(d1, d2)` — 1 at equal distance, falling to 0 once 25 apart (squared).
#[inline]
fn similarity(distance_sqr1: i32, distance_sqr2: i32) -> f64 {
    1.0 - (distance_sqr2 - distance_sqr1) as f64 / 25.0
}

#[inline]
fn substance(fluid: Option<Fluid>) -> Substance {
    match fluid {
        None => Substance::Air,
        Some(Fluid::Water) => Substance::Water,
        Some(Fluid::Lava) => Substance::Lava,
    }
}

/// `Mth.quantize(value, resolution)` = `floor(value / resolution) * resolution`.
#[inline]
fn quantize(value: f64, resolution: i32) -> i32 {
    (value / resolution as f64).floor() as i32 * resolution
}

/// `Mth.map` — unclamped remap (unlike `clampedMap`).
#[inline]
fn map(value: f64, from_min: f64, from_max: f64, to_min: f64, to_max: f64) -> f64 {
    let t = (value - from_min) / (from_max - from_min);
    to_min + t * (to_max - to_min)
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

// -------- grid <-> block coordinates (X/Z spacing 16, Y spacing 12) --------

#[inline]
fn grid_x(block: i32) -> i32 {
    block >> 4
}
#[inline]
fn grid_z(block: i32) -> i32 {
    block >> 4
}
#[inline]
fn grid_y(block: i32) -> i32 {
    block.div_euclid(Y_SPACING)
}
#[inline]
fn from_grid_x(grid: i32, offset: i32) -> i32 {
    (grid << 4) + offset
}
#[inline]
fn from_grid_z(grid: i32, offset: i32) -> i32 {
    (grid << 4) + offset
}
#[inline]
fn from_grid_y(grid: i32, offset: i32) -> i32 {
    grid * Y_SPACING + offset
}

#[cfg(test)]
mod tests {
    use super::*;

    const SEED: i64 = 6954908675375307936;

    #[test]
    fn global_fluid_picker_matches_overworld_settings() {
        assert_eq!(global_fluid(0), FluidStatus { level: SEA_LEVEL, fluid: Fluid::Water });
        assert_eq!(global_fluid(62).at(62), Some(Fluid::Water));
        assert_eq!(global_fluid(63).at(63), None);
        assert_eq!(global_fluid(-55), FluidStatus { level: LAVA_LEVEL, fluid: Fluid::Lava });
        assert_eq!(global_fluid(-55).at(-55), Some(Fluid::Lava));
    }

    #[test]
    fn grid_helpers_round_trip_like_java() {
        // gridY uses floorDiv, so it must round toward negative infinity.
        assert_eq!(grid_y(-63), -6);
        assert_eq!(grid_y(-64), -6);
        assert_eq!(grid_y(0), 0);
        assert_eq!(grid_y(-1), -1);
        assert_eq!(grid_x(-5), -1);
        assert_eq!(from_grid_x(grid_x(37), 0), 32);
        assert_eq!(from_grid_y(grid_y(37), 0), 36);
    }

    #[test]
    fn quantize_and_map_match_mth() {
        assert_eq!(quantize(7.9, 3), 6);
        assert_eq!(quantize(-0.5, 3), -3);
        assert_eq!(map(1.0, 1.0, 0.0, -0.3, 0.8), -0.3);
        assert_eq!(map(0.0, 1.0, 0.0, -0.3, 0.8), 0.8);
    }

    #[test]
    fn ocean_columns_fill_with_water_to_sea_level() {
        // Below sea level in an ocean column, an air-density block must become water, and
        // the block just above sea level must stay air.
        let ow = Overworld::new(SEED);
        let mut found_ocean = false;
        'outer: for cx in -6..6 {
            for cz in -6..6 {
                let (x, z) = (cx * 16 + 8, cz * 16 + 8);
                // An ocean column: no solid terrain at y = 60.
                let d60 = ow.final_density(x, 60, z);
                if d60 > 0.0 {
                    continue;
                }
                let mut aq = ChunkAquifer::new(&ow, cx, cz);
                if aq.compute_substance(x, 60, z, d60) != Substance::Water {
                    continue;
                }
                let d64 = ow.final_density(x, 64, z);
                if d64 <= 0.0 {
                    assert_eq!(aq.compute_substance(x, 64, z, d64), Substance::Air);
                }
                found_ocean = true;
                break 'outer;
            }
        }
        assert!(found_ocean, "expected at least one water column near spawn");
    }

    #[test]
    fn solid_density_is_always_solid() {
        let ow = Overworld::new(SEED);
        let mut aq = ChunkAquifer::new(&ow, 0, 0);
        assert_eq!(aq.compute_substance(4, -50, 4, 1.0), Substance::Solid);
    }

    #[test]
    fn deterministic_across_aquifer_instances() {
        let ow = Overworld::new(SEED);
        for y in [-40, -5, 20, 55] {
            let d = ow.final_density(4, y, 4);
            let a = ChunkAquifer::new(&ow, 0, 0).compute_substance(4, y, 4, d);
            let b = ChunkAquifer::new(&ow, 0, 0).compute_substance(4, y, 4, d);
            assert_eq!(a, b, "aquifer substance at y={y} must be reproducible");
        }
    }
}
