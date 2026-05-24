use std::collections::HashMap;

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

#[derive(Clone)]
pub struct Chunk {
    coord: (i32, i32),
    sections: Vec<Option<Section>>,
}

impl Chunk {
    pub fn new(coord: (i32, i32)) -> Self {
        Self {
            coord,
            sections: (0..SECTION_COUNT).map(|_| None).collect(),
        }
    }

    pub fn coord(&self) -> (i32, i32) {
        self.coord
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

    /// True when any section within [`platform::surface_mesh_depth_blocks`] of the top needs meshing.
    pub fn needs_surface_mesh(&self) -> bool {
        let Some(surface_max_y) = self.max_nonempty_world_y() else {
            return false;
        };
        let depth = crate::platform::surface_mesh_depth_blocks();
        self.populated_section_indices().any(|idx| {
            Self::section_near_surface(idx, surface_max_y, depth)
                && self.section(idx).is_some_and(|s| s.has_any_non_air())
        })
    }

    /// Section is close enough to the chunk surface to worth meshing (skip caves/deep stone).
    pub fn section_near_surface(section_index: usize, surface_max_y: i32, depth_blocks: i32) -> bool {
        let section_base = (section_index as i32 + MIN_SECTION_Y) * SECTION_SIZE as i32;
        let section_top = section_base + SECTION_SIZE as i32;
        section_top >= surface_max_y - depth_blocks
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

pub struct World {
    chunks: HashMap<(i32, i32), Chunk>,
}

impl World {
    pub fn new() -> Self {
        Self {
            chunks: HashMap::new(),
        }
    }

    pub fn chunks(&self) -> impl Iterator<Item = &Chunk> {
        self.chunks.values()
    }

    pub fn chunk(&self, coord: (i32, i32)) -> Option<&Chunk> {
        self.chunks.get(&coord)
    }

    pub fn has_chunk(&self, coord: (i32, i32)) -> bool {
        self.chunks.contains_key(&coord)
    }

    pub fn insert_chunk(&mut self, chunk: Chunk) {
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

    #[allow(dead_code)]
    pub fn generate_procedural_chunk(coord: (i32, i32)) -> Chunk {
        let (chunk_x, chunk_z) = coord;
        let mut chunk = Chunk::new((chunk_x, chunk_z));
        for local_z in 0..SECTION_SIZE {
            for local_x in 0..SECTION_SIZE {
                let world_x = chunk_x * SECTION_SIZE as i32 + local_x as i32;
                let world_z = chunk_z * SECTION_SIZE as i32 + local_z as i32;
                let noise =
                    ((world_x as f32 * 0.11).sin() * 6.0 + (world_z as f32 * 0.09).cos() * 6.0)
                        as i32;
                let top_y = (70 + noise).clamp(50, 96);

                for y in -64..=top_y {
                    let block = if y == top_y {
                        BlockId::GRASS
                    } else if y >= top_y - 3 {
                        BlockId::DIRT
                    } else {
                        BlockId::STONE
                    };
                    chunk.set_block_world(local_x, y, local_z, block);
                }
            }
        }
        chunk
    }
}
