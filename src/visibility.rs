//! Which faces of a chunk section can see through to which other faces.
//!
//! This is the data behind Minecraft's cave culling. A 16³ section is opaque rock almost
//! everywhere underground, so the renderer can skip enormous amounts of the world if it only
//! walks *through* sections along paths that actually exist. For each section we flood-fill the
//! non-opaque cells and record, for every pair of the six faces, whether some connected pocket
//! of air touches both. A solid section connects nothing; an empty one connects everything.
//!
//! The traversal that consumes this lives in [`crate::cull`]. Vanilla's equivalents are
//! `VisibilitySet` (this) and `SectionOcclusionGraph` (that).

use crate::world::{Chunk, Section, SECTION_COUNT, SECTION_SIZE};

/// One of the six axis-aligned faces of a section, and the direction that leaves through it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Facing {
    NegX = 0,
    PosX = 1,
    NegY = 2,
    PosY = 3,
    NegZ = 4,
    PosZ = 5,
}

pub const FACINGS: [Facing; 6] = [
    Facing::NegX,
    Facing::PosX,
    Facing::NegY,
    Facing::PosY,
    Facing::NegZ,
    Facing::PosZ,
];

impl Facing {
    #[inline]
    pub fn index(self) -> usize {
        self as usize
    }

    #[inline]
    pub fn opposite(self) -> Facing {
        match self {
            Facing::NegX => Facing::PosX,
            Facing::PosX => Facing::NegX,
            Facing::NegY => Facing::PosY,
            Facing::PosY => Facing::NegY,
            Facing::NegZ => Facing::PosZ,
            Facing::PosZ => Facing::NegZ,
        }
    }

    /// Step to the neighbouring section, in (x, section, z) units.
    #[inline]
    pub fn offset(self) -> (i32, i32, i32) {
        match self {
            Facing::NegX => (-1, 0, 0),
            Facing::PosX => (1, 0, 0),
            Facing::NegY => (0, -1, 0),
            Facing::PosY => (0, 1, 0),
            Facing::NegZ => (0, 0, -1),
            Facing::PosZ => (0, 0, 1),
        }
    }

    #[inline]
    fn bit(self) -> u8 {
        1 << (self as u8)
    }
}

/// `reach[face]` is the set of faces reachable from `face` through non-opaque blocks.
///
/// Symmetric by construction, and every face reaches itself when it has any non-opaque cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VisibilitySet {
    reach: [u8; 6],
}

impl VisibilitySet {
    /// Nothing is visible through this section.
    pub const OPAQUE: Self = Self { reach: [0; 6] };

    /// Everything is visible through this section. Used for sections with no block data at
    /// all, and as the conservative answer whenever the real set is not known yet — guessing
    /// "connected" can only draw too much, while guessing "blocked" would hide real geometry.
    pub const EMPTY: Self = Self { reach: [0b0011_1111; 6] };

    #[inline]
    pub fn connects(&self, from: Facing, to: Facing) -> bool {
        self.reach[from.index()] & to.bit() != 0
    }

    /// Flood-fill the section's non-opaque cells and record which faces each pocket touches.
    ///
    /// Cost is one pass over the 4096 cells plus the fill itself, so it is bounded by the
    /// section volume no matter how convoluted the caves are.
    pub fn from_section(section: &Section) -> Self {
        const N: usize = SECTION_SIZE;
        const VOLUME: usize = N * N * N;

        let blocks = section.block_data();
        let mut open = [false; VOLUME];
        let mut any_open = false;
        for (i, block) in blocks.iter().enumerate() {
            // Matches the mesher's occlusion rule: only a full opaque cube blocks sight, so
            // water and leaves let the traversal through, as they do in vanilla.
            let blocking = block.is_opaque() && block.is_full_cube();
            open[i] = !blocking;
            any_open |= open[i];
        }
        if !any_open {
            return Self::OPAQUE;
        }

        let mut reach = [0u8; 6];
        let mut visited = [false; VOLUME];
        let mut stack: Vec<u16> = Vec::new();

        for start in 0..VOLUME {
            if !open[start] || visited[start] {
                continue;
            }
            // One connected pocket of non-opaque cells; collect every face it touches.
            let mut touched = 0u8;
            visited[start] = true;
            stack.push(start as u16);
            while let Some(index) = stack.pop() {
                let index = index as usize;
                let x = index % N;
                let y = (index / N) % N;
                let z = index / (N * N);

                if x == 0 {
                    touched |= Facing::NegX.bit();
                }
                if x == N - 1 {
                    touched |= Facing::PosX.bit();
                }
                if y == 0 {
                    touched |= Facing::NegY.bit();
                }
                if y == N - 1 {
                    touched |= Facing::PosY.bit();
                }
                if z == 0 {
                    touched |= Facing::NegZ.bit();
                }
                if z == N - 1 {
                    touched |= Facing::PosZ.bit();
                }

                let mut push = |nx: usize, ny: usize, nz: usize, stack: &mut Vec<u16>| {
                    let n = nx + ny * N + nz * N * N;
                    if open[n] && !visited[n] {
                        visited[n] = true;
                        stack.push(n as u16);
                    }
                };
                if x > 0 {
                    push(x - 1, y, z, &mut stack);
                }
                if x < N - 1 {
                    push(x + 1, y, z, &mut stack);
                }
                if y > 0 {
                    push(x, y - 1, z, &mut stack);
                }
                if y < N - 1 {
                    push(x, y + 1, z, &mut stack);
                }
                if z > 0 {
                    push(x, y, z - 1, &mut stack);
                }
                if z < N - 1 {
                    push(x, y, z + 1, &mut stack);
                }
            }

            // Every face this pocket reached can see every other face it reached.
            for face in FACINGS {
                if touched & face.bit() != 0 {
                    reach[face.index()] |= touched;
                }
            }
        }

        Self { reach }
    }
}

/// Every section of a chunk, for the renderer's traversal.
///
/// Sections with no block data are [`VisibilitySet::EMPTY`] — there is nothing there to block
/// a sightline. Computed on the mesh worker pool, since it walks the same block arrays the
/// mesher just walked and is far too expensive for the main thread.
pub fn chunk_visibility(chunk: &Chunk) -> [VisibilitySet; SECTION_COUNT] {
    std::array::from_fn(|index| match chunk.section(index) {
        Some(section) => VisibilitySet::from_section(section),
        None => VisibilitySet::EMPTY,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block::BlockId;
    use crate::world::Section;

    fn filled(block: BlockId) -> Section {
        let mut section = Section::new();
        for z in 0..SECTION_SIZE {
            for y in 0..SECTION_SIZE {
                for x in 0..SECTION_SIZE {
                    section.set_block(x, y, z, block);
                }
            }
        }
        section
    }

    fn all_pairs_connected(set: &VisibilitySet) -> bool {
        FACINGS
            .iter()
            .all(|&a| FACINGS.iter().all(|&b| set.connects(a, b)))
    }

    #[test]
    fn empty_section_connects_everything() {
        let set = VisibilitySet::from_section(&Section::new());
        assert!(all_pairs_connected(&set));
    }

    #[test]
    fn solid_section_connects_nothing() {
        let set = VisibilitySet::from_section(&filled(BlockId::STONE));
        assert_eq!(set, VisibilitySet::OPAQUE);
        assert!(!set.connects(Facing::NegX, Facing::PosX));
    }

    #[test]
    fn water_does_not_block_sight() {
        // Water is a full cube but not opaque, so a flooded section still sees through —
        // the same rule the mesher uses when deciding whether to cull a face.
        let set = VisibilitySet::from_section(&filled(BlockId::WATER));
        assert!(all_pairs_connected(&set));
    }

    #[test]
    fn a_floor_separates_top_from_bottom() {
        // Solid slab across the middle: the two halves are separate pockets, so you can enter
        // the bottom and leave through its sides, but never reach the top face. This is the
        // property that makes the traversal skip everything under the ground.
        let mut section = filled(BlockId::STONE);
        for z in 0..SECTION_SIZE {
            for x in 0..SECTION_SIZE {
                for y in 0..SECTION_SIZE {
                    if y != 8 {
                        section.set_block(x, y, z, BlockId::AIR);
                    }
                }
            }
        }
        let set = VisibilitySet::from_section(&section);

        assert!(set.connects(Facing::NegY, Facing::PosX), "bottom reaches its own sides");
        assert!(set.connects(Facing::PosY, Facing::PosX), "top reaches its own sides");
        assert!(
            !set.connects(Facing::NegY, Facing::PosY),
            "the slab must separate the two halves"
        );
    }

    #[test]
    fn a_tunnel_connects_only_the_faces_it_opens() {
        // A single 1x1 bore along X through solid rock.
        let mut section = filled(BlockId::STONE);
        for x in 0..SECTION_SIZE {
            section.set_block(x, 8, 8, BlockId::AIR);
        }
        let set = VisibilitySet::from_section(&section);

        assert!(set.connects(Facing::NegX, Facing::PosX));
        assert!(!set.connects(Facing::NegX, Facing::PosY));
        assert!(!set.connects(Facing::NegZ, Facing::PosZ));
    }

    #[test]
    fn connectivity_is_symmetric() {
        let mut section = filled(BlockId::STONE);
        for x in 0..SECTION_SIZE {
            section.set_block(x, 3, 3, BlockId::AIR);
        }
        for z in 0..SECTION_SIZE {
            section.set_block(5, 3, z, BlockId::AIR);
        }
        let set = VisibilitySet::from_section(&section);
        for a in FACINGS {
            for b in FACINGS {
                assert_eq!(
                    set.connects(a, b),
                    set.connects(b, a),
                    "{a:?} <-> {b:?} disagreed"
                );
            }
        }
    }
}
