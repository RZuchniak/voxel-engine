//! Are trees being placed, where, and what do they cost?
//!
//! Three questions that have to be answered together. "No trees" and "trees in the wrong biomes"
//! and "trees are too expensive" all look the same from a screenshot of empty grassland, and the
//! decoration pass is a 3×3 neighbourhood walk whose cost is entirely down to whether the terrain
//! cache is working.
//!
//! Run: `cargo run --release --example diag_trees -- [radius] [seed]`

use std::collections::BTreeMap;
use std::time::Instant;

use voxel_engine::{
    block::BlockId,
    mc::decorate::tree_kind_for_biome,
    source::{SeededProceduralSource, WorldSource},
    world::{SECTION_COUNT, SECTION_SIZE},
};

/// Registry name for a tint-table biome index.
fn biome_name(index: u8) -> &'static str {
    voxel_engine::mc::biome::overworld_biomes()
        .into_iter()
        .map(|(_, n)| n)
        .find(|n| voxel_engine::biome_tint::biome_index(n) == index)
        .unwrap_or("?")
}

fn main() {
    let mut args = std::env::args().skip(1);
    let radius: i32 = args.next().map(|s| s.parse().unwrap()).unwrap_or(4);
    let seed: i64 = args
        .next()
        .map(|s| s.parse().unwrap())
        .unwrap_or(6954908675375307936);

    let t0 = Instant::now();
    let source = SeededProceduralSource::new(seed);
    println!("world setup: {:.1} ms\n", t0.elapsed().as_secs_f64() * 1000.0);

    // Per-chunk cost bucketed by how many trees the chunk ended up with. The reported symptom
    // is "chunks with a lot of trees generate noticeably slower", so an average is useless —
    // what matters is the spread between a bare chunk and a forest one.
    let mut buckets: BTreeMap<usize, (usize, f64, usize)> = BTreeMap::new();

    let mut logs = 0usize;
    let mut leaves = 0usize;
    let mut by_biome: BTreeMap<&'static str, usize> = BTreeMap::new();
    let mut chunks = 0usize;
    let mut chunks_with_trees = 0usize;

    // Generation time is accumulated per call. The biome attribution below is deliberately
    // expensive (it rebuilds the 7594-box biome list per column) and must not be counted as
    // generation cost — conflating them once reported 177 ms/chunk against a true 47.
    let mut generate_ms = 0f64;
    for cz in -radius..=radius {
        for cx in -radius..=radius {
            let t = Instant::now();
            let chunk = source.load_chunk((cx, cz)).expect("procedural gen cannot fail");
            let chunk_ms = t.elapsed().as_secs_f64() * 1000.0;
            generate_ms += chunk_ms;
            chunks += 1;

            let mut chunk_logs = 0usize;
            for index in 0..SECTION_COUNT {
                let Some(section) = chunk.section(index) else { continue };
                for block in section.block_data() {
                    match *block {
                        BlockId::OAK_LOG | BlockId::BIRCH_LOG | BlockId::SPRUCE_LOG => {
                            chunk_logs += 1
                        }
                        BlockId::OAK_LEAVES | BlockId::BIRCH_LEAVES | BlockId::SPRUCE_LEAVES => {
                            leaves += 1
                        }
                        _ => {}
                    }
                }
            }
            logs += chunk_logs;
            if chunk_logs > 0 {
                chunks_with_trees += 1;
            }

            // Mesh the chunk on its own to get its geometry cost. Borders will be slightly
            // over-counted (no neighbours), but identically for every chunk, so the *ratio*
            // between a bare chunk and a forest one — the thing being investigated — holds.
            let mut solo = voxel_engine::world::World::new();
            solo.insert_chunk(chunk);
            let quads: usize = voxel_engine::mesh::mesh_chunk(&solo, (cx, cz))
                .iter()
                .map(|(_, m)| m.indices().len() / 6)
                .sum();

            // Bucket by trunk count, roughly: logs/5 is about one tree.
            let bucket = (chunk_logs / 5).min(12);
            let entry = buckets.entry(bucket).or_insert((0, 0.0, 0));
            entry.0 += 1;
            entry.1 += chunk_ms;
            entry.2 += quads;

            // Attribute each **trunk base** to the biome of its own column.
            //
            // Per-column, and only trunk bases, because those are the two things that make this
            // check mean anything. `BiomeFilter` gates the position a tree is *rooted* at, not
            // the blocks it goes on to occupy — canopy legitimately overhangs a river, and a
            // chunk's biome grid legitimately spans several biomes. Attributing a whole chunk's
            // logs to its first biome cell (an earlier version of this diagnostic) reports both
            // of those as violations.
            let chunk = solo.chunk((cx, cz)).expect("just inserted");
            for lz in 0..SECTION_SIZE as i32 {
                for lx in 0..SECTION_SIZE as i32 {
                    let Some(biome_index) = chunk.biome_at_local(lx, lz) else { continue };
                    let name = biome_name(biome_index);
                    let mut previous_was_log = false;
                    for index in 0..SECTION_COUNT {
                        let Some(section) = chunk.section(index) else {
                            previous_was_log = false;
                            continue;
                        };
                        for ly in 0..SECTION_SIZE {
                            let block = section.block_at(lx as usize, ly, lz as usize);
                            let is_log = matches!(
                                block,
                                BlockId::OAK_LOG | BlockId::BIRCH_LOG | BlockId::SPRUCE_LOG
                            );
                            if is_log && !previous_was_log {
                                *by_biome.entry(name).or_default() += 1;
                            }
                            previous_was_log = is_log;
                        }
                    }
                }
            }
        }
    }
    let elapsed = generate_ms;

    println!("=== are there trees? ===");
    println!("chunks generated       : {chunks}");
    println!("chunks with any log    : {chunks_with_trees} ({:.0}%)", 100.0 * chunks_with_trees as f64 / chunks as f64);
    println!("log blocks             : {logs}");
    println!("leaf blocks            : {leaves}");
    println!("logs per chunk         : {:.2}", logs as f64 / chunks as f64);

    println!("\n=== trunk bases, by the biome of their own column ===");
    let mut violations = 0usize;
    for (biome, count) in &by_biome {
        let expects_trees = tree_kind_for_biome(biome).is_some();
        if !expects_trees && *count > 0 {
            violations += *count;
        }
        println!(
            "  {biome:28} {count:6}{}",
            if !expects_trees && *count > 0 {
                "   <-- VIOLATION: BiomeFilter should have rejected these"
            } else {
                ""
            }
        );
    }
    println!(
        "\n{}",
        if violations == 0 {
            "BiomeFilter holds: every trunk is rooted in a biome that grows trees.".to_string()
        } else {
            format!("{violations} trunks rooted in biomes that grow no trees — BiomeFilter is wrong.")
        }
    );

    println!("
=== cost vs tree density (the reported symptom) ===");
    println!("  {:>10}  {:>7}  {:>12}  {:>10}", "logs", "chunks", "gen ms/chunk", "quads");
    for (bucket, (count, ms, quads)) in &buckets {
        let label = if *bucket == 0 { "0-4".to_string() } else { format!("{}-{}", bucket * 5, bucket * 5 + 4) };
        println!(
            "  {label:>10}  {count:>7}  {:>12.1}  {:>10}",
            ms / *count as f64,
            quads / count
        );
    }

    let (hits, misses) = source.cache_stats();
    let (overlay_hits, overlay_misses) = source.overlay_stats();
    println!("\n=== what does it cost? ===");
    println!("generation total       : {elapsed:.0} ms for {chunks} chunks (analysis excluded)");
    println!("per chunk              : {:.1} ms", elapsed / chunks as f64);
    println!(
        "terrain cache          : {hits} hits / {misses} misses ({:.1}% hit rate)",
        100.0 * hits as f64 / (hits + misses).max(1) as f64
    );
    println!(
        "terrain generations    : {misses} for {chunks} chunks ({:.2}x — ~1.1x is a filled \
         square with the 5×5 overlay neighbourhood, 5×+ means the cache is not sharing work)",
        misses as f64 / chunks as f64
    );
    println!(
        "tree overlays          : {overlay_hits} hits / {overlay_misses} misses ({:.2}x — 1.00x \
         means each chunk's trees ran once)",
        overlay_misses as f64 / chunks as f64
    );

    // The browser does not look like the run above. Native shares ONE source — and therefore one
    // terrain cache — across the whole worker pool. wasm gives **every web worker its own**
    // `SeededProceduralSource`, and `worker_bridge::dispatch_next_jobs` hands chunks out
    // round-robin, so each worker sees every Nth chunk of the streaming spiral and has to
    // generate all nine of its neighbours itself. That difference is the whole reason one
    // platform runs fine and the other freezes, so it is simulated rather than assumed.
    println!("\n=== simulated browser dispatch (per-worker caches) ===");
    println!("  (rr = round-robin, tN = N-chunk affinity tiles)");
    for worker_count in [1usize, 4, 8] {
        let mut results = Vec::new();
        for tile in [0i32, 4, 8, 16] {
            let tiled = tile > 0;
            let sources: Vec<SeededProceduralSource> = (0..worker_count)
                .map(|_| SeededProceduralSource::new(seed))
                .collect();
            let mut dispatched = 0usize;
            // Ring-by-ring outward, which is how the streamer requests chunks.
            for ring in 0..=radius {
                for cz in -ring..=ring {
                    for cx in -ring..=ring {
                        if cx.abs() != ring && cz.abs() != ring {
                            continue;
                        }
                        // Must mirror `worker_bridge::WorkerPool::pick_worker`.
                        let worker = if tiled {
                            let tile_x = cx.div_euclid(tile) as i64;
                            let tile_z = cz.div_euclid(tile) as i64;
                            let hash = tile_x.wrapping_mul(0x9E37_79B9)
                                ^ tile_z.wrapping_mul(0x85EB_CA6B);
                            hash.rem_euclid(worker_count as i64) as usize
                        } else {
                            dispatched % worker_count
                        };
                        let _ = sources[worker].load_chunk((cx, cz));
                        dispatched += 1;
                    }
                }
            }
            let (h, m) = sources
                .iter()
                .map(|s| s.cache_stats())
                .fold((0u64, 0u64), |(ah, am), (h, m)| (ah + h, am + m));
            results.push(format!(
                "{:.1}% hit, {:.2}x gen",
                100.0 * h as f64 / (h + m).max(1) as f64,
                m as f64 / dispatched.max(1) as f64
            ));
        }
        println!(
            "  {worker_count:>8}  rr={:<20} t4={:<20} t8={:<20} t16={}",
            results[0], results[1], results[2], results[3]
        );
    }
    let _ = SECTION_SIZE;
}
