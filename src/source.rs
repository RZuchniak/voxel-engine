#[cfg(not(target_arch = "wasm32"))]
use std::{collections::HashMap, sync::Mutex};
use std::path::{Path, PathBuf};
#[cfg(not(target_arch = "wasm32"))]
use std::{fs::File, io::BufReader};

use anyhow::Result;
#[cfg(not(target_arch = "wasm32"))]
use anyhow::{Context, anyhow};
#[cfg(not(target_arch = "wasm32"))]
use fastanvil::Region;
use fastnbt::LongArray;
use serde::Deserialize;

use crate::{
    block::BlockId,
    world::{Chunk, World},
};

pub trait WorldSource: Send + Sync {
    fn load_chunk(&self, coord: (i32, i32)) -> Result<Chunk>;
}

#[allow(dead_code)]
pub struct ProceduralSource;

impl WorldSource for ProceduralSource {
    fn load_chunk(&self, coord: (i32, i32)) -> Result<Chunk> {
        Ok(World::generate_procedural_chunk(coord))
    }
}

pub struct AnvilSource {
    world_path: PathBuf,
    #[cfg(not(target_arch = "wasm32"))]
    region_cache: Mutex<HashMap<(i32, i32), Region<BufReader<File>>>>,
}

impl AnvilSource {
    pub fn new(world_path: impl AsRef<Path>) -> Self {
        Self {
            world_path: world_path.as_ref().to_path_buf(),
            #[cfg(not(target_arch = "wasm32"))]
            region_cache: Mutex::new(HashMap::new()),
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn region_mut<'a>(
        &'a self,
        cache: &'a mut HashMap<(i32, i32), Region<BufReader<File>>>,
        rx: i32,
        rz: i32,
    ) -> Result<&'a mut Region<BufReader<File>>> {
        if !cache.contains_key(&(rx, rz)) {
            let path = self
                .world_path
                .join("region")
                .join(format!("r.{rx}.{rz}.mca"));
            let file = File::open(&path)
                .with_context(|| format!("failed opening region file {}", path.display()))?;
            let region = Region::from_stream(BufReader::new(file))
                .with_context(|| format!("failed parsing region file {}", path.display()))?;
            cache.insert((rx, rz), region);
        }
        cache
            .get_mut(&(rx, rz))
            .ok_or_else(|| anyhow!("region cache lookup failed"))
    }
}

impl WorldSource for AnvilSource {
    fn load_chunk(&self, coord: (i32, i32)) -> Result<Chunk> {
        #[cfg(target_arch = "wasm32")]
        {
            return Ok(World::generate_procedural_chunk(coord));
        }

        #[cfg(not(target_arch = "wasm32"))]
        {
        let (cx, cz) = coord;
        let rx = cx.div_euclid(32);
        let rz = cz.div_euclid(32);
        let local_x = cx.rem_euclid(32) as usize;
        let local_z = cz.rem_euclid(32) as usize;

        let mut cache = self
            .region_cache
            .lock()
            .map_err(|_| anyhow!("region cache mutex poisoned"))?;
        let region = self.region_mut(&mut cache, rx, rz)?;
        let Some(raw_chunk) = region
            .read_chunk(local_x, local_z)
            .with_context(|| format!("failed reading chunk {cx},{cz} from region {rx},{rz}"))?
        else {
            return Ok(Chunk::new(coord));
        };
        drop(cache);

        let nbt: JavaChunkNbt = fastnbt::from_bytes(&raw_chunk)
            .with_context(|| format!("failed to decode NBT for chunk {cx},{cz}"))?;
        let mut out = Chunk::new(coord);
        let sections = nbt.sections.or_else(|| nbt.level.and_then(|l| l.sections));
        if let Some(sections) = sections {
            for section in sections {
                let Some(block_states) = section.block_states else {
                    continue;
                };
                let palette = block_states.palette;
                if palette.is_empty() {
                    continue;
                }
                let y_base = section.y * 16;
                if let Some(data) = block_states.data {
                    let indices = fastanvil::expand_blockstates(&data, palette.len());
                    for idx in 0..4096usize {
                        let pal_idx = indices[idx] as usize;
                        if pal_idx >= palette.len() {
                            continue;
                        }
                        let block = map_block_name(&palette[pal_idx].name);
                        if block == BlockId::AIR {
                            continue;
                        }
                        let lx = idx & 0xF;
                        let lz = (idx >> 4) & 0xF;
                        let ly = (idx >> 8) & 0xF;
                        out.set_block_world(lx, y_base + ly as i32, lz, block);
                    }
                } else {
                    // Single-value palette sections may omit packed data.
                    let block = map_block_name(&palette[0].name);
                    if block == BlockId::AIR {
                        continue;
                    }
                    for ly in 0..16usize {
                        for lz in 0..16usize {
                            for lx in 0..16usize {
                                out.set_block_world(lx, y_base + ly as i32, lz, block);
                            }
                        }
                    }
                }
            }
        }
        Ok(out)
        }
    }
}

pub struct PackedWebSource {
    fallback: ProceduralSource,
    _base_url: String,
}

impl PackedWebSource {
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            fallback: ProceduralSource,
            _base_url: base_url.into(),
        }
    }
}

impl WorldSource for PackedWebSource {
    fn load_chunk(&self, coord: (i32, i32)) -> Result<Chunk> {
        // Placeholder web path: until chunk fetch/decode is wired, preserve interface
        // and return deterministic procedural terrain so wasm builds can render.
        self.fallback.load_chunk(coord)
    }
}

fn map_block_name(name: &str) -> BlockId {
    match name {
        "minecraft:air" | "minecraft:cave_air" | "minecraft:void_air" => BlockId::AIR,
        "minecraft:stone"
        | "minecraft:granite"
        | "minecraft:andesite"
        | "minecraft:diorite"
        | "minecraft:tuff"
        | "minecraft:calcite"
        | "minecraft:dripstone_block" => BlockId::STONE,
        "minecraft:deepslate"
        | "minecraft:cobbled_deepslate"
        | "minecraft:polished_deepslate"
        | "minecraft:deepslate_iron_ore"
        | "minecraft:deepslate_coal_ore"
        | "minecraft:deepslate_copper_ore"
        | "minecraft:deepslate_gold_ore"
        | "minecraft:deepslate_redstone_ore"
        | "minecraft:deepslate_lapis_ore"
        | "minecraft:deepslate_diamond_ore"
        | "minecraft:deepslate_emerald_ore" => BlockId::DEEPSLATE,
        "minecraft:dirt"
        | "minecraft:coarse_dirt"
        | "minecraft:rooted_dirt"
        | "minecraft:podzol"
        | "minecraft:mycelium" => BlockId::DIRT,
        "minecraft:grass_block" => BlockId::GRASS,
        "minecraft:sand" | "minecraft:red_sand" => BlockId::SAND,
        "minecraft:cobblestone" | "minecraft:mossy_cobblestone" => BlockId::COBBLESTONE,
        "minecraft:iron_ore"
        | "minecraft:coal_ore"
        | "minecraft:copper_ore"
        | "minecraft:gold_ore"
        | "minecraft:redstone_ore"
        | "minecraft:lapis_ore"
        | "minecraft:diamond_ore"
        | "minecraft:emerald_ore" => BlockId::STONE,
        "minecraft:gravel" => BlockId::GRAVEL,
        "minecraft:oak_log"
        | "minecraft:spruce_log"
        | "minecraft:birch_log"
        | "minecraft:jungle_log"
        | "minecraft:acacia_log"
        | "minecraft:dark_oak_log"
        | "minecraft:mangrove_log"
        | "minecraft:cherry_log" => BlockId::OAK_LOG,
        "minecraft:oak_leaves"
        | "minecraft:spruce_leaves"
        | "minecraft:birch_leaves"
        | "minecraft:jungle_leaves"
        | "minecraft:acacia_leaves"
        | "minecraft:dark_oak_leaves"
        | "minecraft:mangrove_leaves"
        | "minecraft:cherry_leaves" => BlockId::OAK_LEAVES,
        "minecraft:oak_planks" => BlockId::OAK_PLANKS,
        "minecraft:water" => BlockId::WATER,
        // Underwater plants / thin blocks: render as water volume (no fake stone columns).
        "minecraft:kelp" | "minecraft:kelp_plant" => BlockId::WATER,
        "minecraft:seagrass" | "minecraft:tall_seagrass" => BlockId::WATER,
        "minecraft:sea_pickle" => BlockId::WATER,
        "minecraft:bubble_column" => BlockId::WATER,
        "minecraft:tube_coral"
        | "minecraft:brain_coral"
        | "minecraft:bubble_coral"
        | "minecraft:fire_coral"
        | "minecraft:horn_coral" => BlockId::WATER,
        "minecraft:tube_coral_fan"
        | "minecraft:brain_coral_fan"
        | "minecraft:bubble_coral_fan"
        | "minecraft:fire_coral_fan"
        | "minecraft:horn_coral_fan" => BlockId::WATER,
        "minecraft:tube_coral_wall_fan"
        | "minecraft:brain_coral_wall_fan"
        | "minecraft:bubble_coral_wall_fan"
        | "minecraft:fire_coral_wall_fan"
        | "minecraft:horn_coral_wall_fan" => BlockId::WATER,
        "minecraft:bedrock" => BlockId::BEDROCK,
        "minecraft:snow_block" => BlockId::SNOW_BLOCK,
        "minecraft:netherrack" => BlockId::NETHERRACK,
        "minecraft:end_stone" => BlockId::END_STONE,
        _ => BlockId::STONE,
    }
}

#[derive(Debug, Deserialize)]
struct JavaChunkNbt {
    #[serde(rename = "sections")]
    sections: Option<Vec<SectionNbt>>,
    #[serde(rename = "Level")]
    level: Option<LegacyLevelNbt>,
}

#[derive(Debug, Deserialize)]
struct LegacyLevelNbt {
    #[serde(rename = "sections")]
    sections: Option<Vec<SectionNbt>>,
}

#[derive(Debug, Deserialize)]
struct SectionNbt {
    #[serde(rename = "Y")]
    y: i32,
    #[serde(rename = "block_states")]
    block_states: Option<BlockStatesNbt>,
}

#[derive(Debug, Deserialize)]
struct BlockStatesNbt {
    #[serde(rename = "palette")]
    palette: Vec<PaletteEntry>,
    #[serde(rename = "data")]
    data: Option<LongArray>,
}

#[derive(Debug, Deserialize)]
struct PaletteEntry {
    #[serde(rename = "Name")]
    name: String,
}
