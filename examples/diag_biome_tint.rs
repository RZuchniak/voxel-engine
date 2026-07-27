//! What does biome tinting actually cost, and is it reaching the mesh at all?
//!
//! Two questions, and they need each other. Breaking greedy runs where the biome changes costs
//! quads; but a run of "0.0% extra quads" is also exactly what you would see if the tint never
//! reached the mesher and every face silently resolved to `NEUTRAL`. So this measures the cost
//! **and** proves the feature is live, over the same chunks.
//!
//! The A/B is exact rather than estimated: the same generated chunks are meshed twice, once as
//! generated and once with `Chunk::biomes` stripped — which is precisely the pre-tint behaviour,
//! since an absent grid makes every face resolve to `NEUTRAL`.
//!
//! Run: `cargo run --release --example diag_biome_tint -- [radius] [seed]`

use std::collections::{BTreeMap, BTreeSet};

use voxel_engine::{
    biome_tint,
    mesh::{mesh_chunk, TINT_MASK, TINT_SHIFT},
    source::{SeededProceduralSource, WorldSource},
    world::{Chunk, World},
};

/// Rebuild a chunk without its biome grid — byte for byte what the mesher saw before tinting.
fn strip_biomes(chunk: &Chunk) -> Chunk {
    let mut out = Chunk::new(chunk.coord());
    for index in chunk.populated_section_indices() {
        if let Some(section) = chunk.section(index) {
            out.insert_section(index, section.clone());
        }
    }
    out
}

fn mesh_stats(world: &World, coords: &[(i32, i32)]) -> (usize, BTreeMap<u8, usize>) {
    let mut quads = 0usize;
    let mut by_tint: BTreeMap<u8, usize> = BTreeMap::new();
    for &coord in coords {
        for (_, mesh) in mesh_chunk(world, coord) {
            quads += mesh.indices().len() / 6;
            for vertex in mesh.vertices() {
                let tint = ((vertex.light >> TINT_SHIFT) & TINT_MASK) as u8;
                *by_tint.entry(tint).or_default() += 1;
            }
        }
    }
    (quads, by_tint)
}

fn main() {
    let mut args = std::env::args().skip(1);
    let radius: i32 = args.next().map(|s| s.parse().unwrap()).unwrap_or(4);
    let seed: i64 = args
        .next()
        .map(|s| s.parse().unwrap())
        .unwrap_or(6954908675375307936);

    let source = SeededProceduralSource::new(seed);

    // Generate a square, then measure only the interior so every measured chunk has all four
    // neighbours present and border face culling is exercised the same way in both passes.
    let mut tinted = World::new();
    let mut plain = World::new();
    let mut measured: Vec<(i32, i32)> = Vec::new();
    let mut biomes_seen: BTreeSet<u8> = BTreeSet::new();

    for cz in -radius..=radius {
        for cx in -radius..=radius {
            let chunk = source.load_chunk((cx, cz)).expect("procedural gen cannot fail");
            if let Some(grid) = chunk.biomes() {
                biomes_seen.extend(grid.iter().copied());
            }
            plain.insert_chunk(strip_biomes(&chunk));
            tinted.insert_chunk(chunk);
            if cx.abs() < radius && cz.abs() < radius {
                measured.push((cx, cz));
            }
        }
    }

    println!("seed {seed}, radius {radius} ({} chunks measured)\n", measured.len());

    let (plain_quads, plain_tints) = mesh_stats(&plain, &measured);
    let (tinted_quads, tinted_tints) = mesh_stats(&tinted, &measured);

    println!("=== is the tint reaching the mesh? ===");
    println!("distinct biomes in the generated area  : {}", biomes_seen.len());
    // Stripping biomes does not mean "no tint" — it falls back to `DEFAULT_BIOME`, so the world
    // takes one uniform colour. That is still the right control for the quad A/B below (one
    // colour everywhere means no tint-driven merge breaks), but it is not a count of 1.
    println!(
        "distinct tint indices, biomes stripped : {}  (one biome's worth — the old uniform look)",
        plain_tints.len()
    );
    println!(
        "distinct tint indices, as generated    : {}",
        tinted_tints.len()
    );
    let tinted_vertices: usize = tinted_tints
        .iter()
        .filter(|(index, _)| **index != biome_tint::NEUTRAL)
        .map(|(_, count)| *count)
        .sum();
    let all_vertices: usize = tinted_tints.values().sum();
    println!(
        "vertices carrying a non-neutral tint    : {tinted_vertices}/{all_vertices} ({:.2}%)",
        100.0 * tinted_vertices as f64 / all_vertices.max(1) as f64
    );

    println!("\n=== what does it cost? ===");
    println!("quads, biomes stripped (the old behaviour) : {plain_quads}");
    println!("quads, as generated                        : {tinted_quads}");
    let delta = tinted_quads as f64 - plain_quads as f64;
    println!(
        "delta                                      : {:+} ({:+.3}%)",
        tinted_quads as i64 - plain_quads as i64,
        100.0 * delta / plain_quads.max(1) as f64
    );
    println!(
        "\nVertex is still {} bytes — the tint rides in spare bits of `light`, so resident mesh\n\
         memory moves only with the quad delta above.",
        std::mem::size_of::<voxel_engine::Vertex>()
    );
}
