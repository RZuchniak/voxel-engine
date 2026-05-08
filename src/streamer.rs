use std::{collections::HashSet, sync::Arc};

use crossbeam_channel::{Receiver, Sender, unbounded};
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
    pool: ThreadPool,
    ready_tx: Sender<MeshedChunk>,
    ready_rx: Receiver<MeshedChunk>,
    in_flight: HashSet<(i32, i32)>,
}

impl ChunkStreamer {
    pub fn new(source: Arc<dyn WorldSource>) -> Self {
        let pool = rayon::ThreadPoolBuilder::new()
            .thread_name(|i| format!("chunk-worker-{i}"))
            .build()
            .expect("failed to create rayon pool");
        let (ready_tx, ready_rx) = unbounded();
        Self {
            source,
            pool,
            ready_tx,
            ready_rx,
            in_flight: HashSet::new(),
        }
    }

    pub fn request_chunk(&mut self, coord: (i32, i32)) {
        if !self.in_flight.insert(coord) {
            return;
        }

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

    pub fn poll_ready(&mut self, budget: usize) -> Vec<MeshedChunk> {
        let mut out = Vec::new();
        for _ in 0..budget {
            let Ok(chunk) = self.ready_rx.try_recv() else {
                break;
            };
            self.in_flight.remove(&chunk.coord);
            out.push(chunk);
        }
        out
    }

    pub fn is_in_flight(&self, coord: (i32, i32)) -> bool {
        self.in_flight.contains(&coord)
    }
}
