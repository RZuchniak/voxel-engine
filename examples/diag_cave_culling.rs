//! Why does the world look empty from underground?
//!
//! Builds a real generated neighbourhood, computes each chunk's `VisibilitySet`, and runs the
//! section occlusion walk from a camera placed in a cave. Reports how many sections with actual
//! geometry the walk reaches, versus how many are there.
//!
//! Run: `cargo run --release --example diag_cave_culling`

use std::collections::HashMap;

use voxel_engine::cull::SectionGraph;
use voxel_engine::mesh::mesh_chunk;
use voxel_engine::source::{SeededProceduralSource, WorldSource};
use voxel_engine::visibility::{chunk_visibility, VisibilitySet, FACINGS};
use voxel_engine::world::{World, MIN_SECTION_Y, SECTION_COUNT, SECTION_SIZE};

const SEED: i64 = 6954908675375307936;

fn section_index_of(world_y: i32) -> usize {
    (world_y.div_euclid(SECTION_SIZE as i32) - MIN_SECTION_Y).clamp(0, SECTION_COUNT as i32 - 1)
        as usize
}

fn open_face_count(set: &VisibilitySet) -> usize {
    FACINGS
        .iter()
        .filter(|&&a| FACINGS.iter().any(|&b| set.connects(a, b)))
        .count()
}

fn main() {
    let radius: i32 = std::env::args()
        .nth(1)
        .and_then(|a| a.parse().ok())
        .unwrap_or(4);
    let center = (0i32, 0i32);

    let source = SeededProceduralSource::new(SEED);
    let mut world = World::new();
    println!("generating {} chunks...", (2 * radius + 1).pow(2));
    for dz in -radius..=radius {
        for dx in -radius..=radius {
            let coord = (center.0 + dx, center.1 + dz);
            world.insert_chunk(source.load_chunk(coord).expect("generate"));
        }
    }

    // Visibility + geometry per section.
    let mut visibility: HashMap<(i32, i32), [VisibilitySet; SECTION_COUNT]> = HashMap::new();
    let mut has_geometry: HashMap<((i32, i32), usize), usize> = HashMap::new();
    for dz in -radius..=radius {
        for dx in -radius..=radius {
            let coord = (center.0 + dx, center.1 + dz);
            visibility.insert(coord, chunk_visibility(world.chunk(coord).unwrap()));
            for (idx, mesh) in mesh_chunk(&world, coord) {
                has_geometry.insert((coord, idx), mesh.indices().len() / 6);
            }
        }
    }
    let total_geo_sections = has_geometry.len();
    let total_geo_quads: usize = has_geometry.values().sum();

    // Find the surface of the centre column, then a genuinely enclosed cave beneath it:
    // air with solid rock overhead, so it is a cave and not just the sky.
    let chunk = world.chunk(center).unwrap();
    let surface_y = chunk.max_nonempty_world_y().unwrap_or(64);
    let mut cave = None;
    'search: for lz in 0..16i32 {
        for lx in 0..16i32 {
            for y in -50..surface_y - 30 {
                let roofed = (1..24).any(|d| !chunk.block_at_local(lx, y + d, lz).is_air());
                if chunk.block_at_local(lx, y, lz).is_air()
                    && chunk.block_at_local(lx, y + 1, lz).is_air()
                    && !chunk.block_at_local(lx, y - 1, lz).is_air()
                    && roofed
                {
                    cave = Some((lx, y, lz));
                    break 'search;
                }
            }
        }
    }
    let cave_y = cave.map(|c| c.1);
    println!("centre chunk surface y = {surface_y}, enclosed cave found at {cave:?}");
    println!(
        "sections with geometry in the volume: {total_geo_sections} ({total_geo_quads} quads)\n"
    );

    let cases: Vec<(&str, i32)> = vec![
        ("above ground", surface_y + 20),
        ("just below surface", surface_y - 20),
        ("in a cave", cave_y.unwrap_or(0)),
        ("deep (y=-40)", -40),
        ("y=0", 0),
    ];

    let cave_xz = cave.map(|c| (c.0, c.2)).unwrap_or((8, 8));
    let mut graph = SectionGraph::new();
    for (label, cam_y) in cases {
        let origin = (center, section_index_of(cam_y));
        let (lx, lz) = if label == "in a cave" { cave_xz } else { (8, 8) };
        let cam_block = chunk.block_at_local(lx, cam_y, lz);
        // What the renderer now decides: connectivity is meaningless inside an opaque block.
        let smart_cull = !(cam_block.is_opaque() && cam_block.is_full_cube());
        let origin_set = visibility[&center][origin.1];
        println!(
            "{label:<20} y={cam_y:<5} section={:<3} camera block={cam_block:?} \
             open faces of camera section={} smart_cull={smart_cull}",
            origin.1,
            open_face_count(&origin_set)
        );

        for mode in [true, false] {
            let mut reached = Vec::new();
            graph.walk(
                origin,
                radius,
                mode,
                |key| {
                    visibility
                        .get(&key.0)
                        .map(|s| s[key.1])
                        .unwrap_or(VisibilitySet::EMPTY)
                },
                |_| true,
                |key| reached.push(key),
            );
            let drawn = reached
                .iter()
                .filter(|k| has_geometry.contains_key(k))
                .count();
            let quads: usize = reached.iter().filter_map(|k| has_geometry.get(k)).sum();
            println!(
                "  smart_cull={mode:<5} reached {:<5} sections, {drawn:<4} with geometry \
                 ({quads} quads = {:.1}% of the volume){}",
                reached.len(),
                100.0 * quads as f64 / total_geo_quads.max(1) as f64,
                if mode == smart_cull { "   <- used" } else { "" }
            );
        }
    }
}
