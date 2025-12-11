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

pub struct Chunk {
    data: [BlockType; 16 * 16 * 16],
    position: (i32, i32, i32),
}

impl Chunk {
    fn new(position: (i32, i32, i32)) -> Self {
        Self {
            data: [BlockType::AIR; 16 * 16 * 16],
            position,
        }
    }
}

pub fn generate_mesh(chunk: &Chunk) -> MeshData {
    let mut mesh_data = MeshData::new();

    for x in 0..16 {
        for y in 0..16 {
            for z in 0..16 {
                let block = chunk.data[x as usize + y as usize * 16 + z as usize * 16 * 16];
                if block != BlockType::AIR {
                    if z != 0 {
                        if chunk.data[x as usize + y as usize * 16 + (z - 1) as usize * 16 * 16]
                            != BlockType::AIR
                        {
                            continue;
                        }
                    }
                }
            }
        }
    }
    for y in 0..16 {}
    for z in 0..16 {}

    mesh_data
}
