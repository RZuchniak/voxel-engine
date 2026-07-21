use serde::{Deserialize, Serialize};

use crate::{
    block::BlockId,
    mesh::MeshData,
    world::{Chunk, Section},
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum WorkerJob {
    LoadChunk { id: u32, cx: i32, cz: i32 },
    RemeshChunk { id: u32, cx: i32, cz: i32 },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum WorkerReply {
    LoadChunk {
        id: u32,
        chunk: ChunkWire,
        #[serde(default)]
        section_meshes: Vec<SectionMeshWire>,
    },
    RemeshChunk {
        id: u32,
        cx: i32,
        cz: i32,
        section_meshes: Vec<SectionMeshWire>,
    },
    Error { id: u32, message: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChunkWire {
    pub cx: i32,
    pub cz: i32,
    pub sections: Vec<(usize, Vec<u16>)>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SectionMeshWire {
    pub section_index: usize,
    pub vertices: Vec<u8>,
    pub indices: Vec<u8>,
}

impl ChunkWire {
    pub fn from_chunk(chunk: &Chunk) -> Self {
        let (cx, cz) = chunk.coord();
        let sections = chunk
            .populated_section_indices()
            .map(|section_index| {
                let blocks = chunk
                    .section(section_index)
                    .expect("populated section")
                    .block_data()
                    .iter()
                    .map(|block| block.0)
                    .collect();
                (section_index, blocks)
            })
            .collect();
        Self { cx, cz, sections }
    }

    pub fn into_chunk(self) -> Chunk {
        let mut chunk = Chunk::new((self.cx, self.cz));
        for (section_index, blocks) in self.sections {
            let mut section = Section::new();
            for (idx, block_id) in blocks.into_iter().enumerate() {
                // `from_chunk` dumps `Section::block_data()` verbatim, and Section indexes
                // as `x + y*16 + z*256`. Decoding with Minecraft's NBT order
                // (`x + z*16 + y*256`) instead transposed Y and Z, which tilted every
                // worker-delivered chunk on its side.
                let x = idx & 0xF;
                let y = (idx >> 4) & 0xF;
                let z = idx >> 8;
                section.set_block(x, y, z, BlockId(block_id));
            }
            chunk.insert_section(section_index, section);
        }
        chunk
    }
}

impl SectionMeshWire {
    pub fn from_mesh(section_index: usize, mesh: &MeshData) -> Self {
        let (vertices, indices) = mesh.to_wire_bytes();
        Self {
            section_index,
            vertices,
            indices,
        }
    }

    pub fn into_mesh(self) -> Option<(usize, MeshData)> {
        let mesh = MeshData::from_wire_bytes(self.vertices, self.indices)?;
        Some((self.section_index, mesh))
    }
}

pub fn encode_job(job: &WorkerJob) -> Vec<u8> {
    bincode::serialize(job).expect("worker job encode")
}

pub fn decode_job(bytes: &[u8]) -> Result<WorkerJob, String> {
    bincode::deserialize(bytes).map_err(|err| err.to_string())
}

pub fn encode_reply(reply: &WorkerReply) -> Vec<u8> {
    bincode::serialize(reply).expect("worker reply encode")
}

pub fn decode_reply(bytes: &[u8]) -> Result<WorkerReply, String> {
    bincode::deserialize(bytes).map_err(|err| err.to_string())
}
