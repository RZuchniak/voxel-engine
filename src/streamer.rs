use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

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
    pub chunk: Option<Chunk>,
    pub section_meshes: Vec<(usize, MeshData)>,
    pub is_remesh: bool,
}

pub struct ChunkStreamer {
    source: Arc<dyn WorldSource>,
    #[cfg(not(target_arch = "wasm32"))]
    pool: ThreadPool,
    ready_tx: Sender<MeshedChunk>,
    ready_rx: Receiver<MeshedChunk>,
    in_flight: HashSet<(i32, i32)>,
    remesh_in_flight: HashSet<(i32, i32)>,
    /// Latest world snapshot for a chunk when a remesh was requested while one was already running.
    pending_remesh_worlds: HashMap<(i32, i32), World>,
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
            remesh_in_flight: HashSet::new(),
            pending_remesh_worlds: HashMap::new(),
            #[cfg(target_arch = "wasm32")]
            pending: Vec::new(),
        }
    }

    fn dispatch_remesh_work(&mut self, coord: (i32, i32), world: World) {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let ready_tx = self.ready_tx.clone();
            self.pool.spawn(move || {
                let mut section_meshes = Vec::new();
                if let Some(chunk) = world.chunk(coord) {
                    for section_index in chunk.populated_section_indices() {
                        if let Some(mesh) = mesh_section(&world, coord, section_index) {
                            section_meshes.push((section_index, mesh));
                        }
                    }
                }

                let _ = ready_tx.send(MeshedChunk {
                    coord,
                    chunk: None,
                    section_meshes,
                    is_remesh: true,
                });
            });
        }

        #[cfg(target_arch = "wasm32")]
        {
            let mut section_meshes = Vec::new();
            if let Some(chunk) = world.chunk(coord) {
                for section_index in chunk.populated_section_indices() {
                    if let Some(mesh) = mesh_section(&world, coord, section_index) {
                        section_meshes.push((section_index, mesh));
                    }
                }
            }
            let _ = self.ready_tx.send(MeshedChunk {
                coord,
                chunk: None,
                section_meshes,
                is_remesh: true,
            });
        }
    }

    fn on_remesh_result_delivered(&mut self, coord: (i32, i32)) {
        self.remesh_in_flight.remove(&coord);
        if let Some(next_world) = self.pending_remesh_worlds.remove(&coord) {
            self.remesh_in_flight.insert(coord);
            self.dispatch_remesh_work(coord, next_world);
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
                // No mesh here: single-chunk meshing draws the full chunk shell (wrong borders).
                // Main thread inserts the chunk then runs `request_remesh` with neighbor context.

                let _ = ready_tx.send(MeshedChunk {
                    coord,
                    chunk: Some(chunk),
                    section_meshes: Vec::new(),
                    is_remesh: false,
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
        MeshedChunk {
            coord,
            chunk: Some(chunk),
            section_meshes: Vec::new(),
            is_remesh: false,
        }
    }

    pub fn request_remesh(&mut self, coord: (i32, i32), world: World) {
        if self.remesh_in_flight.contains(&coord) {
            // Keep the newest snapshot so border culling updates when neighbors finish loading
            // while an older remesh is still running.
            self.pending_remesh_worlds.insert(coord, world);
            return;
        }
        self.remesh_in_flight.insert(coord);
        self.dispatch_remesh_work(coord, world);
    }

    pub fn poll_ready(&mut self, budget: usize) -> Vec<MeshedChunk> {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let mut out = Vec::new();
            for _ in 0..budget {
                let Ok(chunk) = self.ready_rx.try_recv() else {
                    break;
                };
                if chunk.is_remesh {
                    let coord = chunk.coord;
                    self.on_remesh_result_delivered(coord);
                } else {
                    self.in_flight.remove(&chunk.coord);
                }
                out.push(chunk);
            }
            return out;
        }

        #[cfg(target_arch = "wasm32")]
        {
            let mut out = Vec::new();
            for _ in 0..budget {
                if let Ok(chunk) = self.ready_rx.try_recv() {
                    if chunk.is_remesh {
                        let coord = chunk.coord;
                        self.on_remesh_result_delivered(coord);
                    } else {
                        self.in_flight.remove(&chunk.coord);
                    }
                    out.push(chunk);
                } else if let Some(coord) = self.pending.pop() {
                    let meshed = self.build_chunk_inline(coord);
                    self.in_flight.remove(&coord);
                    out.push(meshed);
                } else {
                    break;
                }
            }
            out
        }
    }

    pub fn is_in_flight(&self, coord: (i32, i32)) -> bool {
        self.in_flight.contains(&coord)
    }

    pub fn is_remesh_in_flight(&self, coord: (i32, i32)) -> bool {
        self.remesh_in_flight.contains(&coord)
    }

    /// Drop coalesced remesh snapshots for a chunk that left the simulation (safe if a worker is still running).
    pub fn clear_pending_remesh_snapshot(&mut self, coord: (i32, i32)) {
        self.pending_remesh_worlds.remove(&coord);
    }
}
