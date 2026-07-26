use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use crossbeam_channel::{Receiver, Sender, unbounded};
#[cfg(not(target_arch = "wasm32"))]
use rayon::ThreadPool;

use crate::{
    mesh::{mesh_chunk, MeshData},
    source::WorldSource,
    visibility::{chunk_visibility, VisibilitySet},
    world::{Chunk, World, SECTION_COUNT},
};

#[cfg(target_arch = "wasm32")]
use crate::worker_bridge::{take_failures, WorkerBridge, WorkerSource};

pub struct MeshedChunk {
    pub coord: (i32, i32),
    pub chunk: Option<Chunk>,
    pub section_meshes: Vec<(usize, MeshData)>,
    pub is_remesh: bool,
    /// Section connectivity for the renderer's occlusion traversal, computed alongside the
    /// mesh because both walk the same block arrays. `None` on results that carry no mesh.
    pub visibility: Option<[VisibilitySet; SECTION_COUNT]>,
}

pub struct ChunkStreamer {
    source: Arc<dyn WorldSource>,
    #[cfg(not(target_arch = "wasm32"))]
    pool: ThreadPool,
    /// Meshing gets its own threads because the two kinds of work have opposite shapes.
    /// Generation is bulk throughput — the streamer queues the whole load radius at once, so
    /// thousands of ~21.6 ms jobs can be outstanding. Meshing is latency-critical: a chunk is
    /// invisible until its mesh lands. Sharing one pool puts every mesh job behind that
    /// backlog, and new chunks stop appearing (measured: 5 chunks/s meshed against 330/s
    /// generated).
    #[cfg(not(target_arch = "wasm32"))]
    mesh_pool: ThreadPool,
    ready_tx: Sender<MeshedChunk>,
    ready_rx: Receiver<MeshedChunk>,
    in_flight: HashSet<(i32, i32)>,
    remesh_in_flight: HashSet<(i32, i32)>,
    /// Remesh requests that arrived while one was already running for the same coord, each
    /// holding the snapshot it was requested with. Keeping the snapshot matters: re-meshing a
    /// coord against a world that does not contain it yields an empty mesh, which uploads as
    /// "no geometry" and erases the chunk from the render.
    pending_remesh: HashMap<(i32, i32), World>,
    procedural: bool,
    #[cfg(target_arch = "wasm32")]
    worker_bridge: Option<WorkerBridge>,
}

impl ChunkStreamer {
    pub fn new(source: Arc<dyn WorldSource>) -> Self {
        let procedural = source.is_procedural();
        #[cfg(not(target_arch = "wasm32"))]
        let pool = rayon::ThreadPoolBuilder::new()
            .thread_name(|i| format!("chunk-worker-{i}"))
            .build()
            .expect("failed to create rayon pool");
        // Deliberately oversubscribed against `pool`: these threads are idle most of the time
        // and only need to keep up with the generation rate, so it is worth letting them
        // preempt generation rather than wait behind it.
        #[cfg(not(target_arch = "wasm32"))]
        let mesh_pool = rayon::ThreadPoolBuilder::new()
            .num_threads(crate::platform::mesh_worker_threads())
            .thread_name(|i| format!("mesh-worker-{i}"))
            .build()
            .expect("failed to create rayon mesh pool");
        let (ready_tx, ready_rx) = unbounded();
        Self {
            source,
            #[cfg(not(target_arch = "wasm32"))]
            pool,
            #[cfg(not(target_arch = "wasm32"))]
            mesh_pool,
            ready_tx,
            ready_rx,
            in_flight: HashSet::new(),
            remesh_in_flight: HashSet::new(),
            pending_remesh: HashMap::new(),
            procedural,
            #[cfg(target_arch = "wasm32")]
            worker_bridge: None,
        }
    }

    #[cfg(target_arch = "wasm32")]
    pub fn enable_workers(&mut self, source: WorkerSource<'_>) -> Result<(), String> {
        if !crate::platform::use_wasm_chunk_workers() || crate::platform::wasm_worker_count() == 0 {
            return Ok(());
        }
        let bridge = WorkerBridge::start(source, crate::platform::wasm_worker_count())?;
        self.worker_bridge = Some(bridge);
        Ok(())
    }

    #[cfg(target_arch = "wasm32")]
    fn spawn_main_thread_load(&self, coord: (i32, i32)) {
        let source = Arc::clone(&self.source);
        let ready_tx = self.ready_tx.clone();
        wasm_bindgen_futures::spawn_local(async move {
            let chunk = match source.load_chunk(coord) {
                Ok(chunk) => chunk,
                Err(err) => {
                    web_sys::console::error_1(&wasm_bindgen::JsValue::from_str(&format!(
                        "chunk load failed at ({},{}): {err:#}",
                        coord.0, coord.1
                    )));
                    Chunk::new(coord)
                }
            };
            let _ = ready_tx.send(MeshedChunk {
                coord,
                chunk: Some(chunk),
                section_meshes: Vec::new(),
                is_remesh: false,
                visibility: None,
            });
            crate::web_api::kick_event_loop();
        });
    }

    /// Generate on the main thread. Only used when no worker bridge is available.
    #[cfg(target_arch = "wasm32")]
    fn send_loaded_chunk_sync(&self, coord: (i32, i32)) {
        let chunk = match self.source.load_chunk(coord) {
            Ok(chunk) => chunk,
            Err(err) => {
                web_sys::console::error_1(&wasm_bindgen::JsValue::from_str(&format!(
                    "chunk load failed at ({},{}): {err:#}",
                    coord.0, coord.1
                )));
                Chunk::new(coord)
            }
        };
        self.send_loaded_chunk(coord, chunk);
    }

    #[cfg(target_arch = "wasm32")]
    fn send_loaded_chunk(&self, coord: (i32, i32), chunk: Chunk) {
        let _ = self.ready_tx.send(MeshedChunk {
            coord,
            chunk: Some(chunk),
            section_meshes: Vec::new(),
            is_remesh: false,
            visibility: None,
        });
    }

    fn dispatch_remesh_work(&mut self, coord: (i32, i32), world: World) {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let ready_tx = self.ready_tx.clone();
            self.mesh_pool.spawn(move || {
                let section_meshes = mesh_chunk(&world, coord);
                let visibility = world.chunk(coord).map(chunk_visibility);

                let _ = ready_tx.send(MeshedChunk {
                    coord,
                    chunk: None,
                    section_meshes,
                    is_remesh: true,
                    visibility,
                });
            });
        }

        #[cfg(target_arch = "wasm32")]
        {
            // Remesh uses the main-thread world snapshot; workers only load chunk data.
            let section_meshes = mesh_chunk(&world, coord);
            let visibility = world.chunk(coord).map(chunk_visibility);
            let _ = self.ready_tx.send(MeshedChunk {
                coord,
                chunk: None,
                section_meshes,
                is_remesh: true,
                visibility,
            });
        }
    }

    fn on_remesh_result_delivered(&mut self, coord: (i32, i32)) {
        self.remesh_in_flight.remove(&coord);
        if let Some(world) = self.pending_remesh.remove(&coord) {
            self.remesh_in_flight.insert(coord);
            self.dispatch_remesh_work(coord, world);
        }
    }

    #[cfg(target_arch = "wasm32")]
    fn apply_worker_failures(&mut self) {
        for failure in take_failures() {
            if failure.is_remesh {
                self.remesh_in_flight.remove(&failure.coord);
            } else if self.worker_bridge.is_some() {
                self.in_flight.insert(failure.coord);
                self.spawn_main_thread_load(failure.coord);
            } else {
                self.in_flight.remove(&failure.coord);
            }
        }
    }

    pub fn request_chunk(&mut self, coord: (i32, i32), bootstrap: bool) -> bool {
        if self.is_in_flight(coord) {
            return false;
        }

        #[cfg(not(target_arch = "wasm32"))]
        {
            let _ = bootstrap;
            self.in_flight.insert(coord);
            let source = Arc::clone(&self.source);
            let ready_tx = self.ready_tx.clone();
            self.pool.spawn(move || {
                let chunk = match source.load_chunk(coord) {
                    Ok(chunk) => chunk,
                    Err(err) => {
                        eprintln!("chunk load failed at {:?}: {err:#}", coord);
                        Chunk::new(coord)
                    }
                };

                let _ = ready_tx.send(MeshedChunk {
                    coord,
                    chunk: Some(chunk),
                    section_meshes: Vec::new(),
                    is_remesh: false,
                    visibility: None,
                });
            });
            return true;
        }

        #[cfg(target_arch = "wasm32")]
        {
            // Seed worlds use workers from the first frame: each worker holds the same
            // generator, so there is nothing to wait for. Zip bootstrap stays on the main
            // thread because the workers are still parsing the archive.
            if self.procedural || !bootstrap {
                if let Some(bridge) = self.worker_bridge.as_mut() {
                    if bridge.try_request_load(coord) {
                        return true;
                    }
                    // Fall through to the synchronous path below. An earlier version bailed
                    // out here so bootstrap would wait for the workers, but if the workers
                    // never become ready that loads nothing at all and the screen stays
                    // blank. Degrading to main-thread generation keeps this change strictly
                    // non-regressive: worst case is today's behaviour.
                }
            }

            self.in_flight.insert(coord);
            if self.procedural {
                self.send_loaded_chunk_sync(coord);
            } else {
                self.spawn_main_thread_load(coord);
            }
            true
        }
    }

    pub fn request_remesh(&mut self, coord: (i32, i32), world: World) {
        if self.is_remesh_in_flight(coord) {
            self.pending_remesh.insert(coord, world);
            return;
        }
        #[cfg(target_arch = "wasm32")]
        if self
            .worker_bridge
            .as_ref()
            .is_some_and(|bridge| bridge.is_load_busy(coord))
        {
            self.pending_remesh.insert(coord, world);
            return;
        }
        self.remesh_in_flight.insert(coord);
        self.dispatch_remesh_work(coord, world);
    }

    pub fn poll_ready(&mut self, budget: usize) -> Vec<MeshedChunk> {
        self.poll_ready_limited(budget, budget, budget)
    }

    pub fn poll_ready_limited(
        &mut self,
        budget: usize,
        _chunk_load_limit: usize,
        _remesh_section_limit: usize,
    ) -> Vec<MeshedChunk> {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let _ = (_chunk_load_limit, _remesh_section_limit);
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
            self.apply_worker_failures();
            let mut out = Vec::new();
            if let Some(bridge) = self.worker_bridge.as_mut() {
                bridge.poll_into(&mut out, budget);
            }
            while out.len() < budget {
                let Ok(chunk) = self.ready_rx.try_recv() else {
                    break;
                };
                out.push(chunk);
            }
            for chunk in &out {
                if chunk.is_remesh {
                    let coord = chunk.coord;
                    self.on_remesh_result_delivered(coord);
                } else {
                    self.in_flight.remove(&chunk.coord);
                }
            }
            self.apply_worker_failures();
            out
        }
    }

    pub fn is_procedural(&self) -> bool {
        self.procedural
    }

    pub fn is_in_flight(&self, coord: (i32, i32)) -> bool {
        if self.in_flight.contains(&coord) {
            return true;
        }
        #[cfg(target_arch = "wasm32")]
        if self
            .worker_bridge
            .as_ref()
            .is_some_and(|bridge| bridge.is_load_busy(coord))
        {
            return true;
        }
        false
    }

    pub fn is_remesh_in_flight(&self, coord: (i32, i32)) -> bool {
        self.remesh_in_flight.contains(&coord)
    }

    pub fn clear_pending_remesh_snapshot(&mut self, coord: (i32, i32)) {
        self.pending_remesh.remove(&coord);
    }

    pub fn is_worker_load_busy(&self, coord: (i32, i32)) -> bool {
        #[cfg(target_arch = "wasm32")]
        {
            return self
                .worker_bridge
                .as_ref()
                .is_some_and(|bridge| bridge.is_load_busy(coord));
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let _ = coord;
            false
        }
    }

    pub fn release_stale_worker_load(&mut self, coord: (i32, i32)) {
        #[cfg(target_arch = "wasm32")]
        if let Some(bridge) = self.worker_bridge.as_mut() {
            bridge.release_load_coord(coord);
        }
        #[cfg(not(target_arch = "wasm32"))]
        let _ = coord;
    }

    pub fn release_stale_load(&mut self, coord: (i32, i32)) {
        self.in_flight.remove(&coord);
        self.release_stale_worker_load(coord);
    }

    #[cfg(target_arch = "wasm32")]
    pub fn worker_status(&self) -> Option<(usize, usize, usize, usize)> {
        let bridge = self.worker_bridge.as_ref()?;
        Some((
            bridge.workers_ready_count(),
            bridge.worker_count(),
            bridge.queued_job_count(),
            bridge.in_flight_job_count(),
        ))
    }
}
