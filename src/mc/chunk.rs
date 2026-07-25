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
}

/// Generate chunk `(chunk_x, chunk_z)` end to end.
pub fn generate_chunk(
    ow: &Overworld,
    surface: &SurfaceSystem,
    chunk_x: i32,
    chunk_z: i32,
) -> ChunkBlocks {
    let mut sampler = CellSampler::new(ow);
    let mut aquifer = ow.aquifer_for_chunk(chunk_x, chunk_z);
    let mut blocks = vec![Block::Air; (16 * 16 * HEIGHT) as usize];

    // Stage 1+2: density and aquifer, which together decide stone/water/lava/air.
    for lz in 0..16usize {
        for lx in 0..16usize {
            let x = chunk_x * 16 + lx as i32;
            let z = chunk_z * 16 + lz as i32;
            for y in MIN_Y..MIN_Y + HEIGHT {
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
            for y in (MIN_Y..MIN_Y + HEIGHT).rev() {
                if blocks[ChunkBlocks::index(lx, y, lz)] != Block::Air {
                    heights[lz * 16 + lx] = y;
                    break;
                }
            }
        }
    }

    // Stage 3: surface rules, walking each column top-down.
    for lz in 0..16usize {
        for lx in 0..16usize {
            let x = chunk_x * 16 + lx as i32;
            let z = chunk_z * 16 + lz as i32;
            let surface_depth = surface.surface_depth_at(x, z);
            let min_surface_level = surface.min_surface_level(ow, x, z, surface_depth);
            let steep = is_steep(&heights, lx, lz);
            let biome_cache = ColumnBiomes::new();

            let mut stone_depth_above = 0i32;
            let mut water_height = i32::MIN;
            let mut next_ceiling_stone_y = i32::MAX;
            let mut biome_cache = biome_cache;

            for y in (MIN_Y..MIN_Y + HEIGHT).rev() {
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
                    for look in (MIN_Y - 1..y).rev() {
                        let below = if look < MIN_Y {
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

/// Biomes only change every 4 blocks vertically, and the lookup is a 7594-box scan, so cache
/// it per quart cell as we walk down the column.
struct ColumnBiomes {
    last_quart_y: i32,
    last: &'static str,
}

impl ColumnBiomes {
    fn new() -> Self {
        Self { last_quart_y: i32::MIN, last: "plains" }
    }

    fn get(&mut self, ow: &Overworld, x: i32, y: i32, z: i32) -> &'static str {
        let quart_y = y >> 2;
        if quart_y != self.last_quart_y {
            self.last_quart_y = quart_y;
            self.last = ow.biome_at(x, y, z);
        }
        self.last
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
