/// Runtime tuning — tighter on wasm for browser memory and single-threaded meshing.

/// Render distance, in chunks.
///
/// **Cut hard when the surface band was dropped**, native 40 -> 20 and wasm 16 -> 12, because
/// a chunk is now the full `-64..320` column rather than a shell around the surface. A fully
/// populated chunk is `24 sections x 4096 blocks x 2 bytes = 196 KB` of block data, against
/// roughly 16-32 KB banded — so residency is ~8x per chunk:
///
/// | radius | chunks | block data |
/// |---|---|---|
/// | 40 (old native) | 6561 | **1.29 GB** |
/// | 20 (native) | 1681 | 330 MB |
/// | 16 | 1089 | 214 MB |
/// | 12 (wasm) | 625 | 122 MB |
///
/// 40 was never affordable full-depth; that is the number that had to move. It is also more
/// honest — vanilla's maximum is 32 and its default 12, and the occlusion graph's traversal
/// cost scales with the volume too (radius 16 is 26k sections against radius 41's 165k).
/// Native override: **`VOXEL_LOAD_DISTANCE=<chunks>`** (measurement knob, not clamped).
///
/// Raising *this* is the only way to put more geometry on screen. Draw distance is bounded by it
/// in practice — a chunk outside the load radius has no mesh, so there is nothing to draw there —
/// which is why `VOXEL_DRAW_DISTANCE` alone cannot make the occlusion-graph A/B harder: at load
/// 20, draw 32 draws exactly what draw 20 draws.
///
/// ⚠️ Costs memory linearly in *area*, and a full column is ~196 KB of block data: radius 20 is
/// ~330 MB resident, radius 28 ~640 MB, radius 32 ~830 MB. Deliberately unclamped so a
/// measurement can go where the default will not; do not raise the default without redoing the
/// memory table below.
#[inline]
pub fn load_distance_chunks() -> i32 {
    #[cfg(target_arch = "wasm32")]
    {
        12
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        use std::sync::OnceLock;
        static DISTANCE: OnceLock<i32> = OnceLock::new();
        *DISTANCE.get_or_init(|| {
            std::env::var("VOXEL_LOAD_DISTANCE")
                .ok()
                .and_then(|v| v.trim().parse::<i32>().ok())
                .filter(|v| *v > 0)
                .unwrap_or(20)
        })
    }
}

/// Defaults to the load distance on both targets — drawing further than you load is a no-op, and
/// the two were already equal (native 20/20, wasm 12/12) before this was written down.
///
/// Native override: **`VOXEL_DRAW_DISTANCE=<chunks>`**, for A/B'ing draw-call cost against a
/// fixed load radius. To actually add geometry, raise `VOXEL_LOAD_DISTANCE` instead (or as well).
#[inline]
pub fn default_section_draw_distance_chunks() -> f32 {
    #[cfg(target_arch = "wasm32")]
    {
        load_distance_chunks() as f32
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        use std::sync::OnceLock;
        static DISTANCE: OnceLock<f32> = OnceLock::new();
        *DISTANCE.get_or_init(|| {
            std::env::var("VOXEL_DRAW_DISTANCE")
                .ok()
                .and_then(|v| v.trim().parse::<f32>().ok())
                .filter(|v| *v > 0.0)
                .unwrap_or_else(|| load_distance_chunks() as f32)
        })
    }
}

/// The ceiling the `[` / `]` keys clamp to. Never below the starting distance, or an overridden
/// `VOXEL_DRAW_DISTANCE` would be clamped straight back down on the first keypress.
#[inline]
pub fn max_section_draw_distance_chunks() -> f32 {
    let platform_max: f32 = if cfg!(target_arch = "wasm32") {
        16.0
    } else {
        32.0
    };
    platform_max.max(default_section_draw_distance_chunks())
}

/// How much world is generated *and meshed* before the player is let in.
///
/// This is the loading screen's size. Everything inside it is finished terrain when the
/// world is revealed, so raising it trades a longer wait for a cleaner arrival — fewer
/// frame-time dips and less pop-in while the rest streams in behind you.
///
/// **On wasm this is the whole render distance**, deliberately. It used to be 3 (49 chunks)
/// against a load radius of 12 (625), so the reveal was followed by ~576 chunks arriving at
/// once — generated off-thread, but meshed and uploaded on the main thread, which is what made
/// the first stretch of play choppy until the queue reached zero. Building the full radius up
/// front moves that work behind the loading screen, where frame time is free. It costs a
/// 10–20 s wait; `?loading_radius=<chunks>` in the page URL overrides it (0 disables the wait).
///
/// Native default stays 12 of a 20 radius, overridable with `VOXEL_LOADING_RADIUS=<chunks>`:
/// native meshes on a worker pool, so its post-reveal burst is far cheaper.
#[inline]
pub fn bootstrap_chunk_radius() -> i32 {
    #[cfg(target_arch = "wasm32")]
    {
        use std::sync::OnceLock;
        static RADIUS: OnceLock<i32> = OnceLock::new();
        *RADIUS.get_or_init(|| {
            url_query_param("loading_radius")
                .and_then(|v| v.trim().parse::<i32>().ok())
                .map(|v| v.clamp(0, load_distance_chunks()))
                .unwrap_or_else(load_distance_chunks)
        })
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

/// One `?key=value` from the page URL — the browser's stand-in for an env var.
#[cfg(target_arch = "wasm32")]
fn url_query_param(key: &str) -> Option<String> {
    let search = web_sys::window()?.location().search().ok()?;
    search
        .trim_start_matches('?')
        .split('&')
        .filter_map(|pair| pair.split_once('='))
        .find(|(k, _)| *k == key)
        .map(|(_, v)| v.to_string())
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
/// **On by default.** This is now the *only* thing keeping the underground off the screen —
/// chunks are generated full-depth (`mc::chunk::generate_chunk`) and every populated section
/// is meshed, so without the walk you would draw the whole column.
///
/// It was off while generation banded, and correctly so: a section with no block data has to
/// be treated as transparent, so over a hollow shell the walk fell through the ground and
/// explored ~21k sections of nothing every frame, costing more than the skipped draws saved
/// (274 → 227 FPS). With solid rock underneath, the walk terminates at the surface instead.
///
/// `VOXEL_SECTION_CULLING=0` disables it for measurement. A bug here shows up as terrain
/// popping out of existence, so compare `reachable_sections` and `visible_draws` in the profile
/// line rather than trying to catch it by eye.
#[inline]
pub fn section_occlusion_culling() -> bool {
    #[cfg(target_arch = "wasm32")]
    {
        true
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        use std::sync::OnceLock;
        static ENABLED: OnceLock<bool> = OnceLock::new();
        *ENABLED.get_or_init(|| {
            std::env::var("VOXEL_SECTION_CULLING")
                .map(|v| v.trim() != "0")
                .unwrap_or(true)
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
pub const MIN_SECTION_DRAW_DISTANCE_CHUNKS: f32 = 4.0;

/// The frame time streaming is allowed to cost before it starts giving budget back.
///
/// Wasm is stricter (50 FPS vs 30) because everything that costs frame time there — meshing,
/// GPU upload, the occlusion walk — is on the one thread, so overrunning is the *only* way the
/// browser build can go choppy. On native, meshing is on a pool and 30 FPS is the older,
/// looser floor that the upload ladder was tuned around.
#[inline]
pub const fn target_frame_time_us() -> u128 {
    if cfg!(target_arch = "wasm32") {
        20_000
    } else {
        33_333
    }
}

/// Never back off past this fraction: streaming has to keep making progress, or a heavy frame
/// becomes a permanent stall and the world stops filling in.
const MIN_STREAMING_BUDGET_SCALE: f32 = 0.15;

/// How much of the per-frame streaming budget to spend, given how far the *last* frame overran
/// [`target_frame_time_us`].
///
/// A no-op (1.0) whenever there is frame-time headroom, so it costs nothing in the steady
/// state; it only engages once uploading and meshing are themselves what is making frames
/// long. Full budget at the target, [`MIN_STREAMING_BUDGET_SCALE`] at twice it, linear
/// between. This replaced a native-only three-step ladder that wasm never reached — which is
/// why the browser build had *no* frame-rate floor at all while the post-reveal queue drained.
pub fn streaming_budget_scale(frame_over_us: u128) -> f32 {
    if frame_over_us == 0 {
        return 1.0;
    }
    let over = frame_over_us as f32 / target_frame_time_us() as f32;
    (1.0 - over).clamp(MIN_STREAMING_BUDGET_SCALE, 1.0)
}

/// Apply [`streaming_budget_scale`] to a count, never reaching zero.
pub fn scale_budget(base: usize, scale: f32) -> usize {
    ((base as f32 * scale).round() as usize).max(1)
}

/// Apply [`streaming_budget_scale`] to a millisecond budget, never reaching zero.
pub fn scale_time_budget(base_ms: u128, scale: f32) -> u128 {
    if base_ms == u128::MAX {
        return base_ms;
    }
    ((base_ms as f32 * scale).round() as u128).max(1)
}

/// Finished chunks still queued for GPU upload when the loading screen is allowed to end.
///
/// The readiness scan counts a tile as meshed when its result *arrives*, which is before it is
/// uploaded — so without this the reveal could fire with hundreds of meshes still queued, and
/// they would land as exactly the burst of frame time the loading screen exists to absorb.
/// Nothing new is requested once the radius is full, so the queue drains in a few frames.
pub const REVEAL_MAX_PENDING_UPLOADS: usize = 0;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn streaming_backoff_is_free_when_frames_have_headroom() {
        // The steady state must pay nothing for the floor — a frame inside the target gets the
        // whole budget, so this cannot explain away a slow build.
        assert_eq!(streaming_budget_scale(0), 1.0);
        assert_eq!(scale_budget(12, streaming_budget_scale(0)), 12);
        assert_eq!(scale_time_budget(16, streaming_budget_scale(0)), 16);
    }

    #[test]
    fn streaming_backoff_tightens_as_frames_run_long() {
        let target = target_frame_time_us();
        let half = streaming_budget_scale(target / 2);
        let double = streaming_budget_scale(target);
        assert!((half - 0.5).abs() < 1e-3, "half a target over => half budget, got {half}");
        assert_eq!(double, MIN_STREAMING_BUDGET_SCALE, "a whole target over => the floor");
        assert!(half > double);
    }

    #[test]
    fn streaming_backoff_never_stalls_streaming_completely() {
        // A budget that reaches zero is a permanent stall: no uploads means no shorter frames
        // means no uploads. Every budget has to keep at least one unit of progress.
        let worst = streaming_budget_scale(u128::MAX / 2);
        assert_eq!(worst, MIN_STREAMING_BUDGET_SCALE);
        assert!(scale_budget(1, worst) >= 1);
        assert!(scale_budget(12, worst) >= 1);
        assert!(scale_time_budget(1, worst) >= 1);
        assert!(scale_time_budget(16, worst) >= 1);
    }

    #[test]
    fn an_unbounded_time_budget_stays_unbounded() {
        // Bootstrap uploads pass `u128::MAX`; scaling it must not wrap it to something tiny.
        assert_eq!(scale_time_budget(u128::MAX, 0.5), u128::MAX);
    }

    #[test]
    fn the_loading_screen_covers_the_whole_render_distance_on_wasm() {
        // The point of the change: nothing is left to stream in when the world is revealed, so
        // there is no post-reveal burst of main-thread meshing and uploading.
        if cfg!(target_arch = "wasm32") {
            assert_eq!(bootstrap_chunk_radius(), load_distance_chunks());
        } else {
            assert!(bootstrap_chunk_radius() <= load_distance_chunks());
        }
    }

    #[test]
    fn drawing_never_reaches_past_loading() {
        // Geometry outside the load radius has no mesh, so a draw distance beyond it buys
        // nothing but a bigger frustum loop. Keeping the *default* equal to the load distance is
        // what makes `VOXEL_LOAD_DISTANCE` alone enough to scale a measurement.
        assert_eq!(
            default_section_draw_distance_chunks(),
            load_distance_chunks() as f32
        );
    }

    #[test]
    fn the_draw_distance_ceiling_never_clamps_the_starting_distance() {
        // `max_section_draw_distance_chunks` is what `[` / `]` clamp to. If an override pushed the
        // start above the ceiling, the first keypress would yank it back down.
        assert!(max_section_draw_distance_chunks() >= default_section_draw_distance_chunks());
    }
}

/// How much of an imported Anvil chunk to decode, below its lowest column top.
///
/// **This no longer touches the procedural path or the mesher** — seeded worlds generate the
/// full `-64..320` column and every populated section is meshed. It survives only as a memory
/// bound on zip imports, where a full NBT decode of every chunk in the load radius is real
/// browser memory (see `surface_only_chunk_load`).
///
/// `VOXEL_MESH_DEPTH=<blocks>` overrides it on native.
#[inline]
pub fn surface_band_depth_blocks() -> i32 {
    #[cfg(target_arch = "wasm32")]
    {
        24
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        use std::sync::OnceLock;
        static DEPTH: OnceLock<i32> = OnceLock::new();
        *DEPTH.get_or_init(|| {
            std::env::var("VOXEL_MESH_DEPTH")
                .ok()
                .and_then(|v| v.trim().parse::<i32>().ok())
                .map(|v| v.max(0))
                .unwrap_or(48)
        })
    }
}
