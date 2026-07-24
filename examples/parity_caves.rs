//! Full-column parity: compare `mc::overworld::final_density` (now WITH cave carving) to
//! the real world, block by block, over a grid of generated columns.
//!
//! Metric: solid-vs-void agreement. A block is "solid" if `final_density > 0` (mine) or if
//! the oracle block is not air and not a fluid. Water/lava count as void on both sides —
//! where density > 0 MC places stone regardless of aquifers/surface-rules, so solid-vs-void
//! is a fair measure of terrain + cave shape while aquifers are still deferred.
//!
//!   • "extra solid"   = I say solid, oracle says void  (over-generating / missing a cave)
//!   • "missing solid" = I say void, oracle says solid   (under-generating)
//!
//! Run: `cargo run --release --example parity_caves -- "<region dir>" [seed]`

use std::fs::File;

use fastanvil::{Chunk, CurrentJavaChunk, Region};
use fastnbt::from_bytes;
use voxel_engine::mc::overworld::Overworld;

const Y_LO: i32 = -64;
const Y_HI: i32 = 200; // above this is air in these columns; skip to save time

fn main() {
    let mut args = std::env::args().skip(1);
    let region_dir = args.next().expect("usage: parity_caves <region dir> [seed]");
    let seed: i64 = args.next().map(|s| s.parse().unwrap()).unwrap_or(6954908675375307936);

    let ow = Overworld::new(seed);

    let mut blocks = 0u64;
    let mut agree = 0u64;
    let mut extra_solid = 0u64; // mine solid, oracle void
    let mut missing_solid = 0u64; // mine void, oracle solid
    let mut columns = 0u64;
    let mut worst: Vec<(i32, i32, u32)> = Vec::new();

    for bx in (-96..=96).step_by(8) {
        for bz in (-96..=96).step_by(8) {
            let Some(col) = oracle_column(&region_dir, bx, bz) else { continue };
            columns += 1;
            let mut col_disagree = 0u32;
            for y in Y_LO..=Y_HI {
                let mine_solid = ow.final_density(bx as f64, y as f64, bz as f64) > 0.0;
                let oracle_solid = col[(y - Y_LO) as usize];
                blocks += 1;
                if mine_solid == oracle_solid {
                    agree += 1;
                } else {
                    col_disagree += 1;
                    if mine_solid {
                        extra_solid += 1;
                    } else {
                        missing_solid += 1;
                    }
                }
            }
            worst.push((bx, bz, col_disagree));
        }
    }

    if blocks == 0 {
        println!("no generated columns found — check the region dir");
        return;
    }

    worst.sort_by_key(|&(_, _, d)| std::cmp::Reverse(d));
    let pct = 100.0 * agree as f64 / blocks as f64;
    println!("columns compared: {columns}");
    println!("blocks compared:  {blocks}  (y {Y_LO}..={Y_HI})");
    println!("agreement:        {pct:.3}%  ({agree}/{blocks})");
    println!("  extra solid:    {extra_solid}   (mine solid, oracle void)");
    println!("  missing solid:  {missing_solid}   (mine void, oracle solid)");
    println!("avg disagreements/column: {:.1}", (extra_solid + missing_solid) as f64 / columns as f64);
    println!("\nworst columns (block x,z : disagreeing blocks):");
    for &(x, z, d) in worst.iter().take(8) {
        println!("  {x:>5},{z:>5} : {d}");
    }
}

/// Per-y solidity for a column, indexed `[y - Y_LO]`. Solid = not air and not a fluid.
fn oracle_column(region_dir: &str, block_x: i32, block_z: i32) -> Option<Vec<bool>> {
    let region_x = block_x.div_euclid(512);
    let region_z = block_z.div_euclid(512);
    let cx = block_x.div_euclid(16).rem_euclid(32) as usize;
    let cz = block_z.div_euclid(16).rem_euclid(32) as usize;
    let lx = block_x.rem_euclid(16) as usize;
    let lz = block_z.rem_euclid(16) as usize;

    let path = format!("{region_dir}/r.{region_x}.{region_z}.mca");
    let file = File::open(&path).ok()?;
    let mut region = Region::from_stream(file).ok()?;
    let raw = region.read_chunk(cx, cz).ok()??;
    let chunk: CurrentJavaChunk = from_bytes(&raw).ok()?;

    let mut out = Vec::with_capacity((Y_HI - Y_LO + 1) as usize);
    for y in Y_LO..=Y_HI {
        let name = chunk.block(lx, y as isize, lz).map(|b| b.name()).unwrap_or("minecraft:air");
        out.push(is_solid(name));
    }
    Some(out)
}

fn is_solid(name: &str) -> bool {
    !matches!(
        name,
        "minecraft:air"
            | "minecraft:cave_air"
            | "minecraft:void_air"
            | "minecraft:water"
            | "minecraft:lava"
            | "minecraft:bubble_column"
    )
}
