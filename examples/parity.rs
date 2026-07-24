//! Parity probe: compare the `mc::overworld` predicted surface altitude against the real
//! "Voxel" world, column by column. This is the end-to-end signal for the generator.
//!
//! Caveat: `mc::overworld` is still *pre-cave / pre-aquifer / pre-surface-rules*, so exact
//! block matches aren't expected yet — but the predicted terrain surface altitude should
//! track the oracle's first-solid-from-top closely on land columns. A cave breaching the
//! surface, or water/beaches, will show up as outliers.
//!
//! Run: `cargo run --example parity -- "<region dir>" <seed>`

use std::fs::File;

use fastanvil::{Chunk, CurrentJavaChunk, Region};
use fastnbt::from_bytes;
use voxel_engine::mc::overworld::Overworld;

fn main() {
    let mut args = std::env::args().skip(1);
    let region_dir = args.next().expect("usage: parity <region dir> <seed>");
    let seed: i64 = args.next().map(|s| s.parse().unwrap()).unwrap_or(6954908675375307936);

    let ow = Overworld::new(seed);

    // Sample a grid of columns around spawn (only generated chunks yield data).
    let mut diffs: Vec<i32> = Vec::new();
    let mut checked = 0;
    let mut matched = 0;
    println!("{:>6} {:>6} | {:>7} {:>7} {:>6}", "x", "z", "oracle", "predict", "diff");
    for bx in (-96..=96).step_by(16) {
        for bz in (-96..=96).step_by(16) {
            let Some(oracle) = oracle_surface(&region_dir, bx, bz) else { continue };
            let predict = ow.surface_y(bx as f64, bz as f64, -64, 320).unwrap_or(-64);
            let diff = predict - oracle;
            diffs.push(diff);
            checked += 1;
            if diff == 0 {
                matched += 1;
            }
            if diff.abs() >= 3 {
                println!("{bx:>6} {bz:>6} | {oracle:>7} {predict:>7} {diff:>6}  <-- outlier");
            } else {
                println!("{bx:>6} {bz:>6} | {oracle:>7} {predict:>7} {diff:>6}");
            }
        }
    }

    if checked == 0 {
        println!("no generated columns found in range — is the region dir correct?");
        return;
    }
    diffs.sort_unstable();
    let n = diffs.len();
    let mean: f64 = diffs.iter().map(|&d| d as f64).sum::<f64>() / n as f64;
    let median = diffs[n / 2];
    let within1 = diffs.iter().filter(|&&d| d.abs() <= 1).count();
    let within3 = diffs.iter().filter(|&&d| d.abs() <= 3).count();
    println!("\n--- {checked} columns ---");
    println!("exact match:  {matched}/{checked}");
    println!("within ±1:    {within1}/{checked}");
    println!("within ±3:    {within3}/{checked}");
    println!("mean diff:    {mean:+.2}   median diff: {median:+}");
    println!("range:        [{}, {}]", diffs[0], diffs[n - 1]);
}

/// First non-air block from the top of the column (the oracle surface altitude).
fn oracle_surface(region_dir: &str, block_x: i32, block_z: i32) -> Option<i32> {
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

    for y in (-64..320).rev() {
        let name = chunk.block(lx, y as isize, lz).map(|b| b.name()).unwrap_or("minecraft:air");
        if name != "minecraft:air" && name != "minecraft:cave_air" && name != "minecraft:void_air" {
            return Some(y);
        }
    }
    None
}
