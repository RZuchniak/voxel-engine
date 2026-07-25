/// Runtime tuning — tighter on wasm for browser memory and single-threaded meshing.

#[inline]
pub const fn load_distance_chunks() -> i32 {
    if cfg!(target_arch = "wasm32") {
        16
    } else {
        40
    }
}

#[inline]
pub const fn default_section_draw_distance_chunks() -> f32 {
    if cfg!(target_arch = "wasm32") {
        16.0
    } else {
        40.0
    }
}

#[inline]
pub const fn max_section_draw_distance_chunks() -> f32 {
    if cfg!(target_arch = "wasm32") {
        28.0
    } else {
        64.0
    }
}

/// How much world is generated *and meshed* before the player is let in.
///
/// This is the loading screen's size. Everything inside it is finished terrain when the
/// world is revealed, so raising it trades a longer wait for a cleaner arrival — fewer
/// frame-time dips and less pop-in while the rest streams in behind you.
///
/// Native default is 12 (625 chunks, ~17 s with the Minecraft-parity generator, after which
/// the world holds 98–132 FPS immediately). Override with `VOXEL_LOADING_RADIUS=<chunks>` —
/// 16 buys a wider finished area for ~46 s; 0 disables the wait entirely.
///
/// The wait is dominated by **main-thread meshing** (~4.6 ms/chunk), not by generation or
/// rendering — see the load-profile notes in HANDOFF.md before trying to tune it.
#[inline]
pub fn bootstrap_chunk_radius() -> i32 {
    #[cfg(target_arch = "wasm32")]
    {
        3
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        use std::sync::OnceLock;
        static RADIUS: OnceLock<i32> = OnceLock::new();
        *RADIUS.get_or_init(|| {
            std::env::var("VOXEL_LOADING_RADIUS")
                .ok()
                .and_then(|v| v.trim().parse::<i32>().ok())
                .map(|v| v.clamp(0, load_distance_chunks()))
                .unwrap_or(12)
        })
    }
}

#[inline]
pub const fn max_new_requests_per_frame() -> usize {
    if cfg!(target_arch = "wasm32") {
        4
    } else {
        6
    }
}

/// Procedural generation is cheap — request more chunks per frame on wasm.
#[inline]
pub const fn procedural_max_new_requests_per_frame() -> usize {
    if cfg!(target_arch = "wasm32") {
        10
    } else {
        12
    }
}

#[inline]
pub const fn max_chunk_uploads_per_frame() -> usize {
    if cfg!(target_arch = "wasm32") {
        6
    } else {
        4
    }
}

#[inline]
pub const fn procedural_max_chunk_uploads_per_frame() -> usize {
    if cfg!(target_arch = "wasm32") {
        12
    } else {
        8
    }
}

#[inline]
pub const fn max_upload_time_budget_ms() -> u128 {
    if cfg!(target_arch = "wasm32") {
        16
    } else {
        1
    }
}

/// Cap synchronous main-thread meshing per frame (wasm remesh is inline).
#[inline]
pub const fn remesh_time_budget_ms() -> u128 {
    if cfg!(target_arch = "wasm32") {
        8
    } else {
        2
    }
}

#[inline]
pub const fn procedural_remesh_time_budget_ms() -> u128 {
    if cfg!(target_arch = "wasm32") {
        12
    } else {
        4
    }
}

#[inline]
pub const fn bootstrap_max_new_requests_per_frame() -> usize {
    if cfg!(target_arch = "wasm32") {
        24
    } else {
        32
    }
}

#[inline]
pub const fn bootstrap_ready_drain_budget_per_frame() -> usize {
    if cfg!(target_arch = "wasm32") {
        64
    } else {
        256
    }
}

/// Minecraft's cave culling: only draw sections a sightline actually reaches.
///
/// **Off by default until generation stops banding.** The graph works — it cuts drawn sections
/// by 37–45% — but it cannot pay for itself yet. `generate_chunk_surface` leaves everything
/// below the surface with no block data, and a section with no data has to be treated as
/// transparent, so the walk explores ~21k sections of nothing every frame and costs more time
/// than the skipped draws save (274 → 227 FPS at loading radius 12).
///
/// Once chunks are generated full-depth the ground below is opaque rock, the walk terminates at
/// the surface, and both halves of that trade reverse. Flip this default then.
///
/// `VOXEL_SECTION_CULLING=1` enables it for measurement. A bug here shows up as terrain
/// popping out of existence, so compare `reachable_sections` and `visible_draws` in the profile
/// line rather than trying to catch it by eye.
#[inline]
pub fn section_occlusion_culling() -> bool {
    #[cfg(target_arch = "wasm32")]
    {
        false
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        use std::sync::OnceLock;
        static ENABLED: OnceLock<bool> = OnceLock::new();
        *ENABLED.get_or_init(|| {
            std::env::var("VOXEL_SECTION_CULLING")
                .map(|v| v.trim() == "1")
                .unwrap_or(false)
        })
    }
}

/// Threads dedicated to meshing, separate from the generator pool.
///
/// Only has to keep up with the generation rate, not exceed it: at ~4.6 ms a chunk, two
/// threads mesh ~430 chunks/s against a generator that produces ~330/s on this machine.
#[cfg(not(target_arch = "wasm32"))]
pub fn mesh_worker_threads() -> usize {
    std::thread::available_parallelism()
        .map(|n| (n.get() / 4).clamp(2, 4))
        .unwrap_or(2)
}

/// Mesh jobs handed to the worker pool per loading frame.
///
/// Native can outrun the pool here — the scan only ever offers chunks that are loaded and
/// unmeshed — so this is a queue-depth knob, not a throughput one. On wasm there is no mesh
/// pool and `dispatch_remesh_work` runs inline on the main thread, so this keeps the same
/// per-frame cost bootstrap had before meshing moved off-thread.
#[inline]
pub const fn bootstrap_mesh_dispatch_per_frame() -> usize {
    if cfg!(target_arch = "wasm32") {
        16
    } else {
        32
    }
}

#[inline]
pub const fn bootstrap_max_chunk_uploads_per_frame() -> usize {
    if cfg!(target_arch = "wasm32") {
        16
    } else {
        24
    }
}

#[inline]
pub const fn bootstrap_upload_time_budget_ms() -> u128 {
    if cfg!(target_arch = "wasm32") {
        16
    } else {
        12
    }
}

#[inline]
pub const fn bootstrap_extra_pri_remesh_per_frame() -> usize {
    if cfg!(target_arch = "wasm32") {
        3
    } else {
        8
    }
}

#[inline]
pub const fn bootstrap_extra_neighbor_remesh_per_frame() -> usize {
    if cfg!(target_arch = "wasm32") {
        2
    } else {
        6
    }
}

/// Scale with the machine, leaving a core for the main thread. The bridge clamps to 4.
#[cfg(target_arch = "wasm32")]
pub fn wasm_worker_count() -> usize {
    web_sys::window()
        .map(|window| window.navigator().hardware_concurrency() as usize)
        .map(|cores| cores.saturating_sub(1).clamp(1, 4))
        .unwrap_or(2)
}

#[cfg(not(target_arch = "wasm32"))]
pub const fn wasm_worker_count() -> usize {
    2
}

/// Web chunk workers are fixed; load + mesh run off the main thread.
#[inline]
pub const fn use_wasm_chunk_workers() -> bool {
    true
}

/// Per-worker in-flight cap. The worker wasm instance is not re-entrant, but queueing a
/// second job keeps it busy while a reply crosses back to the main thread.
#[inline]
pub const fn max_worker_jobs_in_flight() -> usize {
    2
}

#[inline]
pub const fn max_worker_job_queue() -> usize {
    48
}

/// On wasm, only decode NBT sections near the surface when loading from disk.
#[inline]
pub const fn surface_only_chunk_load() -> bool {
    cfg!(target_arch = "wasm32")
}

pub const UNLOAD_MARGIN_CHUNKS: i32 = 5;
pub const REMESH_PRIORITY_BUDGET_PER_FRAME: usize = 2;
pub const NEIGHBOR_REMESH_BUDGET_PER_FRAME: usize = 1;
pub const PROCEDURAL_REMESH_PRIORITY_BUDGET_PER_FRAME: usize = 6;
pub const PROCEDURAL_NEIGHBOR_REMESH_BUDGET_PER_FRAME: usize = 3;
pub const READY_DRAIN_BUDGET_PER_FRAME: usize = 64;
pub const MAX_PENDING_READY_CHUNKS: usize = 600;

/// Stop requesting new chunk loads once this many finished ones are waiting to be applied.
///
/// Without a gate the streaming loop requests at a fixed rate per frame while the drain loop
/// applies at whatever rate meshing allows, so the queue fills, [`MAX_PENDING_READY_CHUNKS`]
/// trims completed loads off the back, and those chunks are requested and regenerated again —
/// measured at ~1300 requests/s to apply ~110 chunks/s. The backlog is only there to keep the
/// generator pool from starving between frames; anything past that is work thrown away.
pub const MAX_PENDING_LOAD_BACKLOG: usize = 128;
pub const PENDING_BACKLOG_REMESH_THRESHOLD: usize = 24;
pub const PENDING_BACKLOG_EXTRA_PRI_REMESH: usize = 4;
pub const PENDING_BACKLOG_EXTRA_NEIGHBOR_REMESH: usize = 3;
pub const TARGET_FRAME_TIME_US: u128 = 33_333;
pub const MIN_SECTION_DRAW_DISTANCE_CHUNKS: f32 = 4.0;
pub const SURFACE_LOD_DEPTH_BLOCKS: i32 = 48;
pub const FAR_DETAIL_RING: u8 = 2;

/// How far below the highest block in a chunk we still build GPU meshes.
/// Skips caves and deep stone — major win on wasm.
#[inline]
pub const fn surface_mesh_depth_blocks() -> i32 {
    if cfg!(target_arch = "wasm32") {
        24
    } else {
        48
    }
}
