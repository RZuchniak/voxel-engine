//! Measures the two ways a streamed chunk's mesh can differ from the truth:
//! border faces emitted against a not-yet-loaded neighbour, and the bottom of the meshed
//! band being exposed because the chunk data stops there too.
//!
//! Run: `cargo run --release --example diag_chunk_borders`

use voxel_engine::mesh::mesh_chunk;
use voxel_engine::platform;
use voxel_engine::source::{SeededProceduralSource, WorldSource};
use voxel_engine::world::{Chunk, MIN_SECTION_Y, SECTION_COUNT, SECTION_SIZE, World};

const SEED: i64 = 6954908675375307936;

fn vertex_count(meshes: &[(usize, voxel_engine::mesh::MeshData)]) -> usize {
    meshes.iter().map(|(_, m)| m.vertices().len()).sum()
}

/// Quads whose four corners share one Y and which face downward, i.e. the underside of the
/// meshed band. `expand_corners` nudges them below the integer plane, hence the epsilon.
fn downward_quads(meshes: &[(usize, voxel_engine::mesh::MeshData)]) -> usize {
    let mut count = 0;
    for (_, mesh) in meshes {
        for quad in mesh.vertices().chunks(4) {
            if quad.len() < 4 {
                continue;
            }
            let y = quad[0].position[1];
            if quad.iter().all(|v| (v.position[1] - y).abs() < 1e-4) && y.fract().abs() > 0.5 {
                // fractional part near .998 => nudged down from an integer plane
                count += 1;
            }
        }
    }
    count
}

fn data_extent(chunk: &Chunk) -> (i32, i32) {
    let mut lo = i32::MAX;
    let mut hi = i32::MIN;
    for idx in 0..SECTION_COUNT {
        if chunk.section(idx).is_some_and(|s| s.has_any_non_air()) {
            let base = (idx as i32 + MIN_SECTION_Y) * SECTION_SIZE as i32;
            lo = lo.min(base);
            hi = hi.max(base + SECTION_SIZE as i32 - 1);
        }
    }
    (lo, hi)
}

/// What a chunk costs if the band is dropped: generate the whole -64..320 column and mesh
/// every populated section, rather than only the ones near the surface.
fn full_depth_quads(chunk_x: i32, chunk_z: i32) -> (usize, usize) {
    use voxel_engine::mc::chunk::{generate_chunk, HEIGHT, MIN_Y};
    use voxel_engine::mc::overworld::Overworld;
    use voxel_engine::mc::surface::SurfaceSystem;

    let ow = Overworld::new(SEED);
    let surface = SurfaceSystem::new(SEED);
    let generated = generate_chunk(&ow, &surface, chunk_x, chunk_z);

    let mut chunk = Chunk::new((chunk_x, chunk_z));
    for lz in 0..16usize {
        for lx in 0..16usize {
            for y in MIN_Y..MIN_Y + HEIGHT {
                let block = generated.get(lx, y, lz);
                if block != voxel_engine::mc::surface::Block::Air {
                    chunk.set_block_world(lx, y, lz, block.block_id());
                }
            }
        }
    }
    let sections = chunk.populated_section_indices().count();

    let mut world = World::new();
    world.insert_chunk(chunk);
    let quads: usize = (0..SECTION_COUNT)
        .filter_map(|idx| voxel_engine::mesh::mesh_section(&world, (chunk_x, chunk_z), idx))
        .map(|mesh| mesh.indices().len() / 6)
        .sum();
    (sections, quads)
}

fn main() {
    let source = SeededProceduralSource::new(SEED);
    let centers = [(-17, 7), (-16, 8), (0, 0), (12, -5)];

    println!(
        "mesh depth = {} blocks, generation depth = {} blocks\n",
        platform::surface_band_depth_blocks(),
        platform::surface_band_depth_blocks() + 16
    );

    for center in centers {
        let mut world = World::new();
        for (dx, dz) in [(0, 0), (1, 0), (-1, 0), (0, 1), (0, -1)] {
            let coord = (center.0 + dx, center.1 + dz);
            world.insert_chunk(source.load_chunk(coord).expect("generate"));
        }

        let mut alone = World::new();
        alone.insert_chunk(source.load_chunk(center).expect("generate"));

        let with_neighbours = mesh_chunk(&world, center);
        let without = mesh_chunk(&alone, center);

        let chunk = world.chunk(center).expect("center chunk");
        let (data_lo, data_hi) = data_extent(chunk);
        let top = chunk.max_nonempty_world_y().unwrap_or(i32::MIN);

        // Lowest section the mesher actually emitted geometry for.
        let mesh_lo = with_neighbours
            .iter()
            .map(|(idx, _)| (*idx as i32 + MIN_SECTION_Y) * SECTION_SIZE as i32)
            .min()
            .unwrap_or(0);

        let v_with = vertex_count(&with_neighbours);
        let v_without = vertex_count(&without);
        let extra = v_without as f64 / v_with.max(1) as f64;

        println!("chunk {center:?}");
        println!("  data y {data_lo}..{data_hi}  (top block {top})");
        println!("  meshed from y {mesh_lo}  => {} blocks of data below the mesh floor", mesh_lo - data_lo);
        println!(
            "  vertices: {v_with} with neighbours, {v_without} alone  ({extra:.2}x, {} extra border verts)",
            v_without - v_with
        );
        println!("  downward-facing quads in mesh: {}", downward_quads(&with_neighbours));

        let banded_quads: usize = with_neighbours
            .iter()
            .map(|(_, m)| m.indices().len() / 6)
            .sum();
        let banded_sections = with_neighbours.len();
        let (full_sections, full_quads) = full_depth_quads(center.0, center.1);
        println!(
            "  banded: {banded_sections} sections, {banded_quads} quads  ->  full depth: \
             {full_sections} sections, {full_quads} quads ({:.1}x)",
            full_quads as f64 / banded_quads.max(1) as f64
        );
        println!();
    }
}
