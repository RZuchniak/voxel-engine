//! Oracle probe: read a real block column out of the "Voxel" world's region files.
//!
//! This is the ground-truth signal for MC-parity generation — whatever `src/mc/`
//! generates must match what the game actually wrote here. This example just proves we
//! can decode 26.2 region data and prints a column so we can eyeball it (and see the
//! block palette / y-range we'll diff against).
//!
//! Run: `cargo run --example oracle -- "<region dir>" <blockX> <blockZ>`

use std::fs::File;

use fastanvil::{Chunk, CurrentJavaChunk, Region};
use fastnbt::from_bytes;

fn main() {
    let mut args = std::env::args().skip(1);
    let region_dir = args.next().expect("usage: oracle <region dir> <blockX> <blockZ>");
    let block_x: i32 = args.next().map(|s| s.parse().unwrap()).unwrap_or(0);
    let block_z: i32 = args.next().map(|s| s.parse().unwrap()).unwrap_or(0);

    // Region file = 512×512 blocks (32×32 chunks). Chunk-in-region is mod 32.
    let region_x = block_x.div_euclid(512);
    let region_z = block_z.div_euclid(512);
    let chunk_x = block_x.div_euclid(16);
    let chunk_z = block_z.div_euclid(16);
    let cx_in_region = chunk_x.rem_euclid(32) as usize;
    let cz_in_region = chunk_z.rem_euclid(32) as usize;
    let local_x = block_x.rem_euclid(16) as usize;
    let local_z = block_z.rem_euclid(16) as usize;

    let path = format!("{region_dir}/r.{region_x}.{region_z}.mca");
    println!("region file: {path}");
    println!("chunk ({chunk_x},{chunk_z})  local ({local_x},{local_z})");

    let file = File::open(&path).expect("open region file");
    let mut region = Region::from_stream(file).expect("parse region");

    let raw = match region.read_chunk(cx_in_region, cz_in_region).expect("read chunk") {
        Some(raw) => raw,
        None => {
            println!("chunk not generated (no data in region) — pick a coordinate near spawn");
            return;
        }
    };

    let chunk: CurrentJavaChunk = from_bytes(&raw).expect("decode chunk NBT");

    // Print the column top-down so the surface is at the top of the output.
    println!("--- column at block ({block_x},{block_z}) ---");
    let mut last: Option<String> = None;
    let mut run_start = 0i32;
    for y in (-64..320).rev() {
        let name = chunk
            .block(local_x, y as isize, local_z)
            .map(|b| b.name().to_string())
            .unwrap_or_else(|| "<none>".to_string());
        match &last {
            Some(prev) if *prev == name => {}
            _ => {
                if let Some(prev) = &last {
                    println!("  y {:>4}..{:<4}  {}", run_start, y + 1, prev);
                }
                last = Some(name);
                run_start = y;
            }
        }
    }
    if let Some(prev) = &last {
        println!("  y {:>4}..{:<4}  {}", run_start, -64, prev);
    }
}
