//! Full block-name parity: generate whole chunks (density → aquifer → surface rules) and
//! compare every block to the real world by name.
//!
//! This is the top-level scoreboard, superseding `parity_aquifer` (which only compared
//! solid/water/lava/air). It is the first harness that can tell grass from dirt from sand.
//!
//! Expect a floor on the achievable score: features (trees, grass, flowers, ores),
//! structures and carvers all run *after* the stages ported here, so blocks they place will
//! always read as disagreements. The confusion table separates those out — anything in the
//! `stone → *_ore` or `air → <plant>` families is a later stage, not a bug here.
//!
//! Run: `cargo run --release --example parity_surface -- "<region dir>" [seed]`

use std::collections::BTreeMap;
use std::fs::File;

use fastanvil::{Chunk, CurrentJavaChunk, Region};
use fastnbt::from_bytes;
use voxel_engine::mc::chunk::{generate_chunk, HEIGHT, MIN_Y};
use voxel_engine::mc::overworld::Overworld;
use voxel_engine::mc::surface::SurfaceSystem;

const CHUNK_RADIUS: i32 = 3;

fn main() {
    let mut args = std::env::args().skip(1);
    let region_dir = args.next().expect("usage: parity_surface <region dir> [seed]");
    let seed: i64 = args.next().map(|s| s.parse().unwrap()).unwrap_or(6954908675375307936);

    let ow = Overworld::new(seed);
    let surface = SurfaceSystem::new(seed);

    let mut total = 0u64;
    let mut agree = 0u64;
    let mut chunks = 0u64;
    let mut confusion: BTreeMap<(String, String), u64> = BTreeMap::new();
    // The surface band — the topmost few solid blocks of each column — is what the surface
    // rules actually govern. Ore blobs and structures barely reach it, so this scores this
    // stage rather than the ones that come after it.
    let mut band_total = 0u64;
    let mut band_agree = 0u64;
    let mut band_confusion: BTreeMap<(String, String), u64> = BTreeMap::new();
    const BAND_DEPTH: i32 = 5;

    for cx in -CHUNK_RADIUS..=CHUNK_RADIUS {
        for cz in -CHUNK_RADIUS..=CHUNK_RADIUS {
            let Some(oracle) = load_chunk(&region_dir, cx, cz) else { continue };
            chunks += 1;
            let mine = generate_chunk(&ow, &surface, cx, cz);
            for lz in 0..16usize {
                for lx in 0..16usize {
                    // Where my own column's surface is, so the band can be measured.
                    let my_top = (MIN_Y..MIN_Y + HEIGHT)
                        .rev()
                        .find(|&y| mine.get(lx, y, lz).name() != "air");
                    for y in MIN_Y..MIN_Y + HEIGHT {
                        let want = oracle
                            .block(lx, y as isize, lz)
                            .map(|b| b.name())
                            .unwrap_or("minecraft:air")
                            .trim_start_matches("minecraft:");
                        let got = mine.get(lx, y, lz).name();
                        total += 1;
                        if got == want {
                            agree += 1;
                        } else {
                            *confusion
                                .entry((got.to_string(), want.to_string()))
                                .or_default() += 1;
                        }

                        if let Some(top) = my_top {
                            if y <= top && y > top - BAND_DEPTH {
                                band_total += 1;
                                if got == want {
                                    band_agree += 1;
                                } else {
                                    *band_confusion
                                        .entry((got.to_string(), want.to_string()))
                                        .or_default() += 1;
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    if total == 0 {
        println!("no generated chunks found — check the region dir");
        return;
    }

    println!("chunks compared: {chunks}");
    println!("blocks compared: {total}");
    println!("agreement:       {:.3}%  ({agree}/{total})", 100.0 * agree as f64 / total as f64);

    println!(
        "\nsurface band (top {BAND_DEPTH} solid blocks — what surface rules govern):\n  \
         agreement:     {:.3}%  ({band_agree}/{band_total})",
        100.0 * band_agree as f64 / band_total as f64
    );

    let mut rows: Vec<_> = confusion.into_iter().collect();
    rows.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
    println!("\ntop whole-column disagreements (mine → oracle):");
    for ((got, want), n) in rows.iter().take(20) {
        println!("  {got:<22} → {want:<22} {n:>8}  ({:.3}%)", 100.0 * *n as f64 / total as f64);
    }

    let mut band_rows: Vec<_> = band_confusion.into_iter().collect();
    band_rows.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
    println!("\ntop surface-band disagreements (mine → oracle):");
    for ((got, want), n) in band_rows.iter().take(20) {
        println!("  {got:<22} → {want:<22} {n:>8}  ({:.3}%)", 100.0 * *n as f64 / band_total as f64);
    }
}

fn load_chunk(region_dir: &str, chunk_x: i32, chunk_z: i32) -> Option<CurrentJavaChunk> {
    let path = format!("{region_dir}/r.{}.{}.mca", chunk_x.div_euclid(32), chunk_z.div_euclid(32));
    let mut region = Region::from_stream(File::open(&path).ok()?).ok()?;
    let raw = region
        .read_chunk(chunk_x.rem_euclid(32) as usize, chunk_z.rem_euclid(32) as usize)
        .ok()??;
    from_bytes(&raw).ok()
}
