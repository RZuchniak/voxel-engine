use std::sync::{Mutex, OnceLock};

use crate::{
    mesh::mesh_chunk_surface,
    source::{MemoryAnvilSource, SeededProceduralSource, WorldSource},
    worker_protocol::{WorkerJob, WorkerReply, ChunkWire, SectionMeshWire},
    world::World,
};

struct Engine {
    source: Box<dyn WorldSource>,
}

static ENGINE: OnceLock<Mutex<Option<Engine>>> = OnceLock::new();

fn engine_slot() -> &'static Mutex<Option<Engine>> {
    ENGINE.get_or_init(|| Mutex::new(None))
}

fn install(source: Box<dyn WorldSource>) -> Result<(), String> {
    if let Ok(mut slot) = engine_slot().lock() {
        *slot = Some(Engine { source });
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

fn with_source<R>(f: impl FnOnce(&dyn WorldSource) -> R) -> Result<R, String> {
    let slot = engine_slot().lock().map_err(|_| "worker engine mutex poisoned".to_string())?;
    let Some(engine) = slot.as_ref() else {
        return Err("worker engine not initialized".to_string());
    };
    Ok(f(engine.source.as_ref()))
}

fn load_chunk(cx: i32, cz: i32) -> Result<ChunkWire, String> {
    with_source(|source| {
        let coord = (cx, cz);
        let chunk = match source.load_chunk(coord) {
            Ok(chunk) => chunk,
            Err(err) => {
                eprintln!("worker chunk load failed at ({cx},{cz}): {err:#}");
                return ChunkWire {
                    cx,
                    cz,
                    sections: Vec::new(),
                };
            }
        };
        ChunkWire::from_chunk(&chunk)
    })
}

fn remesh_chunk(cx: i32, cz: i32) -> Result<Vec<SectionMeshWire>, String> {
    with_source(|source| {
        let coord = (cx, cz);
        let mut world = World::new();
        for (dx, dz) in [(0, 0), (1, 0), (-1, 0), (0, 1), (0, -1)] {
            let neighbor = (cx + dx, cz + dz);
            if let Ok(chunk) = source.load_chunk(neighbor) {
                world.insert_chunk(chunk);
            }
        }

        let section_meshes = mesh_chunk_surface(&world, coord)
            .into_iter()
            .map(|(section_index, mesh)| SectionMeshWire::from_mesh(section_index, &mesh))
            .collect();
        section_meshes
    })
}

pub fn handle_job(job: WorkerJob) -> WorkerReply {
    match job {
        WorkerJob::LoadChunk { id, cx, cz } => match load_chunk(cx, cz) {
            Ok(chunk) => WorkerReply::LoadChunk {
                id,
                chunk,
                section_meshes: Vec::new(),
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
