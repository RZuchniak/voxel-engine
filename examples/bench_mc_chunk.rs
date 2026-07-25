//! How long does one Minecraft-parity chunk take to generate?
//!
//! The engine streams chunks on a budget, so this is the number that decides whether
//! `mc::chunk` can back a live `WorldSource` or needs a perf pass first. Reports the total
//! and a per-stage split, since the stages have very different costs.
//!
//! Run: `cargo run --release --example bench_mc_chunk -- [chunks] [seed]`

use std::time::Instant;

use voxel_engine::mc::chunk::generate_chunk;
use voxel_engine::mc::overworld::{CellSampler, Overworld};
use voxel_engine::mc::surface::SurfaceSystem;

fn main() {
    let mut args = std::env::args().skip(1);
    let count: i32 = args.next().map(|s| s.parse().unwrap()).unwrap_or(16);
    let seed: i64 = args.next().map(|s| s.parse().unwrap()).unwrap_or(6954908675375307936);

    let t0 = Instant::now();
    let ow = Overworld::new(seed);
    let surface = SurfaceSystem::new(seed);
    println!("world setup:        {:>8.1} ms", t0.elapsed().as_secs_f64() * 1000.0);

    // Stage split, measured on one chunk.
    let t = Instant::now();
    let mut sampler = CellSampler::new(&ow);
    for z in 0..16 {
        for x in 0..16 {
            for y in -64..320 {
                std::hint::black_box(sampler.final_density(x, y, z));
            }
        }
    }
    println!("density only:       {:>8.1} ms/chunk", t.elapsed().as_secs_f64() * 1000.0);

    let t = Instant::now();
    for z in (0..16).step_by(4) {
        for x in (0..16).step_by(4) {
            for y in (-64..320).step_by(4) {
                std::hint::black_box(ow.biome_at(x, y, z));
            }
        }
    }
    println!(
        "biome lookups only: {:>8.1} ms/chunk  (one per quart cell)",
        t.elapsed().as_secs_f64() * 1000.0
    );

    let t = Instant::now();
    for i in 0..count {
        std::hint::black_box(generate_chunk(&ow, &surface, i % 8, i / 8));
    }
    let total = t.elapsed().as_secs_f64() * 1000.0;
    println!("\nfull generate_chunk: {:>7.1} ms/chunk  ({count} chunks, {total:.0} ms total)", total / count as f64);
}
