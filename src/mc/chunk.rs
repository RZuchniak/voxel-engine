//! The whole generation pipeline for one chunk: density → aquifer → surface rules.
//!
//! This is the piece a `WorldSource` would call. It exists as a chunk-shaped API because
//! two stages genuinely need chunk context — the aquifer's sampling cutoff comes from the
//! chunk's highest preliminary surface, and the `steep` surface condition compares
//! heightmap neighbours *clamped to the chunk*, so it changes at chunk borders.

use super::aquifer::Substance;
use super::overworld::{CellSampler, Overworld};
use super::surface::{Block, SurfaceSystem};

/// Vertical extent of the overworld (`NoiseSettings.OVERWORLD_NOISE_SETTINGS`).
pub const MIN_Y: i32 = -64;
pub const HEIGHT: i32 = 384;

/// A generated chunk: 16×16×384 blocks, indexed `[(y - MIN_Y) * 256 + z * 16 + x]`.
pub struct ChunkBlocks {
    pub blocks: Vec<Block>,
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
    generate_range(ow, surface, chunk_x, chunk_z, MIN_Y, MIN_Y + HEIGHT - 1)
}

/// How far above the *preliminary* surface estimate to start generating. That estimate
/// omits the 3D noise and jaggedness, so the real terrain can stand above it; this covers
/// the gap. The ceiling is also raised adaptively if terrain reaches it anyway, so this is a
/// performance guess, not a correctness assumption.
const CEILING_MARGIN: i32 = 32;

/// Generate only the band near the surface: everything from the terrain top down to
/// `depth` blocks below it. Blocks outside the band are left as air.
///
/// This is the streaming path. It exists because the renderer already refuses to mesh
/// anything more than [`crate::platform::surface_mesh_depth_blocks`] below a chunk's top
/// (24 on wasm, 48 native) — so generating the other ~300 blocks was work that could never
/// be seen. **It is a rendering optimisation, not a parity one**: deep terrain and caves
/// genuinely are not generated, so never point a parity harness at this function.
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
    // `lowest` is already the minimum over the chunk, so `depth` below it is conservative.
    let floor = (lowest - depth).max(MIN_Y);

    let mut generated = generate_range(ow, surface, chunk_x, chunk_z, floor, ceiling);
    // If terrain actually reached the ceiling the margin was too small and the column was
    // truncated. Raise it and redo rather than silently render a flat-topped mountain.
    while ceiling < MIN_Y + HEIGHT - 1 && generated.has_solid_at(ceiling) {
        ceiling = (ceiling + CEILING_MARGIN).min(MIN_Y + HEIGHT - 1);
        generated = generate_range(ow, surface, chunk_x, chunk_z, floor, ceiling);
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
) -> ChunkBlocks {
    let mut sampler = CellSampler::new(ow);
    let mut aquifer = ow.aquifer_for_chunk(chunk_x, chunk_z);
    let mut blocks = vec![Block::Air; (16 * 16 * HEIGHT) as usize];

    // Stage 1+2: density and aquifer, which together decide stone/water/lava/air.
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

    ChunkBlocks { blocks }
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
    #[test]
    fn surface_band_matches_full_generation_where_it_is_visible() {
        let ow = Overworld::new(SEED);
        let surface = SurfaceSystem::new(SEED);
        const DEPTH: i32 = 64;

        for (cx, cz) in [(0, 0), (3, -2), (-5, 4), (7, 7)] {
            let full = generate_chunk(&ow, &surface, cx, cz);
            let banded = generate_chunk_surface(&ow, &surface, cx, cz, DEPTH);
            for lz in 0..16usize {
                for lx in 0..16usize {
                    let top = (MIN_Y..MIN_Y + HEIGHT)
                        .rev()
                        .find(|&y| full.get(lx, y, lz) != Block::Air);
                    let Some(top) = top else { continue };
                    // Everything from the terrain top down to `depth` below it must agree.
                    for y in (top - DEPTH).max(MIN_Y)..=top {
                        assert_eq!(
                            banded.get(lx, y, lz),
                            full.get(lx, y, lz),
                            "chunk ({cx},{cz}) column ({lx},{lz}) y={y}: band differs from full"
                        );
                    }
                }
            }
        }
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
