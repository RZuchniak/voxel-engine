#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BlockId(pub u16);

#[derive(Debug, Clone, Copy)]
pub enum Face {
    Top,
    Bottom,
    Side,
}

impl BlockId {
    pub const AIR: BlockId = BlockId(0);
    pub const STONE: BlockId = BlockId(1);
    pub const DIRT: BlockId = BlockId(2);
    pub const GRASS: BlockId = BlockId(3);
    pub const SAND: BlockId = BlockId(4);
    pub const COBBLESTONE: BlockId = BlockId(5);
    pub const OAK_LOG: BlockId = BlockId(6);
    pub const OAK_LEAVES: BlockId = BlockId(7);
    pub const OAK_PLANKS: BlockId = BlockId(8);
    pub const WATER: BlockId = BlockId(9);
    pub const BEDROCK: BlockId = BlockId(10);
    pub const DEEPSLATE: BlockId = BlockId(11);
    pub const GRAVEL: BlockId = BlockId(12);
    pub const SNOW_BLOCK: BlockId = BlockId(13);
    pub const NETHERRACK: BlockId = BlockId(14);
    pub const END_STONE: BlockId = BlockId(15);

    #[inline]
    pub fn info(self) -> &'static BlockInfo {
        let idx = self.0 as usize;
        if idx < BLOCK_TABLE.len() {
            &BLOCK_TABLE[idx]
        } else {
            &BLOCK_TABLE[0]
        }
    }

    #[inline]
    pub fn is_air(self) -> bool {
        self == BlockId::AIR
    }

    #[inline]
    pub fn is_opaque(self) -> bool {
        self.info().is_opaque
    }

    #[inline]
    pub fn is_full_cube(self) -> bool {
        self.info().is_full_cube
    }

    #[inline]
    pub fn texture_layer(self, face: Face) -> u32 {
        let tex = self.info().textures;
        match face {
            Face::Top => tex[0],
            Face::Bottom => tex[1],
            Face::Side => tex[2],
        }
    }
}

#[allow(dead_code)]
pub struct BlockInfo {
    pub name: &'static str,
    pub is_full_cube: bool,
    pub is_opaque: bool,
    pub textures: [u32; 3], // top, bottom, side
}

const fn info(name: &'static str, full: bool, opaque: bool, textures: [u32; 3]) -> BlockInfo {
    BlockInfo {
        name,
        is_full_cube: full,
        is_opaque: opaque,
        textures,
    }
}

// Indexed by BlockId.0; keep ordering in sync with the constants above.
pub static BLOCK_TABLE: &[BlockInfo] = &[
    info("air", false, false, [0, 0, 0]),
    info("stone", true, true, [1, 1, 1]),
    info("dirt", true, true, [2, 2, 2]),
    info("grass", true, true, [3, 2, 4]),
    info("sand", true, true, [2, 2, 2]),
    info("cobblestone", true, true, [1, 1, 1]),
    info("oak_log", true, true, [6, 6, 6]),
    info("oak_leaves", true, true, [7, 7, 7]),
    info("oak_planks", true, true, [10, 10, 10]),
    info("water", true, false, [5, 5, 5]),
    info("bedrock", true, true, [11, 11, 11]),
    info("deepslate", true, true, [12, 12, 12]),
    info("gravel", true, true, [13, 13, 13]),
    info("snow_block", true, true, [14, 14, 14]),
    info("netherrack", true, true, [15, 15, 15]),
    info("end_stone", true, true, [16, 16, 16]),
];
