//! Three-way parity: compare `mc::aquifer`'s **substance** (solid / water / lava / air) to
//! the real world, block by block. This supersedes `parity_caves`, which lumped water and
//! lava into "void" because aquifers weren't ported yet.
//!
//! The aquifer is per-chunk in vanilla, so this walks whole chunks: one `ChunkAquifer` per
//! chunk, every column in it (subsampled), every Y in range.
//!
//! Run: `cargo run --release --example parity_aquifer -- "<region dir>" [seed]`

use std::collections::BTreeMap;
use std::fs::File;

use fastanvil::{Chunk, CurrentJavaChunk, Region};
use fastnbt::from_bytes;
use voxel_engine::mc::aquifer::Substance;
use voxel_engine::mc::overworld::{CellSampler, Overworld};

const Y_LO: i32 = -64;
const Y_HI: i32 = 200;
/// Chunks to sweep around the origin, in each direction.
const CHUNK_RADIUS: i32 = 6;
/// Sample every Nth column within a chunk (16×16 columns × 265 Y is a lot of noise calls).
/// Deliberately **not** 4: the climate layer is quart-cached, so a step of 4 would only
/// ever land on quart-aligned columns and hide any error in the other 15/16 of the world.
const COLUMN_STEP: usize = 3;

fn main() {
    let mut args = std::env::args().skip(1);
    let region_dir = args.next().expect("usage: parity_aquifer <region dir> [seed]");
    let seed: i64 = args.next().map(|s| s.parse().unwrap()).unwrap_or(6954908675375307936);

    let ow = Overworld::new(seed);

    let mut blocks = 0u64;
    let mut agree = 0u64;
    let mut chunks = 0u64;
    // (mine, oracle) -> count, for every disagreeing pair.
    let mut confusion: BTreeMap<(&'static str, &'static str), u64> = BTreeMap::new();
    // Which real blocks are involved, per disagreement class — this is what tells apart a
    // density bug from a later worldgen step (carvers remove, features add).
    let mut culprits: BTreeMap<&'static str, BTreeMap<String, u64>> = BTreeMap::new();
    // ...and at which altitudes, which separates a surface-height bias from cave-shape error.
    let mut bands: BTreeMap<&'static str, BTreeMap<i32, u64>> = BTreeMap::new();
    let mut worst: Vec<(i32, i32, u32)> = Vec::new();

    for cx in -CHUNK_RADIUS..=CHUNK_RADIUS {
        for cz in -CHUNK_RADIUS..=CHUNK_RADIUS {
            let Some(chunk) = load_chunk(&region_dir, cx, cz) else { continue };
            chunks += 1;
            let mut aq = ow.aquifer_for_chunk(cx, cz);
            let mut sampler = CellSampler::new(&ow);
            let mut chunk_disagree = 0u32;

            for lx in (0..16).step_by(COLUMN_STEP) {
                for lz in (0..16).step_by(COLUMN_STEP) {
                    let x = cx * 16 + lx as i32;
                    let z = cz * 16 + lz as i32;
                    for y in Y_LO..=Y_HI {
                        let density = sampler.final_density(x, y, z);
                        let mine = aq.compute_substance(x, y, z, density);
                        let name =
                            chunk.block(lx, y as isize, lz).map(|b| b.name()).unwrap_or("minecraft:air");
                        let oracle = classify(name);
                        blocks += 1;
                        if mine == oracle {
                            agree += 1;
                        } else {
                            chunk_disagree += 1;
                            *confusion.entry((label(mine), label(oracle))).or_default() += 1;
                            if (mine == Substance::Air && oracle == Substance::Solid)
                                || (mine == Substance::Solid && oracle == Substance::Air)
                            {
                                let class = if mine == Substance::Air { "air → solid" } else { "solid → air" };
                                *culprits
                                    .entry(class)
                                    .or_default()
                                    .entry(name.trim_start_matches("minecraft:").to_string())
                                    .or_default() += 1;
                                *bands.entry(class).or_default().entry(y / 32 * 32).or_default() += 1;
                            }
                        }
                    }
                }
            }
            worst.push((cx, cz, chunk_disagree));
        }
    }

    if blocks == 0 {
        println!("no generated chunks found — check the region dir");
        return;
    }

    worst.sort_by_key(|&(_, _, d)| std::cmp::Reverse(d));
    println!("chunks compared:  {chunks}");
    println!("blocks compared:  {blocks}  (y {Y_LO}..={Y_HI}, every {COLUMN_STEP}th column)");
    println!("agreement:        {:.3}%  ({agree}/{blocks})", 100.0 * agree as f64 / blocks as f64);
    println!("\ndisagreements (mine → oracle):");
    let mut rows: Vec<_> = confusion.iter().collect();
    rows.sort_by_key(|&(_, n)| std::cmp::Reverse(*n));
    for ((mine, oracle), n) in rows {
        println!("  {mine:>5} → {oracle:<5} : {n:>8}  ({:.3}%)", 100.0 * *n as f64 / blocks as f64);
    }
    for (class, blocks) in &culprits {
        println!("\ntop real blocks behind \"{class}\":");
        let mut rows: Vec<_> = blocks.iter().collect();
        rows.sort_by_key(|&(_, n)| std::cmp::Reverse(*n));
        for (name, n) in rows.into_iter().take(12) {
            println!("  {name:<28} {n:>7}");
        }
    }
    for (class, by_y) in &bands {
        println!("\naltitude of \"{class}\" (32-block bands):");
        for (y, n) in by_y {
            println!("  y {y:>5}..{:<5} {n:>7}", y + 31);
        }
    }
    println!("\nworst chunks (chunk x,z : disagreeing blocks):");
    for &(x, z, d) in worst.iter().take(8) {
        println!("  {x:>4},{z:>4} : {d}");
    }
}

fn load_chunk(region_dir: &str, chunk_x: i32, chunk_z: i32) -> Option<CurrentJavaChunk> {
    let region_x = chunk_x.div_euclid(32);
    let region_z = chunk_z.div_euclid(32);
    let path = format!("{region_dir}/r.{region_x}.{region_z}.mca");
    let file = File::open(&path).ok()?;
    let mut region = Region::from_stream(file).ok()?;
    let raw = region
        .read_chunk(chunk_x.rem_euclid(32) as usize, chunk_z.rem_euclid(32) as usize)
        .ok()??;
    from_bytes(&raw).ok()
}

fn classify(name: &str) -> Substance {
    match name {
        "minecraft:air" | "minecraft:cave_air" | "minecraft:void_air" => Substance::Air,
        "minecraft:water" | "minecraft:bubble_column" => Substance::Water,
        "minecraft:lava" => Substance::Lava,
        _ => Substance::Solid,
    }
}

fn label(s: Substance) -> &'static str {
    match s {
        Substance::Solid => "solid",
        Substance::Air => "air",
        Substance::Water => "water",
        Substance::Lava => "lava",
    }
}
