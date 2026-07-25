//! Biome parity against the real world — does `Overworld::biome_at` pick the same biome
//! Minecraft did, at the same quart cell?
//!
//! `examples/parity_biomes` already proves the climate *table* matches Mojang's datagen dump
//! exactly. What this checks is the other half: that the six climate coordinates fed into
//! the lookup (temperature, vegetation, continentalness, erosion, depth, weirdness) are the
//! right ones, sampled the right way. A swapped or mis-scaled axis would sail past the table
//! check and fail here.
//!
//! `fastanvil 0.32`'s `Biome` enum is pre-1.18 numeric IDs, so it can't read 26.2 biomes;
//! this parses the per-section biome palette out of the chunk NBT directly.
//!
//! Run: `cargo run --release --example parity_biome_world -- "<region dir>" [seed]`

use std::collections::BTreeMap;
use std::fs::File;

use fastanvil::Region;
use fastnbt::{from_bytes, LongArray};
use serde::Deserialize;
use voxel_engine::mc::overworld::Overworld;

const CHUNK_RADIUS: i32 = 6;

#[derive(Deserialize)]
struct ChunkNbt {
    #[serde(default)]
    sections: Vec<Section>,
}

#[derive(Deserialize)]
struct Section {
    #[serde(rename = "Y")]
    y: i8,
    biomes: Option<BiomePalette>,
}

/// The 4×4×4 biome grid for one 16-block section: a string palette plus (when the palette
/// has more than one entry) bit-packed indices into it.
#[derive(Deserialize)]
struct BiomePalette {
    palette: Vec<String>,
    data: Option<LongArray>,
}

impl BiomePalette {
    /// The biome at a quart cell within the section, `x4`/`y4`/`z4` each in `0..4`.
    fn at(&self, x4: usize, y4: usize, z4: usize) -> &str {
        if self.palette.len() == 1 {
            return &self.palette[0];
        }
        let Some(data) = &self.data else { return &self.palette[0] };
        // Same packing as block states: ceil(log2(len)) bits, entries never straddle a long.
        let bits = (usize::BITS - (self.palette.len() - 1).leading_zeros()).max(1) as usize;
        let per_long = 64 / bits;
        let index = (y4 * 4 + z4) * 4 + x4;
        let long = data.iter().nth(index / per_long).copied().unwrap_or(0) as u64;
        let shift = (index % per_long) * bits;
        let value = (long >> shift) & ((1u64 << bits) - 1);
        self.palette.get(value as usize).map(String::as_str).unwrap_or(&self.palette[0])
    }
}

fn main() {
    let mut args = std::env::args().skip(1);
    let region_dir = args.next().expect("usage: parity_biome_world <region dir> [seed]");
    let seed: i64 = args.next().map(|s| s.parse().unwrap()).unwrap_or(6954908675375307936);

    let ow = Overworld::new(seed);

    let mut total = 0u64;
    let mut agree = 0u64;
    let mut confusion: BTreeMap<(String, String), u64> = BTreeMap::new();

    for cx in -CHUNK_RADIUS..=CHUNK_RADIUS {
        for cz in -CHUNK_RADIUS..=CHUNK_RADIUS {
            let Some(chunk) = load_chunk(&region_dir, cx, cz) else { continue };
            for section in &chunk.sections {
                let Some(biomes) = &section.biomes else { continue };
                let section_min_y = section.y as i32 * 16;
                for y4 in 0..4 {
                    for z4 in 0..4 {
                        for x4 in 0..4 {
                            let block_x = cx * 16 + x4 as i32 * 4;
                            let block_y = section_min_y + y4 as i32 * 4;
                            let block_z = cz * 16 + z4 as i32 * 4;
                            // Only the range the generator actually produces.
                            if !(-64..320).contains(&block_y) {
                                continue;
                            }
                            let oracle = biomes.at(x4, y4, z4).trim_start_matches("minecraft:");
                            let mine = ow.biome_at(block_x, block_y, block_z);
                            total += 1;
                            if mine == oracle {
                                agree += 1;
                            } else {
                                *confusion
                                    .entry((mine.to_string(), oracle.to_string()))
                                    .or_default() += 1;
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

    println!("quart cells compared: {total}");
    println!("agreement:            {:.3}%  ({agree}/{total})", 100.0 * agree as f64 / total as f64);
    if !confusion.is_empty() {
        println!("\ndisagreements (mine → oracle):");
        let mut rows: Vec<_> = confusion.iter().collect();
        rows.sort_by_key(|&(_, n)| std::cmp::Reverse(*n));
        for ((mine, oracle), n) in rows.into_iter().take(15) {
            println!("  {mine:<28} → {oracle:<28} {n:>7}");
        }
    }
}

fn load_chunk(region_dir: &str, chunk_x: i32, chunk_z: i32) -> Option<ChunkNbt> {
    let path = format!("{region_dir}/r.{}.{}.mca", chunk_x.div_euclid(32), chunk_z.div_euclid(32));
    let mut region = Region::from_stream(File::open(&path).ok()?).ok()?;
    let raw = region
        .read_chunk(chunk_x.rem_euclid(32) as usize, chunk_z.rem_euclid(32) as usize)
        .ok()??;
    from_bytes(&raw).ok()
}
