use std::collections::{HashMap, VecDeque};
use std::sync::{Mutex, OnceLock};

use crate::{
    mesh::mesh_chunk_surface,
    source::{MemoryAnvilSource, SeededProceduralSource, WorldSource},
    worker_protocol::{WorkerJob, WorkerReply, ChunkWire, SectionMeshWire},
    world::{Chunk, World},
};

/// Chunks retained per worker so meshing a chunk does not re-generate its neighbours.
///
/// Meshing needs the target plus its 4 orthogonal neighbours, and adjacent chunks share
/// neighbours, so without a cache each chunk would cost 5 generations instead of ~1.
/// A chunk is a few tens of KB, so this is single-digit MB per worker.
const CHUNK_CACHE_CAPACITY: usize = 64;

struct Engine {
    source: Box<dyn WorldSource>,
    cache: HashMap<(i32, i32), Chunk>,
    /// Insertion order for FIFO eviction.
    order: VecDeque<(i32, i32)>,
}

impl Engine {
    /// Cached chunk, generating it on first use. Returns a clone because the caller
    /// assembles a local `World`; the cache keeps the original for neighbouring chunks.
    fn chunk(&mut self, coord: (i32, i32)) -> Chunk {
        if !self.cache.contains_key(&coord) {
            let chunk = self
                .source
                .load_chunk(coord)
                .unwrap_or_else(|_| Chunk::new(coord));
            self.cache.insert(coord, chunk);
            self.order.push_back(coord);
            while self.order.len() > CHUNK_CACHE_CAPACITY {
                if let Some(evicted) = self.order.pop_front() {
                    self.cache.remove(&evicted);
                }
            }
        }
        self.cache
            .get(&coord)
            .cloned()
            .unwrap_or_else(|| Chunk::new(coord))
    }

    /// The target chunk plus its 4 orthogonal neighbours, which is what the mesher needs
    /// to cull faces correctly at chunk borders.
    fn neighbourhood(&mut self, coord: (i32, i32)) -> World {
        let mut world = World::new();
        for (dx, dz) in [(0, 0), (1, 0), (-1, 0), (0, 1), (0, -1)] {
            let c = (coord.0 + dx, coord.1 + dz);
            world.insert_chunk(self.chunk(c));
        }
        world
    }
}

static ENGINE: OnceLock<Mutex<Option<Engine>>> = OnceLock::new();

fn engine_slot() -> &'static Mutex<Option<Engine>> {
    ENGINE.get_or_init(|| Mutex::new(None))
}

fn install(source: Box<dyn WorldSource>) -> Result<(), String> {
    if let Ok(mut slot) = engine_slot().lock() {
        *slot = Some(Engine {
            source,
            cache: HashMap::new(),
            order: VecDeque::new(),
        });
        Ok(())
    } else {
        Err("worker engine mutex poisoned".to_string())
    }
}

pub fn init_from_zip(bytes: &[u8]) -> Result<(), String> {
    let source = MemoryAnvilSource::from_zip(bytes).map_err(|err| err.to_string())?;
    install(Box::new(source))
}

/// Seed worlds ship a seed rather than chunk data — generation is deterministic, so
/// every worker reproduces the same terrain independently.
pub fn init_from_seed(seed: i64) -> Result<(), String> {
    install(Box::new(SeededProceduralSource::new(seed)))
}

fn with_engine<R>(f: impl FnOnce(&mut Engine) -> R) -> Result<R, String> {
    let mut slot = engine_slot()
        .lock()
        .map_err(|_| "worker engine mutex poisoned".to_string())?;
    let Some(engine) = slot.as_mut() else {
        return Err("worker engine not initialized".to_string());
    };
    Ok(f(engine))
}

fn mesh_to_wire(world: &World, coord: (i32, i32)) -> Vec<SectionMeshWire> {
    mesh_chunk_surface(world, coord)
        .into_iter()
        .map(|(section_index, mesh)| SectionMeshWire::from_mesh(section_index, &mesh))
        .collect()
}

/// Generate a chunk *and* mesh it, so the main thread only has to upload buffers.
///
/// Meshing used to run on the main thread, where it became the bottleneck once workers
/// made generation cheap — the pending-upload queue saturated at its cap and completed
/// chunks were being discarded.
fn load_and_mesh(cx: i32, cz: i32) -> Result<(ChunkWire, Vec<SectionMeshWire>), String> {
    with_engine(|engine| {
        let coord = (cx, cz);
        let world = engine.neighbourhood(coord);
        let wire = world
            .chunk(coord)
            .map(ChunkWire::from_chunk)
            .unwrap_or(ChunkWire {
                cx,
                cz,
                sections: Vec::new(),
            });
        let section_meshes = mesh_to_wire(&world, coord);
        (wire, section_meshes)
    })
}

fn remesh_chunk(cx: i32, cz: i32) -> Result<Vec<SectionMeshWire>, String> {
    with_engine(|engine| {
        let coord = (cx, cz);
        let world = engine.neighbourhood(coord);
        mesh_to_wire(&world, coord)
    })
}

pub fn handle_job(job: WorkerJob) -> WorkerReply {
    match job {
        WorkerJob::LoadChunk { id, cx, cz } => match load_and_mesh(cx, cz) {
            Ok((chunk, section_meshes)) => WorkerReply::LoadChunk {
                id,
                chunk,
                section_meshes,
            },
            Err(message) => WorkerReply::Error { id, message },
        },
        WorkerJob::RemeshChunk { id, cx, cz } => match remesh_chunk(cx, cz) {
            Ok(section_meshes) => WorkerReply::RemeshChunk {
                id,
                cx,
                cz,
                section_meshes,
            },
            Err(message) => WorkerReply::Error { id, message },
        },
    }
}
