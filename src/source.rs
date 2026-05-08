use std::{
    collections::HashMap,
    fs::File,
    io::BufReader,
    path::{Path, PathBuf},
    sync::Mutex,
};

use anyhow::{Context, Result, anyhow};
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
    region_cache: Mutex<HashMap<(i32, i32), Region<BufReader<File>>>>,
}

impl AnvilSource {
    pub fn new(world_path: impl AsRef<Path>) -> Self {
        Self {
            world_path: world_path.as_ref().to_path_buf(),
            region_cache: Mutex::new(HashMap::new()),
        }
    }

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

fn map_block_name(name: &str) -> BlockId {
    match name {
        "minecraft:air" | "minecraft:cave_air" | "minecraft:void_air" => BlockId::AIR,
        "minecraft:stone"
        | "minecraft:granite"
        | "minecraft:andesite"
        | "minecraft:diorite"
        | "minecraft:deepslate"
        | "minecraft:tuff"
        | "minecraft:calcite"
        | "minecraft:dripstone_block" => BlockId::STONE,
        "minecraft:dirt"
        | "minecraft:coarse_dirt"
        | "minecraft:rooted_dirt"
        | "minecraft:podzol"
        | "minecraft:mycelium" => BlockId::DIRT,
        "minecraft:grass_block" => BlockId::GRASS,
        "minecraft:sand" | "minecraft:red_sand" => BlockId::SAND,
        "minecraft:cobblestone" | "minecraft:mossy_cobblestone" => BlockId::COBBLESTONE,
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
        "minecraft:bedrock" => BlockId::BEDROCK,
        _ => BlockId::AIR,
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
