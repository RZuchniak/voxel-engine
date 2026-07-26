//! Round-trip fidelity for the worker wire format.
//!
//! Chunks generated in a Web Worker cross to the main thread as `ChunkWire`. Nothing
//! else validates that hop, and a mistake there is invisible in Rust — it shows up as
//! subtly wrong terrain in the browser. These run natively because the wire types are
//! pure data.

use voxel_engine::{
    block::BlockId,
    source::{SeededProceduralSource, WorldSource},
    world::{Chunk, World, SECTION_SIZE},
    worker_protocol::ChunkWire,
};

/// Every block in `a` and `b` matches across the full world height.
fn assert_chunks_identical(a: &Chunk, b: &Chunk, context: &str) {
    let mut compared = 0usize;
    for section_index in 0..voxel_engine::world::SECTION_COUNT {
        let base_y = (section_index as i32 + voxel_engine::world::MIN_SECTION_Y)
            * SECTION_SIZE as i32;
        for y in 0..SECTION_SIZE as i32 {
            for z in 0..SECTION_SIZE as i32 {
                for x in 0..SECTION_SIZE as i32 {
                    let wy = base_y + y;
                    let lhs = a.block_at_local(x, wy, z);
                    let rhs = b.block_at_local(x, wy, z);
                    assert_eq!(
                        lhs, rhs,
                        "{context}: block mismatch at local ({x},{wy},{z})"
                    );
                    compared += 1;
                }
            }
        }
    }
    assert!(compared > 0);
}

#[test]
fn chunk_wire_roundtrip_preserves_every_block() {
    let source = SeededProceduralSource::new(12345);
    for coord in [(0, 0), (3, -2), (-7, 5)] {
        let original = source.load_chunk(coord).expect("procedural gen cannot fail");
        let restored = ChunkWire::from_chunk(&original).into_chunk();
        assert_chunks_identical(&original, &restored, &format!("coord {coord:?}"));
    }
}

/// Directly pins the axis order. A Y/Z transposition in the wire decode previously
/// tilted every worker-delivered chunk on its side, and a whole-chunk equality check on
/// symmetric terrain can miss that — this cannot.
#[test]
fn chunk_wire_roundtrip_does_not_transpose_axes() {
    let mut chunk = Chunk::new((0, 0));
    // Asymmetric in all three axes so any swap changes the result.
    chunk.set_block_world(1, 0, 0, BlockId::STONE);
    chunk.set_block_world(0, 5, 0, BlockId::DIRT);
    chunk.set_block_world(0, 0, 9, BlockId::SAND);

    let restored = ChunkWire::from_chunk(&chunk).into_chunk();

    assert_eq!(restored.block_at_local(1, 0, 0), BlockId::STONE, "x axis moved");
    assert_eq!(restored.block_at_local(0, 5, 0), BlockId::DIRT, "y axis moved");
    assert_eq!(restored.block_at_local(0, 0, 9), BlockId::SAND, "z axis moved");
}

/// The worker regenerates terrain from the seed rather than receiving it, so a chunk
/// produced worker-side must equal one produced main-thread-side.
#[test]
fn worker_side_generation_matches_main_thread() {
    let coord = (2, -1);
    let main_thread = SeededProceduralSource::new(777)
        .load_chunk(coord)
        .expect("procedural gen cannot fail");

    // Same seed, independent generator — this is what each worker constructs.
    let worker_side = SeededProceduralSource::new(777)
        .load_chunk(coord)
        .expect("procedural gen cannot fail");
    let delivered = ChunkWire::from_chunk(&worker_side).into_chunk();

    assert_chunks_identical(&main_thread, &delivered, "worker vs main thread");
}

/// Meshing a wire-roundtripped chunk must produce the same geometry as the original.
#[test]
fn roundtripped_chunk_meshes_identically() {
    let source = SeededProceduralSource::new(4242);
    let coord = (0, 0);

    let mut direct = World::new();
    let mut viaworker = World::new();
    for (dx, dz) in [(0, 0), (1, 0), (-1, 0), (0, 1), (0, -1)] {
        let c = (coord.0 + dx, coord.1 + dz);
        let chunk = source.load_chunk(c).expect("procedural gen cannot fail");
        viaworker.insert_chunk(ChunkWire::from_chunk(&chunk).into_chunk());
        direct.insert_chunk(chunk);
    }

    let a = voxel_engine::mesh::mesh_chunk(&direct, coord);
    let b = voxel_engine::mesh::mesh_chunk(&viaworker, coord);

    assert_eq!(a.len(), b.len(), "section count differs");
    for ((ai, am), (bi, bm)) in a.iter().zip(b.iter()) {
        assert_eq!(ai, bi, "section index differs");
        assert_eq!(am.to_wire_bytes(), bm.to_wire_bytes(), "mesh bytes differ");
    }
}
