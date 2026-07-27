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
    pub const ICE: BlockId = BlockId(16);
    // Added for the Minecraft-parity generator (`mc::chunk`).
    pub const LAVA: BlockId = BlockId(17);
    pub const SANDSTONE: BlockId = BlockId(18);
    pub const RED_SAND: BlockId = BlockId(19);
    pub const RED_SANDSTONE: BlockId = BlockId(20);
    pub const PODZOL: BlockId = BlockId(21);
    pub const COARSE_DIRT: BlockId = BlockId(22);
    pub const MYCELIUM: BlockId = BlockId(23);
    pub const MUD: BlockId = BlockId(24);
    pub const CALCITE: BlockId = BlockId(25);
    pub const PACKED_ICE: BlockId = BlockId(26);
    pub const POWDER_SNOW: BlockId = BlockId(27);
    pub const TERRACOTTA: BlockId = BlockId(28);
    pub const WHITE_TERRACOTTA: BlockId = BlockId(29);
    pub const ORANGE_TERRACOTTA: BlockId = BlockId(30);
    pub const YELLOW_TERRACOTTA: BlockId = BlockId(31);
    pub const BROWN_TERRACOTTA: BlockId = BlockId(32);
    pub const RED_TERRACOTTA: BlockId = BlockId(33);
    pub const LIGHT_GRAY_TERRACOTTA: BlockId = BlockId(34);

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

    /// Does this block hide the face of the neighbour behind it?
    ///
    /// Deliberately **not** `is_opaque`, which also governs sight lines in the occlusion graph and
    /// the per-vertex lighting. Face culling needs its own answer, because whether a face is
    /// *visible* and whether a block *transmits light* are different questions:
    ///
    /// - **Fluids do not occlude.** Water and lava have to let the block they touch draw its face,
    ///   or the boundary is left with no surface at all.
    /// - **Cutouts do not occlude.** Leaves have holes; culling behind them shows the sky.
    /// - **Ice does occlude**, despite not being `is_opaque` — see [`info_solid_looking`].
    ///
    /// Overloading `is_opaque` for this is what produced two separate reported bugs: leaves
    /// deleting the ground's face, and water z-fighting against ice.
    #[inline]
    pub fn occludes_faces(self) -> bool {
        self.info().occludes_faces
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
    /// See [`BlockId::occludes_faces`]. Defaults to `is_opaque`; only [`info_solid_looking`]
    /// separates them.
    pub occludes_faces: bool,
    pub textures: [u32; 3], // top, bottom, side
}

const fn info(name: &'static str, full: bool, opaque: bool, textures: [u32; 3]) -> BlockInfo {
    BlockInfo {
        name,
        is_full_cube: full,
        is_opaque: opaque,
        occludes_faces: opaque,
        textures,
    }
}

/// A full cube that is drawn solid but is not light-opaque, so it hides its neighbour's face while
/// still letting sight lines through the occlusion graph.
///
/// Ice is the only such block today. **Nothing blends in this renderer** (`BlendState::REPLACE`), so
/// ice rasterises as a solid cube whatever its alpha says; if it did not occlude, it and the water
/// beside it would each emit a face on their shared plane and the two would z-fight.
const fn info_solid_looking(name: &'static str, textures: [u32; 3]) -> BlockInfo {
    BlockInfo {
        name,
        is_full_cube: true,
        is_opaque: false,
        occludes_faces: true,
        textures,
    }
}

// Indexed by BlockId.0; keep ordering in sync with the constants above.
pub static BLOCK_TABLE: &[BlockInfo] = &[
    info("air", false, false, [0, 0, 0]),
    info("stone", true, true, [1, 1, 1]),
    info("dirt", true, true, [2, 2, 2]),
    info("grass", true, true, [3, 2, 4]),
    info("sand", true, true, [8, 8, 8]),
    info("cobblestone", true, true, [9, 9, 9]),
    info("oak_log", true, true, [6, 6, 6]),
    // Leaves are a **cutout**: binary alpha, ~60% covered. They must not be `is_opaque`, for two
    // separate reasons that both showed up as "you can see through the world" where leaves touch
    // solid blocks:
    //   1. `mesh.rs` culls a face whose neighbour `is_full_cube() && is_opaque()`, so leaves were
    //      deleting the ground's top face, and the leaf's own alpha holes then looked at the sky.
    //   2. Only non-opaque full cubes get the double-sided treatment. Backface culling removes the
    //      inside of a cube's far faces, so a *lone* opaque-flagged leaf block was see-through too.
    info("oak_leaves", true, false, [7, 7, 7]),
    info("oak_planks", true, true, [10, 10, 10]),
    info("water", true, false, [5, 5, 5]),
    info("bedrock", true, true, [11, 11, 11]),
    info("deepslate", true, true, [12, 12, 12]),
    info("gravel", true, true, [13, 13, 13]),
    info("snow_block", true, true, [14, 14, 14]),
    info("netherrack", true, true, [15, 15, 15]),
    info("end_stone", true, true, [16, 16, 16]),
    info_solid_looking("ice", [17, 17, 17]),
    // `mc::chunk` palette. Lava is non-opaque so it glows through like water does.
    info("lava", true, false, [18, 18, 18]),
    info("sandstone", true, true, [19, 19, 19]),
    info("red_sand", true, true, [20, 20, 20]),
    info("red_sandstone", true, true, [21, 21, 21]),
    info("podzol", true, true, [22, 2, 22]),
    info("coarse_dirt", true, true, [23, 23, 23]),
    info("mycelium", true, true, [24, 2, 24]),
    info("mud", true, true, [25, 25, 25]),
    info("calcite", true, true, [26, 26, 26]),
    info("packed_ice", true, true, [27, 27, 27]),
    info("powder_snow", true, true, [28, 28, 28]),
    info("terracotta", true, true, [29, 29, 29]),
    info("white_terracotta", true, true, [30, 30, 30]),
    info("orange_terracotta", true, true, [31, 31, 31]),
    info("yellow_terracotta", true, true, [32, 32, 32]),
    info("brown_terracotta", true, true, [33, 33, 33]),
    info("red_terracotta", true, true, [34, 34, 34]),
    info("light_gray_terracotta", true, true, [35, 35, 35]),
];
