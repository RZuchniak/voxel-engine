use crate::{
    block::BlockId,
    noise::OctavePerlinNoise,
    platform,
    world::{Chunk, MIN_SECTION_Y, SECTION_SIZE},
};

/// Minecraft overworld sea level.
pub const SEA_LEVEL: i32 = 63;

/// Lowest section base Y in this engine (`MIN_SECTION_Y * 16`).
const WORLD_MIN_Y: i32 = MIN_SECTION_Y * SECTION_SIZE as i32;

/// Terrain generator seeded like Minecraft — Perlin octaves with low/high/selector blend.
pub struct TerrainGenerator {
    low: OctavePerlinNoise,
    high: OctavePerlinNoise,
    selector: OctavePerlinNoise,
    depth: OctavePerlinNoise,
    continental: OctavePerlinNoise,
}

impl TerrainGenerator {
    pub fn new(seed: i64) -> Self {
        #[cfg(target_arch = "wasm32")]
        {
            Self {
                low: OctavePerlinNoise::create(seed, -7, 6),
                high: OctavePerlinNoise::create(seed.wrapping_add(1), -7, 5),
                selector: OctavePerlinNoise::create(seed.wrapping_add(2), -5, 4),
                depth: OctavePerlinNoise::create(seed.wrapping_add(3), -3, 3),
                continental: OctavePerlinNoise::create(seed.wrapping_add(4), -9, 4),
            }
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            Self {
                low: OctavePerlinNoise::create(seed, -7, 8),
                high: OctavePerlinNoise::create(seed.wrapping_add(1), -7, 6),
                selector: OctavePerlinNoise::create(seed.wrapping_add(2), -5, 5),
                depth: OctavePerlinNoise::create(seed.wrapping_add(3), -3, 4),
                continental: OctavePerlinNoise::create(seed.wrapping_add(4), -9, 6),
            }
        }
    }

    /// Surface block Y for a world column (solid top, not including water).
    pub fn height_at(&self, world_x: i32, world_z: i32) -> i32 {
        let x = world_x as f64;
        let z = world_z as f64;

        // Low noise: broad rolling hills (similar scale to MC low noise ~1/191 blocks).
        let low = self.low.sample_2d(x * 0.00522, z * 0.00522);
        // High noise: sharper peaks when selector is high (~1/60 blocks).
        let high = self.high.sample_2d(x * 0.01671, z * 0.01671);
        // Selector: blend low vs high terrain (MC selector noise).
        let mut select = self.selector.sample_2d(x * 0.00153, z * 0.00153);
        select = select.clamp(0.0, 1.0);

        let blended = low * (1.0 - select) + high * select;
        // Continentalness: push some areas below sea level for oceans.
        let continental = self.continental.sample_2d(x * 0.0008, z * 0.0008);
        let depth = self.depth.sample_2d(x * 0.02, z * 0.02) * 3.0;

        let height =
            SEA_LEVEL as f64 + blended * 28.0 + continental * 18.0 + depth;
        height.round() as i32
    }

    fn block_at_depth(&self, y: i32, surface_y: i32) -> BlockId {
        if y == 0 {
            BlockId::BEDROCK
        } else if y < 0 {
            BlockId::DEEPSLATE
        } else if y == surface_y {
            if surface_y <= SEA_LEVEL + 1 {
                BlockId::SAND
            } else if surface_y >= 90 {
                BlockId::SNOW_BLOCK
            } else {
                BlockId::GRASS
            }
        } else if y >= surface_y - 3 {
            if surface_y <= SEA_LEVEL + 1 {
                BlockId::SAND
            } else {
                BlockId::DIRT
            }
        } else {
            BlockId::STONE
        }
    }
}

pub fn generate_chunk(generator: &TerrainGenerator, coord: (i32, i32)) -> Chunk {
    let (chunk_x, chunk_z) = coord;
    let mut chunk = Chunk::new(coord);
    let depth = platform::surface_mesh_depth_blocks().max(4);

    for local_z in 0..SECTION_SIZE {
        for local_x in 0..SECTION_SIZE {
            let world_x = chunk_x * SECTION_SIZE as i32 + local_x as i32;
            let world_z = chunk_z * SECTION_SIZE as i32 + local_z as i32;
            let surface_y = generator.height_at(world_x, world_z);
            let column_top = surface_y.max(SEA_LEVEL);
            let column_bottom = (surface_y - depth).min(SEA_LEVEL - depth).max(WORLD_MIN_Y);

            for y in column_bottom..=column_top {
                let block = if y > surface_y {
                    BlockId::WATER
                } else {
                    generator.block_at_depth(y, surface_y)
                };
                chunk.set_block_world(local_x, y, local_z, block);
            }
        }
    }
    chunk
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_seed_same_height() {
        let a = TerrainGenerator::new(12345);
        let b = TerrainGenerator::new(12345);
        assert_eq!(a.height_at(0, 0), b.height_at(0, 0));
        assert_eq!(a.height_at(-100, 200), b.height_at(-100, 200));
    }

    #[test]
    fn different_seed_different_height() {
        let a = TerrainGenerator::new(1);
        let b = TerrainGenerator::new(999_999_991);
        let differs = (0..32).any(|i| {
            a.height_at(i * 17, i * -13) != b.height_at(i * 17, i * -13)
        });
        assert!(differs, "expected at least one column to differ between seeds");
    }
}
