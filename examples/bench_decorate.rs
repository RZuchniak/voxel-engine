//! Split the tree-decoration tax from terrain generation.
//!
//! Run: `cargo run --release --example bench_decorate -- [chunks] [seed]`

use std::sync::Arc;
use std::time::Instant;

use voxel_engine::mc::chunk::generate_chunk;
use voxel_engine::mc::decorate::{decorate_centre, NeighbourTerrain};
use voxel_engine::mc::overworld::Overworld;
use voxel_engine::mc::surface::{Block, SurfaceSystem};

fn main() {
    let mut args = std::env::args().skip(1);
    let count: i32 = args.next().map(|s| s.parse().unwrap()).unwrap_or(25);
    let seed: i64 = args
        .next()
        .map(|s| s.parse().unwrap())
        .unwrap_or(6954908675375307936);

    let ow = Overworld::new(seed);
    let surface = SurfaceSystem::new(seed);

    // Pre-generate a padded grid so decorate is measured without terrain-cache misses.
    let side = (count as f64).sqrt().ceil() as i32;
    let mut terrain = std::collections::HashMap::new();
    for z in -1..=side {
        for x in -1..=side {
            terrain.insert((x, z), Arc::new(generate_chunk(&ow, &surface, x, z)));
        }
    }

    let mut decorate_ms = 0.0;
    let mut tree_blocks = 0usize;
    let mut n = 0usize;
    for cz in 0..side {
        for cx in 0..side {
            if n >= count as usize {
                break;
            }
            let mut held = Vec::with_capacity(9);
            for dz in -1..=1 {
                for dx in -1..=1 {
                    let blocks = terrain.get(&(cx + dx, cz + dz)).expect("pre-gen").clone();
                    held.push(((cx + dx, cz + dz), blocks));
                }
            }
            let neighbours: Vec<_> = held
                .iter()
                .map(|((nx, nz), blocks)| NeighbourTerrain {
                    chunk_x: *nx,
                    chunk_z: *nz,
                    blocks: blocks.as_ref(),
                })
                .collect();
            let t = Instant::now();
            let out = decorate_centre(seed, cx, cz, &neighbours);
            decorate_ms += t.elapsed().as_secs_f64() * 1000.0;
            tree_blocks += out
                .iter()
                .filter(|b| {
                    matches!(
                        *b,
                        Block::OakLog
                            | Block::OakLeaves
                            | Block::BirchLog
                            | Block::BirchLeaves
                            | Block::SpruceLog
                            | Block::SpruceLeaves
                    )
                })
                .count();
            std::hint::black_box(out);
            n += 1;
        }
    }

    println!(
        "decorate_centre: {:>7.2} ms/chunk  ({n} chunks, terrain pre-warmed, {tree_blocks} tree blocks)",
        decorate_ms / n as f64
    );
}
