use crate::{BlockType, Vertex};

pub struct MeshData {
    vertices: Vec<Vertex>,
    indices: Vec<u32>,
}

impl MeshData {
    pub fn new() -> Self {
        Self {
            vertices: Vec::new(),
            indices: Vec::new(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.vertices.is_empty()
    }
}

enum Direction {
    XPositive,
    XNegative,
    YPositive,
    YNegative,
    ZPositive,
    ZNegative,
}

pub struct Chunk {
    data: [BlockType; 16 * 16 * 16],
    position: (i32, i32, i32),
    mesh: Option<MeshData>,
}

impl Chunk {
    pub fn new(position: (i32, i32, i32)) -> Self {
        Self {
            data: [BlockType::AIR; 16 * 16 * 16],
            position,
            mesh: None,
        }
    }
    pub fn generate_mesh(&mut self) {
        let mut mesh_data = MeshData::new();
    }

    #[inline]
    fn get_block(&self, x: usize, y: usize, z: usize) -> BlockType {
        if x >= 16 || y >= 16 || z >= 16 {
            BlockType::AIR
        } else {
            self.data[x + y * 16 + z * 16 * 16]
        }
    }

    fn get_block_color(block_type: BlockType) -> [f32; 4] {
        match block_type {
            BlockType::AIR => [0.0, 0.0, 0.0, 0.0],
            BlockType::STONE => [0.5, 0.5, 0.5, 1.0],
            BlockType::GRASS => [0.0, 1.0, 0.0, 1.0],
        }
    }

    fn mesh_direction(&mut self, dir: Direction) {
        let mut merged = [[false; 16]; 16];

        let (primary_axis, secondary_axis, tertiary_axis, direction_offset, is_positive) = match dir
        {
            Direction::XPositive => (0, 1, 2, (1, 0, 0), true),
            Direction::XNegative => (0, 1, 2, (-1, 0, 0), false),
            Direction::YPositive => (1, 2, 0, (0, 1, 0), true),
            Direction::YNegative => (1, 2, 0, (0, -1, 0), false),
            Direction::ZPositive => (2, 0, 1, (0, 0, 1), true),
            Direction::ZNegative => (2, 0, 1, (0, 0, -1), false),
        };
        
        for primary in 0usize..16 {
            for secondary in 0usize..16 {
                for tertiary in 0usize..16 {
                    if merged[secondary][tertiary] {
                    	continue;
                    }
                    
                    let (cx, cy, cz)
                }
            }
        }
    }
}
