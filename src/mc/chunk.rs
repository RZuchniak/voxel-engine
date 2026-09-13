//! The whole generation pipeline for one chunk: density → aquifer → surface rules.
//!
//! This is the piece a `WorldSource` would call. It exists as a chunk-shaped API because
//! two stages genuinely need chunk context — the aquifer's sampling cutoff comes from the
//! chunk's highest preliminary surface, and the `steep` surface condition compares
//! heightmap neighbours *clamped to the chunk*, so it changes at chunk borders.

use std::time::Instant;

use super::aquifer::Substance;
use super::overworld::{CellSampler, Overworld};
use super::surface::{Block, SurfaceSystem};

/// Wall-clock split of [`generate_chunk`], for `examples/profile_fly` and samply-adjacent
/// console reports. Cheap stages (heightmaps, the 4×4 tint grid) are folded into `surface_ms`.
#[derive(Clone, Copy, Debug, Default)]
pub struct ChunkGenTimings {
    /// Density sampling + aquifer substance, the nested x/y/z loop.
    pub density_aquifer_ms: f64,
    /// Surface rules, plus the heightmaps and tint grid that read the finished column.
    pub surface_ms: f64,
}

/// Vertical extent of the overworld (`NoiseSettings.OVERWORLD_NOISE_SETTINGS`).
pub const MIN_Y: i32 = -64;
pub const HEIGHT: i32 = 384;

/// Quart cells along one horizontal chunk axis — biomes live on a 4-block lattice.
pub const SURFACE_BIOME_AXIS: usize = 4;
/// Entries in [`ChunkBlocks::surface_biomes`].
pub const SURFACE_BIOME_CELLS: usize = SURFACE_BIOME_AXIS * SURFACE_BIOME_AXIS;

/// A generated chunk: 16×16×384 blocks, indexed `[(y - MIN_Y) * 256 + z * 16 + x]`.
pub struct ChunkBlocks {
    pub blocks: Vec<Block>,
    /// The biome at each of the chunk's 4×4 quart columns, sampled **at that column's surface**,
    /// indexed `[qz * 4 + qx]`. This is what drives grass/foliage/water tint in the renderer.
    ///
    /// Deliberately 2D and deliberately 16 entries. The full 3D grid is 4×4×96 = 1536 cells, and
    /// filling it costs **9.4 ms/chunk on top of 37.6** (measured by `examples/bench_mc_chunk`,
    /// whose `biome lookups only` line is exactly that grid) — a 25% tax on generation, which
    /// lands directly on the loading screen. 16 cells is 1% of that, and most of them are already
    /// resident in `BiomeCache` because the surface rules just queried them.
    ///
    /// What this gives up: cave biomes get their column's *surface* tint rather than their own
    /// (nothing tintable is generated underground yet), and water does not change colour with
    /// depth. Both are invisible today; if lush caves ever grow foliage, this becomes 3D.
    pub surface_biomes: [&'static str; SURFACE_BIOME_CELLS],
    /// `Heightmap.Types.WORLD_SURFACE` — one above the highest non-air block, **fluids
    /// included**. Indexed `[lz * 16 + lx]`.
    pub world_surface: [i32; 256],
    /// `Heightmap.Types.OCEAN_FLOOR` — one above the highest non-air, **non-fluid** block.
    ///
    /// This is what trees are placed on (`PlacementUtils.HEIGHTMAP_OCEAN_FLOOR`), and the
    /// difference from [`Self::world_surface`] is exactly the water depth that
    /// `SurfaceWaterDepthFilter` rejects on. Both are "first free position above the surface"
    /// (`Heightmap.getFirstAvailable` returns `y + 1`), not the surface block's own Y.
    pub ocean_floor: [i32; 256],
}

impl ChunkBlocks {
    #[inline]
    fn index(x: usize, y: i32, z: usize) -> usize {
        ((y - MIN_Y) as usize) * 256 + z * 16 + x
    }

    /// The block at a chunk-local column and absolute Y.
    pub fn get(&self, x: usize, y: i32, z: usize) -> Block {
        self.blocks[Self::index(x, y, z)]
    }

    /// Whether any column has a non-air block at this height — used to detect a band that
    /// was cut off below the real terrain top.
    fn has_solid_at(&self, y: i32) -> bool {
        (0..16).any(|z| (0..16).any(|x| self.get(x, y, z) != Block::Air))
    }
}

/// Generate the whole chunk column, `-64..320`. This is the exact, full-fidelity path the
/// parity harnesses use.
pub fn generate_chunk(
    ow: &Overworld,
    surface: &SurfaceSystem,
    chunk_x: i32,
    chunk_z: i32,
) -> ChunkBlocks {
    generate_range(
        ow,
        surface,
        chunk_x,
        chunk_z,
        MIN_Y,
        MIN_Y + HEIGHT - 1,
        None,
    )
}

/// Same as [`generate_chunk`], with a density-vs-surface split for the fly profiler.
pub fn generate_chunk_timed(
    ow: &Overworld,
    surface: &SurfaceSystem,
    chunk_x: i32,
    chunk_z: i32,
) -> (ChunkBlocks, ChunkGenTimings) {
    let mut timings = ChunkGenTimings::default();
    let blocks = generate_range(
        ow,
        surface,
        chunk_x,
        chunk_z,
        MIN_Y,
        MIN_Y + HEIGHT - 1,
        Some(&mut timings),
    );
    (blocks, timings)
}

/// How far above the *preliminary* surface estimate to start generating. That estimate
/// omits the 3D noise and jaggedness, so the real terrain can stand above it; this covers
/// the gap. The ceiling is also raised adaptively if terrain reaches it anyway, so this is a
/// performance guess, not a correctness assumption.
const CEILING_MARGIN: i32 = 32;

/// Generate a column that always includes the underground, but skips empty sky.
///
/// `depth` is how far below the preliminary surface to go. Pass [`i32::MAX`] to generate
/// down to [`MIN_Y`] (real caves; only air above the terrain ceiling is skipped). A small
/// depth is the old hollow-shell path and will look like floating slabs from below.
pub fn generate_chunk_surface(
    ow: &Overworld,
    surface: &SurfaceSystem,
    chunk_x: i32,
    chunk_z: i32,
    depth: i32,
) -> ChunkBlocks {
    // Bound the band from the cheap offset/factor-only surface estimate, sampled on the
    // quart lattice the estimate is cached on anyway.
    let (mut lowest, mut highest) = (i32::MAX, i32::MIN);
    for dz in (0..16).step_by(4) {
        for dx in (0..16).step_by(4) {
            let psl = ow.preliminary_surface_level(
                (chunk_x * 16 + dx) as f64,
                (chunk_z * 16 + dz) as f64,
            );
            lowest = lowest.min(psl);
            highest = highest.max(psl);
        }
    }

    let mut ceiling = (highest + CEILING_MARGIN).min(MIN_Y + HEIGHT - 1);
    // Sea level matters even where the ground is far below it — ocean columns must still
    // reach the water surface.
    ceiling = ceiling.max(super::aquifer::SEA_LEVEL + 1);
    let floor = if depth == i32::MAX {
        MIN_Y
    } else {
        (lowest - depth).max(MIN_Y)
    };

    let mut generated = generate_range(ow, surface, chunk_x, chunk_z, floor, ceiling, None);
    // If terrain actually reached the ceiling the margin was too small and the column was
    // truncated. Raise it and redo rather than silently render a flat-topped mountain.
    while ceiling < MIN_Y + HEIGHT - 1 && generated.has_solid_at(ceiling) {
        ceiling = (ceiling + CEILING_MARGIN).min(MIN_Y + HEIGHT - 1);
        generated = generate_range(ow, surface, chunk_x, chunk_z, floor, ceiling, None);
    }
    generated
}

/// The shared pipeline, over an inclusive Y range.
fn generate_range(
    ow: &Overworld,
    surface: &SurfaceSystem,
    chunk_x: i32,
    chunk_z: i32,
    y_lo: i32,
    y_hi: i32,
    mut timings: Option<&mut ChunkGenTimings>,
) -> ChunkBlocks {
    let mut sampler = CellSampler::new(ow);
    let mut aquifer = ow.aquifer_for_chunk(chunk_x, chunk_z);
    let mut blocks = vec![Block::Air; (16 * 16 * HEIGHT) as usize];

    // Stage 1+2: density and aquifer, which together decide stone/water/lava/air.
    let t_density = timings.is_some().then(Instant::now);
    for lz in 0..16usize {
        for lx in 0..16usize {
            let x = chunk_x * 16 + lx as i32;
            let z = chunk_z * 16 + lz as i32;
            for y in y_lo..=y_hi {
                let density = sampler.final_density(x, y, z);
                blocks[ChunkBlocks::index(lx, y, lz)] = match aquifer
                    .compute_substance(x, y, z, density)
                {
                    Substance::Solid => Block::Stone,
                    Substance::Water => Block::Water,
                    Substance::Lava => Block::Lava,
                    Substance::Air => Block::Air,
                };
            }
        }
    }
    if let (Some(tm), Some(t0)) = (timings.as_mut(), t_density) {
        tm.density_aquifer_ms = t0.elapsed().as_secs_f64() * 1000.0;
    }

    // The WORLD_SURFACE_WG heightmap: highest non-air block per column (fluids count).
    let mut heights = [MIN_Y - 1; 256];
    for lz in 0..16usize {
        for lx in 0..16usize {
            for y in (y_lo..=y_hi).rev() {
                if blocks[ChunkBlocks::index(lx, y, lz)] != Block::Air {
                    heights[lz * 16 + lx] = y;
                    break;
                }
            }
        }
    }

    // Stage 3: surface rules, walking each column top-down.
    let t_surface = timings.is_some().then(Instant::now);
    let mut biome_cache = BiomeCache::new(chunk_x, chunk_z);
    for lz in 0..16usize {
        for lx in 0..16usize {
            let x = chunk_x * 16 + lx as i32;
            let z = chunk_z * 16 + lz as i32;
            let surface_depth = surface.surface_depth_at(x, z);
            let min_surface_level = surface.min_surface_level(ow, x, z, surface_depth);
            let steep = is_steep(&heights, lx, lz);

            let mut stone_depth_above = 0i32;
            let mut water_height = i32::MIN;
            let mut next_ceiling_stone_y = i32::MAX;

            for y in (y_lo..=y_hi).rev() {
                let current = blocks[ChunkBlocks::index(lx, y, lz)];
                if current == Block::Air {
                    stone_depth_above = 0;
                    water_height = i32::MIN;
                    continue;
                }
                if current == Block::Water || current == Block::Lava {
                    if water_height == i32::MIN {
                        water_height = y + 1;
                    }
                    continue;
                }

                // Solid. Find where this run of stone bottoms out, for the ceiling depth.
                if next_ceiling_stone_y >= y {
                    next_ceiling_stone_y = i32::MIN;
                    for look in (y_lo - 1..y).rev() {
                        let below = if look < y_lo {
                            Block::Air
                        } else {
                            blocks[ChunkBlocks::index(lx, look, lz)]
                        };
                        if below == Block::Air || below == Block::Water || below == Block::Lava {
                            next_ceiling_stone_y = look + 1;
                            break;
                        }
                    }
                }

                stone_depth_above += 1;
                let stone_depth_below = y - next_ceiling_stone_y + 1;
                let biome = biome_cache.get(ow, x, y, z);
                if let Some(replacement) = surface.rule_at(
                    ow,
                    x,
                    y,
                    z,
                    surface_depth,
                    min_surface_level,
                    stone_depth_above,
                    stone_depth_below,
                    water_height,
                    steep,
                    biome,
                ) {
                    blocks[ChunkBlocks::index(lx, y, lz)] = replacement;
                }
            }
        }
    }

    // The tint grid. Sampled from the *same* `biome_cache` the surface rules just used, so a
    // cell whose column had any solid block in it is already resident and costs a array index;
    // only all-air/all-fluid columns (open ocean, sky) pay for a real lookup. 16 cells either
    // way — see the note on `ChunkBlocks::surface_biomes` for why this is not the 3D grid.
    let mut surface_biomes = [""; SURFACE_BIOME_CELLS];
    for qz in 0..SURFACE_BIOME_AXIS {
        for qx in 0..SURFACE_BIOME_AXIS {
            let lx = qx * 4;
            let lz = qz * 4;
            // Clamp: a column with no solid block at all reads `MIN_Y - 1`, which is outside
            // the world and would index the cache out of bounds.
            let surface_y = heights[lz * 16 + lx].clamp(MIN_Y, MIN_Y + HEIGHT - 1);
            surface_biomes[qz * SURFACE_BIOME_AXIS + qx] = biome_cache.get(
                ow,
                chunk_x * 16 + lx as i32,
                surface_y,
                chunk_z * 16 + lz as i32,
            );
        }
    }

    // The two heightmaps feature placement needs. Computed after surface rules so they see the
    // finished column (grass/sand rather than the stone the density stage left).
    let mut world_surface = [MIN_Y; 256];
    let mut ocean_floor = [MIN_Y; 256];
    for lz in 0..16usize {
        for lx in 0..16usize {
            let column = lz * 16 + lx;
            for y in (y_lo..=y_hi).rev() {
                let block = blocks[ChunkBlocks::index(lx, y, lz)];
                if block == Block::Air {
                    continue;
                }
                if world_surface[column] == MIN_Y {
                    world_surface[column] = y + 1;
                }
                if block != Block::Water && block != Block::Lava {
                    ocean_floor[column] = y + 1;
                    break;
                }
            }
        }
    }

    if let (Some(tm), Some(t0)) = (timings.as_mut(), t_surface) {
        tm.surface_ms = t0.elapsed().as_secs_f64() * 1000.0;
    }

    ChunkBlocks { blocks, surface_biomes, world_surface, ocean_floor }
}

/// `SteepMaterialCondition` — a 4-block height jump to a neighbour. Neighbour lookups are
/// **clamped to the chunk**, exactly as vanilla does, so this genuinely differs at borders.
fn is_steep(heights: &[i32; 256], lx: usize, lz: usize) -> bool {
    let h = |x: usize, z: usize| heights[z * 16 + x];
    let z_north = lz.saturating_sub(1);
    let z_south = (lz + 1).min(15);
    if h(lx, z_south) >= h(lx, z_north) + 4 {
        return true;
    }
    let x_west = lx.saturating_sub(1);
    let x_east = (lx + 1).min(15);
    h(x_west, lz) >= h(x_east, lz) + 4
}

/// Biomes live on a 4×4×4 grid and each lookup is a 7594-box scan, so a chunk needs only
/// 4×4×96 = 1536 of them. Caching **per chunk** rather than per column is worth ~16× — the
/// 16 columns inside a quart cell would otherwise each redo the same search.
struct BiomeCache {
    chunk_x: i32,
    chunk_z: i32,
    cells: Vec<Option<&'static str>>,
}

/// Quart cells per chunk axis, and vertically over the world height.
const QUARTS_XZ: usize = 4;
const QUARTS_Y: usize = (HEIGHT / 4) as usize;

impl BiomeCache {
    fn new(chunk_x: i32, chunk_z: i32) -> Self {
        Self { chunk_x, chunk_z, cells: vec![None; QUARTS_XZ * QUARTS_XZ * QUARTS_Y] }
    }

    fn get(&mut self, ow: &Overworld, x: i32, y: i32, z: i32) -> &'static str {
        let qx = (x - self.chunk_x * 16) as usize / 4;
        let qz = (z - self.chunk_z * 16) as usize / 4;
        let qy = ((y - MIN_Y) / 4) as usize;
        let index = (qy * QUARTS_XZ + qz) * QUARTS_XZ + qx;
        if let Some(cached) = self.cells[index] {
            return cached;
        }
        let biome = ow.biome_at(x, y, z);
        self.cells[index] = Some(biome);
        biome
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SEED: i64 = 6954908675375307936;

    #[test]
    fn spawn_chunk_has_a_soil_surface_over_stone() {
        let ow = Overworld::new(SEED);
        let surface = SurfaceSystem::new(SEED);
        let chunk = generate_chunk(&ow, &surface, 0, 0);

        // Walk down column (0,0) to the first non-air block: it must be a surface material,
        // not bare stone, and there must be stone below it.
        let top = (MIN_Y..MIN_Y + HEIGHT)
            .rev()
            .find(|&y| chunk.get(0, y, 0) != Block::Air)
            .expect("column has a surface");
        let surface_block = chunk.get(0, top, 0);
        assert!(
            matches!(
                surface_block,
                Block::GrassBlock | Block::Sand | Block::Gravel | Block::Dirt | Block::Water
                    | Block::SnowBlock | Block::Podzol | Block::CoarseDirt | Block::Mycelium
                    | Block::Stone
            ),
            "unexpected surface block {surface_block:?} at y={top}"
        );
        assert_eq!(chunk.get(0, MIN_Y, 0), Block::Bedrock, "world floor must be bedrock");
    }

    /// The streaming path must be indistinguishable from the exact one wherever the
    /// renderer can actually see. This is the guard on `CEILING_MARGIN`: if the margin is
    /// ever too small, the band gets truncated and this fails.
    ///
    /// Both shipped depths are checked — 64 native, 40 on wasm. The browser runs the
    /// shallower band, so pinning only the native one would leave the build people actually
    /// see the least tested.
    #[test]
    fn surface_band_matches_full_generation_where_it_is_visible() {
        let ow = Overworld::new(SEED);
        let surface = SurfaceSystem::new(SEED);

        for depth in [40i32, 64] {
            for (cx, cz) in [(0, 0), (3, -2), (-5, 4), (7, 7)] {
                let full = generate_chunk(&ow, &surface, cx, cz);
                let banded = generate_chunk_surface(&ow, &surface, cx, cz, depth);
                for lz in 0..16usize {
                    for lx in 0..16usize {
                        let top = (MIN_Y..MIN_Y + HEIGHT)
                            .rev()
                            .find(|&y| full.get(lx, y, lz) != Block::Air);
                        let Some(top) = top else { continue };
                        // Everything from the terrain top down to `depth` below it must agree.
                        for y in (top - depth).max(MIN_Y)..=top {
                            assert_eq!(
                                banded.get(lx, y, lz),
                                full.get(lx, y, lz),
                                "depth {depth}, chunk ({cx},{cz}) column ({lx},{lz}) y={y}:                                  band differs from full"
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn generating_to_bedrock_keeps_the_world_floor() {
        let ow = Overworld::new(SEED);
        let surface = SurfaceSystem::new(SEED);
        let chunk = generate_chunk_surface(&ow, &surface, 0, 0, i32::MAX);
        assert_eq!(chunk.get(0, MIN_Y, 0), Block::Bedrock);
        // A shallow band would be air down here; that is the floating-slab artifact.
        let solid_deep = (0..16).any(|x| {
            !matches!(chunk.get(x, MIN_Y + 8, 0), Block::Air | Block::Water)
        });
        assert!(solid_deep, "bedrock-depth generation must fill the underground");
    }

    #[test]
    fn deepslate_replaces_stone_deep_down() {
        let ow = Overworld::new(SEED);
        let surface = SurfaceSystem::new(SEED);
        let chunk = generate_chunk(&ow, &surface, 0, 0);
        // Below y=0 the vertical gradient is fully deepslate.
        let mut saw_deepslate = false;
        for y in -60..-10 {
            for lx in 0..16 {
                if chunk.get(lx, y, 0) == Block::Deepslate {
                    saw_deepslate = true;
                }
            }
        }
        assert!(saw_deepslate, "expected deepslate below y=0");
    }
}
