//! Deterministic work-volume baseline for seed generation + meshing.
//!
//! These counters are machine-independent: they measure how much work the engine does,
//! not how fast it does it. Timing is deliberately excluded — it varies per machine and
//! would flake. An optimization that is meant to preserve behaviour must not move these
//! numbers; one that intentionally changes terrain will move them, and the diff is the
//! review artifact.

use std::collections::BTreeMap;

use voxel_engine::{
    mesh::mesh_chunk_surface,
    source::{SeededProceduralSource, WorldSource},
    world::World,
};

const SEED: i64 = 12345;

/// Chunks meshed for the baseline. Their 4 neighbours are also generated so face culling
/// at chunk borders is exercised.
const MEASURED: &[(i32, i32)] = &[(0, 0), (1, 0), (0, 1), (-1, -1), (5, -3)];

#[derive(Debug, Default, PartialEq, Eq)]
struct Counters {
    chunks_generated: usize,
    sections_meshed: usize,
    quads: usize,
    vertices: usize,
    indices: usize,
    mesh_bytes: usize,
}

fn collect(seed: i64) -> Counters {
    let source = SeededProceduralSource::new(seed);
    let mut world = World::new();
    let mut counters = Counters::default();

    // Generate every measured chunk plus its orthogonal neighbours.
    let mut needed: Vec<(i32, i32)> = Vec::new();
    for &(cx, cz) in MEASURED {
        for (dx, dz) in [(0, 0), (1, 0), (-1, 0), (0, 1), (0, -1)] {
            let coord = (cx + dx, cz + dz);
            if !needed.contains(&coord) {
                needed.push(coord);
            }
        }
    }
    for coord in &needed {
        let chunk = source.load_chunk(*coord).expect("procedural gen cannot fail");
        world.insert_chunk(chunk);
        counters.chunks_generated += 1;
    }

    for &coord in MEASURED {
        for (_, mesh) in mesh_chunk_surface(&world, coord) {
            counters.sections_meshed += 1;
            counters.quads += mesh.indices().len() / 6;
            counters.vertices += mesh.vertices().len();
            counters.indices += mesh.indices().len();
            let (v, i) = mesh.to_wire_bytes();
            counters.mesh_bytes += v.len() + i.len();
        }
    }
    counters
}

#[test]
fn seed_generation_is_deterministic_across_runs() {
    assert_eq!(
        collect(SEED),
        collect(SEED),
        "same seed must produce identical work volume"
    );
}

#[test]
fn different_seeds_produce_different_terrain() {
    assert_ne!(collect(SEED).quads, collect(SEED + 991).quads);
}

#[test]
fn work_volume_baseline() {
    let actual = collect(SEED);

    // Recorded on the native tuning profile. Print actuals with:
    //     cargo test --test perf_baseline -- --nocapture
    //
    // Invariants worth knowing when reading a diff here:
    //   vertices == quads * 4, indices == quads * 6
    //   mesh_bytes == vertices * size_of::<Vertex>() + indices * 4   (Vertex is 28 bytes)
    // so a vertex-format change moves mesh_bytes alone, while a terrain change moves quads
    // and everything downstream of it.
    let expected = Counters {
        chunks_generated: 18,
        sections_meshed: 8,
        quads: 371,
        vertices: 1484,
        indices: 2226,
        mesh_bytes: 50456,
    };

    let mut report: BTreeMap<&str, (usize, usize)> = BTreeMap::new();
    report.insert("chunks_generated", (expected.chunks_generated, actual.chunks_generated));
    report.insert("sections_meshed", (expected.sections_meshed, actual.sections_meshed));
    report.insert("quads", (expected.quads, actual.quads));
    report.insert("vertices", (expected.vertices, actual.vertices));
    report.insert("indices", (expected.indices, actual.indices));
    report.insert("mesh_bytes", (expected.mesh_bytes, actual.mesh_bytes));
    println!("=== work volume (expected, actual) ===");
    for (k, (e, a)) in &report {
        println!("{k:20} expected={e:<10} actual={a}");
    }

    assert_eq!(actual, expected, "work volume changed — review the diff");
}
