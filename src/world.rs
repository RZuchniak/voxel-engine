use std::collections::HashMap;
use std::sync::Arc;

use crate::block::BlockId;

pub const SECTION_SIZE: usize = 16;
pub const SECTION_VOLUME: usize = SECTION_SIZE * SECTION_SIZE * SECTION_SIZE;
pub const MIN_SECTION_Y: i32 = -4; // world y = -64
pub const SECTION_COUNT: usize = 24; // -64..320

#[derive(Clone)]
pub struct Section {
    data: [BlockId; SECTION_VOLUME],
}

impl Section {
    pub fn new() -> Self {
        Self {
            data: [BlockId::AIR; SECTION_VOLUME],
        }
    }

    #[inline]
    fn index(x: usize, y: usize, z: usize) -> usize {
        x + y * SECTION_SIZE + z * SECTION_SIZE * SECTION_SIZE
    }

    #[inline]
    pub fn set_block(&mut self, x: usize, y: usize, z: usize, block: BlockId) {
        let idx = Self::index(x, y, z);
        self.data[idx] = block;
    }

    #[inline]
    pub fn block_at(&self, x: usize, y: usize, z: usize) -> BlockId {
        self.data[Self::index(x, y, z)]
    }

    #[inline]
    pub fn block_data(&self) -> &[BlockId] {
        &self.data
    }

    #[inline]
    pub fn has_any_non_air(&self) -> bool {
        self.data.iter().any(|b| !b.is_air())
    }
}

/// Quart cells along one horizontal chunk axis; see [`Chunk::biomes`].
pub const BIOME_AXIS: usize = 4;
pub const BIOME_CELLS: usize = BIOME_AXIS * BIOME_AXIS;

#[derive(Clone)]
pub struct Chunk {
    coord: (i32, i32),
    sections: Vec<Option<Section>>,
    /// Biome index (into `biome_tint::BIOME_TINTS`) for each 4×4 quart column, `[qz * 4 + qx]`.
    ///
    /// **16 bytes per chunk**, against ~196 KB of block data — a rounding error, which is the
    /// whole reason the tint grid is stored per column rather than per block. `None` when the
    /// source did not supply biomes (the Anvil import path), in which case faces fall back to
    /// [`crate::biome_tint::NEUTRAL`] and render exactly as they did before tinting existed.
    biomes: Option<[u8; BIOME_CELLS]>,
}

impl Chunk {
    pub fn new(coord: (i32, i32)) -> Self {
        Self {
            coord,
            sections: (0..SECTION_COUNT).map(|_| None).collect(),
            biomes: None,
        }
    }

    pub fn coord(&self) -> (i32, i32) {
        self.coord
    }

    pub fn set_biomes(&mut self, biomes: [u8; BIOME_CELLS]) {
        self.biomes = Some(biomes);
    }

    pub fn biomes(&self) -> Option<&[u8; BIOME_CELLS]> {
        self.biomes.as_ref()
    }

    /// The biome index covering a chunk-local column, or `None` if this chunk carries no biome
    /// data. Out-of-range coordinates clamp rather than wrap: the mesher reads a chunk's
    /// neighbours by world coordinate, and a wrap would tint a border quad with the colour from
    /// the far side of the chunk.
    pub fn biome_at_local(&self, local_x: i32, local_z: i32) -> Option<u8> {
        let biomes = self.biomes.as_ref()?;
        let qx = (local_x.clamp(0, SECTION_SIZE as i32 - 1) / 4) as usize;
        let qz = (local_z.clamp(0, SECTION_SIZE as i32 - 1) / 4) as usize;
        Some(biomes[qz * BIOME_AXIS + qx])
    }

    pub fn section(&self, section_index: usize) -> Option<&Section> {
        self.sections.get(section_index).and_then(|s| s.as_ref())
    }

    pub fn populated_section_indices(&self) -> impl Iterator<Item = usize> + '_ {
        self.sections
            .iter()
            .enumerate()
            .filter_map(|(idx, section)| section.as_ref().map(|_| idx))
    }

    pub fn insert_section(&mut self, section_index: usize, section: Section) {
        if section_index < self.sections.len() {
            self.sections[section_index] = Some(section);
        }
    }

    /// True if any stored section contains a non-air block (meshes may still be empty if fully occluded).
    pub fn needs_rendered_mesh(&self) -> bool {
        self.populated_section_indices()
            .any(|idx| self.section(idx).is_some_and(|s| s.has_any_non_air()))
    }

    /// Highest world Y containing a non-air block, if any.
    pub fn max_nonempty_world_y(&self) -> Option<i32> {
        for idx in (0..SECTION_COUNT).rev() {
            if self.section(idx).is_some_and(|s| s.has_any_non_air()) {
                let section_base = (idx as i32 + MIN_SECTION_Y) * SECTION_SIZE as i32;
                return Some(section_base + SECTION_SIZE as i32 - 1);
            }
        }
        None
    }

    fn section_index_from_world_y(world_y: i32) -> Option<usize> {
        let section_y = world_y.div_euclid(SECTION_SIZE as i32);
        let idx = section_y - MIN_SECTION_Y;
        if idx >= 0 && (idx as usize) < SECTION_COUNT {
            Some(idx as usize)
        } else {
            None
        }
    }

    pub fn set_block_world(&mut self, local_x: usize, world_y: i32, local_z: usize, block: BlockId) {
        if local_x >= SECTION_SIZE || local_z >= SECTION_SIZE {
            return;
        }
        let Some(section_index) = Self::section_index_from_world_y(world_y) else {
            return;
        };
        let local_y = world_y.rem_euclid(SECTION_SIZE as i32) as usize;
        let section = self.sections[section_index].get_or_insert_with(Section::new);
        section.set_block(local_x, local_y, local_z, block);
    }

    pub fn block_at_local(&self, local_x: i32, world_y: i32, local_z: i32) -> BlockId {
        if local_x < 0
            || local_z < 0
            || local_x >= SECTION_SIZE as i32
            || local_z >= SECTION_SIZE as i32
        {
            return BlockId::AIR;
        }
        let Some(section_index) = Self::section_index_from_world_y(world_y) else {
            return BlockId::AIR;
        };
        let Some(section) = self.section(section_index) else {
            return BlockId::AIR;
        };
        let ly = world_y.rem_euclid(SECTION_SIZE as i32) as usize;
        section.block_at(local_x as usize, ly, local_z as usize)
    }
}

/// Chunks are shared, not copied, because meshing runs off the main thread: a mesh job needs a
/// snapshot of a chunk plus its four neighbours, and cloning ~40 KB × 5 per job for 24 jobs a
/// frame is real main-thread memcpy. Nothing mutates a chunk once it is in the world, so the
/// jobs can just hold `Arc`s.
pub struct World {
    chunks: HashMap<(i32, i32), Arc<Chunk>>,
}

impl World {
    pub fn new() -> Self {
        Self {
            chunks: HashMap::new(),
        }
    }

    pub fn chunks(&self) -> impl Iterator<Item = &Chunk> {
        self.chunks.values().map(|chunk| chunk.as_ref())
    }

    pub fn chunk(&self, coord: (i32, i32)) -> Option<&Chunk> {
        self.chunks.get(&coord).map(|chunk| chunk.as_ref())
    }

    /// The chunk as a shared handle, for building a mesh job's snapshot without copying.
    pub fn chunk_shared(&self, coord: (i32, i32)) -> Option<Arc<Chunk>> {
        self.chunks.get(&coord).map(Arc::clone)
    }

    pub fn has_chunk(&self, coord: (i32, i32)) -> bool {
        self.chunks.contains_key(&coord)
    }

    pub fn insert_chunk(&mut self, chunk: Chunk) {
        self.insert_shared(Arc::new(chunk));
    }

    pub fn insert_shared(&mut self, chunk: Arc<Chunk>) {
        self.chunks.insert(chunk.coord(), chunk);
    }

    pub fn remove_chunk(&mut self, coord: (i32, i32)) {
        self.chunks.remove(&coord);
    }

    pub fn block_at(&self, world_x: i32, world_y: i32, world_z: i32) -> BlockId {
        let chunk_x = world_x.div_euclid(SECTION_SIZE as i32);
        let chunk_z = world_z.div_euclid(SECTION_SIZE as i32);
        let local_x = world_x.rem_euclid(SECTION_SIZE as i32);
        let local_z = world_z.rem_euclid(SECTION_SIZE as i32);
        let Some(chunk) = self.chunks.get(&(chunk_x, chunk_z)) else {
            return BlockId::AIR;
        };
        chunk.block_at_local(local_x, world_y, local_z)
    }

    /// The biome index at a world column, or `None` outside loaded chunks / for sources that do
    /// not supply biomes. Horizontal only — see [`Chunk::biomes`].
    pub fn biome_at(&self, world_x: i32, world_z: i32) -> Option<u8> {
        let chunk_x = world_x.div_euclid(SECTION_SIZE as i32);
        let chunk_z = world_z.div_euclid(SECTION_SIZE as i32);
        let local_x = world_x.rem_euclid(SECTION_SIZE as i32);
        let local_z = world_z.rem_euclid(SECTION_SIZE as i32);
        self.chunks
            .get(&(chunk_x, chunk_z))?
            .biome_at_local(local_x, local_z)
    }

    #[allow(dead_code)]
    pub fn generate_procedural_chunk(seed: i64, coord: (i32, i32)) -> Chunk {
        let generator = crate::terrain::TerrainGenerator::new(seed);
        crate::terrain::generate_chunk(&generator, coord)
    }
}
