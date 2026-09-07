//! Headless fly-forward generation profile.
//!
//! Holds a heading, requests chunks in the same distance-then-facing order the streamer
//! uses, and times terrain / decorate / mesh separately. Designed to be recorded with
//! samply so those stages show up as named frames:
//!
//! ```text
//! cargo build --profile profiling --example profile_fly
//! samply record --save-only -o fly.json.gz -- target\profiling\profile_fly.exe 6 forest
//! samply load fly.json.gz
//! ```
//!
//! Args: `[seconds] [yaw_deg|forest] [seed] [radius]`
//! Defaults: 6 seconds, seek a dense-tree heading, demo seed, load radius 8.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Instant;

use voxel_engine::mc::chunk::{generate_chunk_timed, ChunkBlocks, ChunkGenTimings, HEIGHT, MIN_Y};
use voxel_engine::mc::decorate::{decorate_centre, tree_kind_for_biome, NeighbourTerrain, TreeKind};
use voxel_engine::mc::overworld::Overworld;
use voxel_engine::mc::surface::{Block, SurfaceSystem};
use voxel_engine::mesh::mesh_chunk;
use voxel_engine::world::{Chunk, World, BIOME_CELLS, SECTION_SIZE};

const DEMO_SEED: i64 = 6_954_908_675_375_307_936;
const FLY_SPEED: f32 = 20.0;
const TICK_HZ: f32 = 60.0;
const REQUESTS_PER_TICK: usize = 12;

struct Args {
    seconds: f32,
    yaw_deg: Option<f32>,
    seed: i64,
    radius: i32,
}

fn parse_args() -> Args {
    let mut args = std::env::args().skip(1);
    let seconds = args
        .next()
        .map(|s| s.parse().expect("seconds"))
        .unwrap_or(6.0);
    let heading = args.next();
    let yaw_deg = match heading.as_deref() {
        None | Some("forest") => None,
        Some(s) => Some(s.parse().expect("yaw_deg")),
    };
    let seed = args
        .next()
        .map(|s| s.parse().expect("seed"))
        .unwrap_or(DEMO_SEED);
    let radius = args
        .next()
        .map(|s| s.parse().expect("radius"))
        .unwrap_or(8);
    Args {
        seconds,
        yaw_deg,
        seed,
        radius,
    }
}

fn tree_score(biome: &str) -> i32 {
    match tree_kind_for_biome(biome) {
        Some(TreeKind::Forest | TreeKind::BirchForest | TreeKind::Taiga | TreeKind::Grove) => 10,
        Some(TreeKind::Savanna) => 2,
        Some(_) => 1,
        None => 0,
    }
}

/// Pick a heading into a dense-tree biome near spawn, using cheap climate lookups rather
/// than full chunk generation.
fn seek_forest(ow: &Overworld) -> (f32, i32, i32, &'static str) {
    let mut best = (0i32, 0i32, 0i32, "plains");
    for cz in (-48..=48).step_by(2) {
        for cx in (-48..=48).step_by(2) {
            let x = cx * 16 + 8;
            let z = cz * 16 + 8;
            let y = ow.preliminary_surface_level(x as f64, z as f64);
            let biome = ow.biome_at(x, y, z);
            let score = tree_score(biome);
            if score > best.0 {
                best = (score, cx, cz, biome);
            }
        }
    }
    let yaw = (best.2 as f32).atan2(best.1 as f32);
    (yaw.to_degrees(), best.1, best.2, best.3)
}

#[derive(Clone, Copy, Default)]
struct StageAcc {
    chunks: usize,
    terrain_ms: f64,
    density_ms: f64,
    surface_ms: f64,
    terrain_misses: usize,
    decorate_ms: f64,
    assemble_ms: f64,
    mesh_ms: f64,
    tree_blocks: usize,
    log_blocks: usize,
}

impl StageAcc {
    fn add(&mut self, sample: &ChunkSample) {
        self.chunks += 1;
        self.terrain_ms += sample.terrain_ms;
        self.density_ms += sample.density_ms;
        self.surface_ms += sample.surface_ms;
        self.terrain_misses += sample.terrain_misses;
        self.decorate_ms += sample.decorate_ms;
        self.assemble_ms += sample.assemble_ms;
        self.mesh_ms += sample.mesh_ms;
        self.tree_blocks += sample.tree_blocks;
        self.log_blocks += sample.log_blocks;
    }

    fn total_ms(&self) -> f64 {
        self.terrain_ms + self.decorate_ms + self.assemble_ms + self.mesh_ms
    }

    fn print(&self, label: &str) {
        if self.chunks == 0 {
            println!("{label}: no chunks");
            return;
        }
        let n = self.chunks as f64;
        println!("=== {label} ({} chunks) ===", self.chunks);
        println!(
            "  terrain (generate_chunk) : {:>7.2} ms/chunk   ({:.0} ms total, {:.2} gens/chunk)",
            self.terrain_ms / n,
            self.terrain_ms,
            self.terrain_misses as f64 / n
        );
        println!(
            "    density + aquifer      : {:>7.2} ms/chunk",
            self.density_ms / n
        );
        println!(
            "    surface rules          : {:>7.2} ms/chunk",
            self.surface_ms / n
        );
        println!(
            "  decorate (trees, 3×3)    : {:>7.2} ms/chunk   ({:.0} ms total)",
            self.decorate_ms / n,
            self.decorate_ms
        );
        println!(
            "  assemble column          : {:>7.2} ms/chunk",
            self.assemble_ms / n
        );
        println!(
            "  mesh                     : {:>7.2} ms/chunk   ({:.0} ms total)",
            self.mesh_ms / n,
            self.mesh_ms
        );
        println!(
            "  all stages               : {:>7.2} ms/chunk   ({:.1} chunks/s single-thread)",
            self.total_ms() / n,
            1000.0 / (self.total_ms() / n)
        );
        println!(
            "  tree blocks              : {:.0}/chunk  ({} logs)",
            self.tree_blocks as f64 / n,
            self.log_blocks
        );
    }
}

struct ChunkSample {
    terrain_ms: f64,
    density_ms: f64,
    surface_ms: f64,
    terrain_misses: usize,
    decorate_ms: f64,
    assemble_ms: f64,
    mesh_ms: f64,
    tree_blocks: usize,
    log_blocks: usize,
}

struct FlyWorld {
    seed: i64,
    ow: Overworld,
    surface: SurfaceSystem,
    terrain: HashMap<(i32, i32), Arc<ChunkBlocks>>,
    world: World,
}

impl FlyWorld {
    fn new(seed: i64, ow: Overworld, surface: SurfaceSystem) -> Self {
        Self {
            seed,
            ow,
            surface,
            terrain: HashMap::new(),
            world: World::new(),
        }
    }

    fn terrain_at(&mut self, coord: (i32, i32)) -> (Arc<ChunkBlocks>, Option<ChunkGenTimings>) {
        if let Some(hit) = self.terrain.get(&coord) {
            return (Arc::clone(hit), None);
        }
        let (blocks, timings) = stage_terrain(&self.ow, &self.surface, coord.0, coord.1);
        let blocks = Arc::new(blocks);
        self.terrain.insert(coord, Arc::clone(&blocks));
        (blocks, Some(timings))
    }

    fn generate(&mut self, coord: (i32, i32)) -> ChunkSample {
        let mut density_ms = 0.0;
        let mut surface_ms = 0.0;
        let mut terrain_misses = 0usize;
        let mut held = Vec::with_capacity(9);
        let t_terrain = Instant::now();
        for dz in -1..=1 {
            for dx in -1..=1 {
                let ncoord = (coord.0 + dx, coord.1 + dz);
                let (blocks, timings) = self.terrain_at(ncoord);
                if let Some(t) = timings {
                    density_ms += t.density_aquifer_ms;
                    surface_ms += t.surface_ms;
                    terrain_misses += 1;
                }
                held.push((ncoord, blocks));
            }
        }
        let terrain_ms = t_terrain.elapsed().as_secs_f64() * 1000.0;

        let neighbours: Vec<NeighbourTerrain<'_>> = held
            .iter()
            .map(|((nx, nz), blocks)| NeighbourTerrain {
                chunk_x: *nx,
                chunk_z: *nz,
                blocks: blocks.as_ref(),
            })
            .collect();
        let (decorated, decorate_ms) = stage_decorate(self.seed, coord.0, coord.1, &neighbours);

        let mut tree_blocks = 0usize;
        let mut log_blocks = 0usize;
        for block in &decorated {
            if is_log(*block) {
                log_blocks += 1;
                tree_blocks += 1;
            } else if is_leaf(*block) {
                tree_blocks += 1;
            }
        }

        let centre = held
            .iter()
            .find(|(c, _)| *c == coord)
            .expect("centre of 3×3")
            .1
            .clone();
        let t_assemble = Instant::now();
        let chunk = assemble_chunk(coord, &decorated, &centre.surface_biomes);
        let assemble_ms = t_assemble.elapsed().as_secs_f64() * 1000.0;

        self.world.insert_chunk(chunk);
        let mesh_ms = stage_mesh(&self.world, coord);

        ChunkSample {
            terrain_ms,
            density_ms,
            surface_ms,
            terrain_misses,
            decorate_ms,
            assemble_ms,
            mesh_ms,
            tree_blocks,
            log_blocks,
        }
    }

    fn unload_far(&mut self, pcx: i32, pcz: i32, radius: i32) {
        let keep = radius + 2;
        let drop: Vec<(i32, i32)> = self
            .world
            .chunks()
            .map(|c| c.coord())
            .filter(|&(cx, cz)| (cx - pcx).abs() > keep || (cz - pcz).abs() > keep)
            .collect();
        for coord in drop {
            self.world.remove_chunk(coord);
        }
    }
}

#[inline(never)]
fn stage_terrain(
    ow: &Overworld,
    surface: &SurfaceSystem,
    cx: i32,
    cz: i32,
) -> (ChunkBlocks, ChunkGenTimings) {
    std::hint::black_box(generate_chunk_timed(ow, surface, cx, cz))
}

#[inline(never)]
fn stage_decorate(
    seed: i64,
    cx: i32,
    cz: i32,
    neighbours: &[NeighbourTerrain<'_>],
) -> (Vec<Block>, f64) {
    let t = Instant::now();
    let out = decorate_centre(seed, cx, cz, neighbours);
    let ms = t.elapsed().as_secs_f64() * 1000.0;
    (std::hint::black_box(out), ms)
}

#[inline(never)]
fn stage_mesh(world: &World, coord: (i32, i32)) -> f64 {
    let t = Instant::now();
    let meshes = mesh_chunk(world, coord);
    std::hint::black_box(meshes);
    t.elapsed().as_secs_f64() * 1000.0
}

fn assemble_chunk(
    coord: (i32, i32),
    decorated: &[Block],
    surface_biomes: &[&'static str; voxel_engine::mc::chunk::SURFACE_BIOME_CELLS],
) -> Chunk {
    let mut out = Chunk::new(coord);
    let mut biomes = [0u8; BIOME_CELLS];
    for (cell, name) in surface_biomes.iter().enumerate() {
        biomes[cell] = voxel_engine::biome_tint::biome_index(name);
    }
    out.set_biomes(biomes);
    for lz in 0..16usize {
        for lx in 0..16usize {
            for y in MIN_Y..MIN_Y + HEIGHT {
                let block = decorated[((y - MIN_Y) as usize) * 256 + lz * 16 + lx];
                if block != Block::Air {
                    out.set_block_world(lx, y, lz, block.block_id());
                }
            }
        }
    }
    out
}

fn is_log(block: Block) -> bool {
    matches!(block, Block::OakLog | Block::BirchLog | Block::SpruceLog)
}

fn is_leaf(block: Block) -> bool {
    matches!(
        block,
        Block::OakLeaves | Block::BirchLeaves | Block::SpruceLeaves
    )
}

fn desired_chunks(
    pcx: i32,
    pcz: i32,
    yaw: f32,
    radius: i32,
) -> Vec<((i32, i32), i32, f32)> {
    let forward = (yaw.cos(), yaw.sin());
    let mut desired = Vec::new();
    for dz in -radius..=radius {
        for dx in -radius..=radius {
            let dist2 = dx * dx + dz * dz;
            let facing = if dist2 == 0 {
                1.0
            } else {
                let len = (dist2 as f32).sqrt();
                (dx as f32 / len) * forward.0 + (dz as f32 / len) * forward.1
            };
            desired.push(((pcx + dx, pcz + dz), dist2, facing));
        }
    }
    desired.sort_by(|a, b| {
        a.1.cmp(&b.1)
            .then_with(|| b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal))
    });
    desired
}

fn main() {
    let args = parse_args();
    let t_setup = Instant::now();
    let ow = Overworld::new(args.seed);
    let surface = SurfaceSystem::new(args.seed);
    println!(
        "world setup: {:.1} ms  seed {}",
        t_setup.elapsed().as_secs_f64() * 1000.0,
        args.seed
    );

    let (yaw_deg, forest_cx, forest_cz, forest_biome) = match args.yaw_deg {
        Some(yaw) => (yaw, 0, 0, "given"),
        None => seek_forest(&ow),
    };
    let yaw = yaw_deg.to_radians();
    // Start a few chunks short of the forest so the fly actually *enters* it, matching
    // the "flying towards a dense area" report. A given yaw starts at spawn.
    let (mut x, mut z) = if args.yaw_deg.is_some() {
        (8.0f32, 8.0f32)
    } else {
        let back = 5.0 * SECTION_SIZE as f32;
        (
            forest_cx as f32 * 16.0 + 8.0 - yaw.cos() * back,
            forest_cz as f32 * 16.0 + 8.0 - yaw.sin() * back,
        )
    };
    let y = 120.0f32;
    println!(
        "heading {yaw_deg:.1}°  start=({x:.0},{y:.0},{z:.0})  forest=({forest_cx},{forest_cz} {forest_biome})"
    );
    println!(
        "live-app reproduction:\n  $env:VOXEL_CAMERA=\"{x:.0},{y:.0},{z:.0},{yaw_deg:.1},0\"\n  $env:VOXEL_AUTOPILOT=\"forward\""
    );

    let mut fly = FlyWorld::new(args.seed, ow, surface);
    let mut loaded = HashSet::new();
    let mut bootstrap = StageAcc::default();
    let mut flying = StageAcc::default();
    let mut by_logs: HashMap<usize, (usize, f64, f64, f64)> = HashMap::new();

    let mut pcx = (x.floor() as i32).div_euclid(SECTION_SIZE as i32);
    let mut pcz = (z.floor() as i32).div_euclid(SECTION_SIZE as i32);

    fn generate_wanted(
        fly: &mut FlyWorld,
        loaded: &mut HashSet<(i32, i32)>,
        acc: &mut StageAcc,
        by_logs: &mut HashMap<usize, (usize, f64, f64, f64)>,
        pcx: i32,
        pcz: i32,
        yaw: f32,
        radius: i32,
        budget: usize,
    ) -> usize {
        let mut n = 0usize;
        for (coord, _, _) in desired_chunks(pcx, pcz, yaw, radius) {
            if n >= budget {
                break;
            }
            if !loaded.insert(coord) {
                continue;
            }
            let sample = fly.generate(coord);
            let bucket = (sample.log_blocks / 5).min(12);
            let entry = by_logs.entry(bucket).or_insert((0, 0.0, 0.0, 0.0));
            entry.0 += 1;
            entry.1 += sample.terrain_ms;
            entry.2 += sample.decorate_ms;
            entry.3 += sample.mesh_ms;
            acc.add(&sample);
            n += 1;
        }
        n
    }

    let t_boot = Instant::now();
    let bootstrap_area = (2 * args.radius + 1) as usize;
    let bootstrap_total = bootstrap_area * bootstrap_area;
    loop {
        let made = generate_wanted(
            &mut fly,
            &mut loaded,
            &mut bootstrap,
            &mut by_logs,
            pcx,
            pcz,
            yaw,
            args.radius,
            64,
        );
        if made == 0 {
            break;
        }
    }
    println!(
        "bootstrap: {} / {} tiles in {:.1}s",
        bootstrap.chunks,
        bootstrap_total,
        t_boot.elapsed().as_secs_f64()
    );
    bootstrap.print("bootstrap (standing still)");

    let ticks = (args.seconds * TICK_HZ).max(1.0) as usize;
    let step = FLY_SPEED / TICK_HZ;
    let t_fly = Instant::now();
    let mut requested = 0usize;
    for _ in 0..ticks {
        x += yaw.cos() * step;
        z += yaw.sin() * step;
        pcx = (x.floor() as i32).div_euclid(SECTION_SIZE as i32);
        pcz = (z.floor() as i32).div_euclid(SECTION_SIZE as i32);
        requested += generate_wanted(
            &mut fly,
            &mut loaded,
            &mut flying,
            &mut by_logs,
            pcx,
            pcz,
            yaw,
            args.radius,
            REQUESTS_PER_TICK,
        );
        fly.unload_far(pcx, pcz, args.radius);
    }
    let fly_wall = t_fly.elapsed().as_secs_f64();
    println!(
        "\nflew {:.0} blocks in {fly_wall:.1}s sim ({:.0} s wall)  new chunks {requested}  end=({x:.0},{z:.0})",
        FLY_SPEED * args.seconds,
        args.seconds
    );
    flying.print("fly (new chunks at the load front)");

    let front_width = 2 * args.radius + 1;
    let chunks_per_chunk_travelled = front_width as f64;
    let needed = (FLY_SPEED / SECTION_SIZE as f32) as f64 * chunks_per_chunk_travelled;
    if flying.chunks > 0 {
        let have = 1000.0 / (flying.total_ms() / flying.chunks as f64);
        println!(
            "\nkeeping up: this heading needs ~{needed:.0} chunks/s at the front \
             (radius {}, {} block/s). single-thread produces {have:.0}/s.",
            args.radius, FLY_SPEED
        );
        if have < needed {
            println!(
                "  behind: empty chunks while flying are expected until workers cover the gap."
            );
        }
    }

    println!("\n=== cost vs log count (bootstrap + fly) ===");
    println!(
        "  {:>10}  {:>7}  {:>12}  {:>12}  {:>10}",
        "logs", "chunks", "terrain ms", "decorate ms", "mesh ms"
    );
    let mut keys: Vec<_> = by_logs.keys().copied().collect();
    keys.sort_unstable();
    for bucket in keys {
        let (count, terrain, decorate, mesh) = by_logs[&bucket];
        let label = if bucket == 0 {
            "0-4".to_string()
        } else {
            format!("{}-{}", bucket * 5, bucket * 5 + 4)
        };
        println!(
            "  {label:>10}  {count:>7}  {:>12.1}  {:>12.1}  {:>10.1}",
            terrain / count as f64,
            decorate / count as f64,
            mesh / count as f64
        );
    }
}
