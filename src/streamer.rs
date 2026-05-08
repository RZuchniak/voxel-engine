use std::{collections::HashSet, sync::Arc};

use crossbeam_channel::{Receiver, Sender, unbounded};
#[cfg(not(target_arch = "wasm32"))]
use rayon::ThreadPool;

use crate::{
    mesh::{MeshData, mesh_section},
    source::WorldSource,
    world::{Chunk, World},
};

pub struct MeshedChunk {
    pub coord: (i32, i32),
    pub chunk: Chunk,
    pub section_meshes: Vec<(usize, MeshData)>,
}

pub struct ChunkStreamer {
    source: Arc<dyn WorldSource>,
    #[cfg(not(target_arch = "wasm32"))]
    pool: ThreadPool,
    ready_tx: Sender<MeshedChunk>,
    ready_rx: Receiver<MeshedChunk>,
    in_flight: HashSet<(i32, i32)>,
    #[cfg(target_arch = "wasm32")]
    pending: Vec<(i32, i32)>,
}

impl ChunkStreamer {
    pub fn new(source: Arc<dyn WorldSource>) -> Self {
        #[cfg(not(target_arch = "wasm32"))]
        let pool = rayon::ThreadPoolBuilder::new()
            .thread_name(|i| format!("chunk-worker-{i}"))
            .build()
            .expect("failed to create rayon pool");
        let (ready_tx, ready_rx) = unbounded();
        Self {
            source,
            #[cfg(not(target_arch = "wasm32"))]
            pool,
            ready_tx,
            ready_rx,
            in_flight: HashSet::new(),
            #[cfg(target_arch = "wasm32")]
            pending: Vec::new(),
        }
    }

    pub fn request_chunk(&mut self, coord: (i32, i32)) {
        if !self.in_flight.insert(coord) {
            return;
        }

        #[cfg(not(target_arch = "wasm32"))]
        {
            let source = Arc::clone(&self.source);
            let ready_tx = self.ready_tx.clone();
            self.pool.spawn(move || {
                let chunk = match source.load_chunk(coord) {
                    Ok(chunk) => chunk,
                    Err(err) => {
                        eprintln!("chunk load failed at {:?}: {err:#}", coord);
                        // Return an empty chunk so the request is marked complete and can be retried.
                        Chunk::new(coord)
                    }
                };
                // Mesh against a world that currently only contains this chunk.
                // Border faces are conservatively generated until neighbor remeshing is added.
                let mut local_world = World::new();
                local_world.insert_chunk(chunk.clone());
                let mut section_meshes = Vec::new();
                for section_index in chunk.populated_section_indices() {
                    if let Some(mesh) = mesh_section(&local_world, coord, section_index) {
                        section_meshes.push((section_index, mesh));
                    }
                }

                let _ = ready_tx.send(MeshedChunk {
                    coord,
                    chunk,
                    section_meshes,
                });
            });
        }

        #[cfg(target_arch = "wasm32")]
        {
            self.pending.push(coord);
        }
    }

    #[cfg(target_arch = "wasm32")]
    fn build_chunk_inline(&self, coord: (i32, i32)) -> MeshedChunk {
        let chunk = match self.source.load_chunk(coord) {
            Ok(chunk) => chunk,
            Err(err) => {
                eprintln!("chunk load failed at {:?}: {err:#}", coord);
                Chunk::new(coord)
            }
        };
        let mut local_world = World::new();
        local_world.insert_chunk(chunk.clone());
        let mut section_meshes = Vec::new();
        for section_index in chunk.populated_section_indices() {
            if let Some(mesh) = mesh_section(&local_world, coord, section_index) {
                section_meshes.push((section_index, mesh));
            }
        }
        MeshedChunk {
            coord,
            chunk,
            section_meshes,
        }
    }

    pub fn poll_ready(&mut self, budget: usize) -> Vec<MeshedChunk> {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let mut out = Vec::new();
            for _ in 0..budget {
                let Ok(chunk) = self.ready_rx.try_recv() else {
                    break;
                };
                self.in_flight.remove(&chunk.coord);
                out.push(chunk);
            }
            return out;
        }

        #[cfg(target_arch = "wasm32")]
        {
            let mut out = Vec::new();
            for _ in 0..budget {
                let Some(coord) = self.pending.pop() else {
                    break;
                };
                let meshed = self.build_chunk_inline(coord);
                self.in_flight.remove(&coord);
                out.push(meshed);
            }
            out
        }
    }

    pub fn is_in_flight(&self, coord: (i32, i32)) -> bool {
        self.in_flight.contains(&coord)
    }
}
