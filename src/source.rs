use std::{
    collections::HashMap,
    io::{Cursor, Read},
    path::{Path, PathBuf},
    sync::Mutex,
};
#[cfg(not(target_arch = "wasm32"))]
use std::{
    fs::File,
    io::{BufReader, ErrorKind},
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

fn decode_section(out: &mut Chunk, section: SectionNbt) -> Option<i32> {
    let Some(block_states) = section.block_states else {
        return None;
    };
    let palette = block_states.palette;
    if palette.is_empty() {
        return None;
    }
    let y_base = section.y * 16;
    let mut section_max_y: Option<i32> = None;
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
            let wy = y_base + ly as i32;
            out.set_block_world(lx, wy, lz, block);
            section_max_y = Some(section_max_y.map(|max_y| max_y.max(wy)).unwrap_or(wy));
        }
    } else {
        let block = map_block_name(&palette[0].name);
        if block == BlockId::AIR {
            return None;
        }
        for ly in 0..16usize {
            for lz in 0..16usize {
                for lx in 0..16usize {
                    let wy = y_base + ly as i32;
                    out.set_block_world(lx, wy, lz, block);
                    section_max_y = Some(section_max_y.map(|max_y| max_y.max(wy)).unwrap_or(wy));
                }
            }
        }
    }
    section_max_y
}

fn decode_java_chunk(coord: (i32, i32), raw: &[u8]) -> Result<Chunk> {
    let surface_only =
        crate::platform::surface_only_chunk_load() && crate::platform::surface_mesh_depth_blocks() > 0;
    decode_java_chunk_inner(coord, raw, surface_only)
}

fn decode_java_chunk_inner(coord: (i32, i32), raw: &[u8], surface_only: bool) -> Result<Chunk> {
    let nbt: JavaChunkNbt =
        fastnbt::from_bytes(raw).with_context(|| format!("failed to decode NBT for chunk {coord:?}"))?;
    let mut out = Chunk::new(coord);
    let sections = nbt.sections.or_else(|| nbt.level.and_then(|l| l.sections));
    let Some(mut sections) = sections else {
        return Ok(out);
    };

    if surface_only {
        sections.sort_by(|a, b| b.y.cmp(&a.y));
    }

    let depth = crate::platform::surface_mesh_depth_blocks();
    let mut surface_max_y: Option<i32> = None;
    for section in sections {
        if surface_only {
            if let Some(max_y) = surface_max_y {
                if section.y * 16 + 15 < max_y - depth {
                    break;
                }
            }
        }
        if let Some(section_max_y) = decode_section(&mut out, section) {
            surface_max_y = Some(
                surface_max_y
                    .map(|max_y| max_y.max(section_max_y))
                    .unwrap_or(section_max_y),
            );
        }
    }
    Ok(out)
}

fn parse_region_filename(name: &str) -> Option<(i32, i32)> {
    let name = name.rsplit(['/', '\\']).next()?;
    let lower = name.to_ascii_lowercase();
    let name = lower.strip_suffix(".mca")?;
    let coords = name.strip_prefix("r.")?;
    let (rx, rz) = coords.split_once('.')?;
    Some((rx.parse().ok()?, rz.parse().ok()?))
}

fn path_looks_like_region_file(path: &str) -> bool {
    path.to_ascii_lowercase().ends_with(".mca")
}

enum MemoryRegionEntry {
    Missing,
    Raw(Vec<u8>),
    Open(Region<Cursor<Vec<u8>>>),
}

pub struct MemoryAnvilSource {
    regions: Mutex<HashMap<(i32, i32), MemoryRegionEntry>>,
}

impl MemoryAnvilSource {
    pub fn from_zip(bytes: &[u8]) -> Result<Self> {
        let mut regions = HashMap::new();
        let mut sample_paths = Vec::new();
        let mut mca_like_paths = Vec::new();
        let mut archive =
            zip::ZipArchive::new(Cursor::new(bytes)).context("failed to read world zip")?;
        for i in 0..archive.len() {
            let mut file = archive
                .by_index(i)
                .with_context(|| format!("failed to read zip entry {i}"))?;
            let path = file.name().replace('\\', "/");
            if sample_paths.len() < 12 {
                sample_paths.push(path.clone());
            }
            if !path_looks_like_region_file(&path) {
                continue;
            }
            if mca_like_paths.len() < 6 {
                mca_like_paths.push(path.clone());
            }
            let Some(coord) = parse_region_filename(&path) else {
                continue;
            };
            let mut data = Vec::new();
            file.read_to_end(&mut data)
                .with_context(|| format!("failed to read {path}"))?;
            regions.insert(coord, MemoryRegionEntry::Raw(data));
        }
        if regions.is_empty() {
            let samples = if sample_paths.is_empty() {
                "(zip appears empty)".to_string()
            } else {
                sample_paths.join(", ")
            };
            let mca_hint = if mca_like_paths.is_empty() {
                String::new()
            } else {
                format!(
                    " Found .mca-like paths but none named r.<x>.<z>.mca: {}.",
                    mca_like_paths.join(", ")
                )
            };
            return Err(anyhow!(
                "No region files found. Sample zip paths: {samples}.{mca_hint} \
                 Zip a Java world save folder so paths look like MyWorld/region/r.0.0.mca."
            ));
        }
        Ok(Self {
            regions: Mutex::new(regions),
        })
    }

    fn ensure_region_open(
        cache: &mut HashMap<(i32, i32), MemoryRegionEntry>,
        rx: i32,
        rz: i32,
    ) -> Result<bool> {
        match cache.get(&(rx, rz)) {
            None | Some(MemoryRegionEntry::Missing) => {
                cache.entry((rx, rz)).or_insert(MemoryRegionEntry::Missing);
                return Ok(false);
            }
            Some(MemoryRegionEntry::Open(_)) => return Ok(true),
            Some(MemoryRegionEntry::Raw(_)) => {}
        }

        let Some(MemoryRegionEntry::Raw(bytes)) = cache.remove(&(rx, rz)) else {
            return Ok(false);
        };
        let region = Region::from_stream(Cursor::new(bytes))
            .with_context(|| format!("failed parsing region r.{rx}.{rz}.mca"))?;
        cache.insert((rx, rz), MemoryRegionEntry::Open(region));
        Ok(true)
    }
}

impl WorldSource for MemoryAnvilSource {
    fn load_chunk(&self, coord: (i32, i32)) -> Result<Chunk> {
        let (cx, cz) = coord;
        let rx = cx.div_euclid(32);
        let rz = cz.div_euclid(32);
        let local_x = cx.rem_euclid(32) as usize;
        let local_z = cz.rem_euclid(32) as usize;

        let mut cache = self
            .regions
            .lock()
            .map_err(|_| anyhow!("memory region cache mutex poisoned"))?;
        if !Self::ensure_region_open(&mut cache, rx, rz)? {
            return Ok(Chunk::new(coord));
        }
        let Some(MemoryRegionEntry::Open(region)) = cache.get_mut(&(rx, rz)) else {
            return Ok(Chunk::new(coord));
        };
        let Some(raw_chunk) = region
            .read_chunk(local_x, local_z)
            .with_context(|| format!("failed reading chunk {cx},{cz} from region {rx},{rz}"))?
        else {
            return Ok(Chunk::new(coord));
        };
        decode_java_chunk(coord, &raw_chunk)
    }
}

#[cfg(not(target_arch = "wasm32"))]
enum RegionEntry {
    Missing,
    Open(Region<BufReader<File>>),
}

pub struct AnvilSource {
    world_path: PathBuf,
    #[cfg(not(target_arch = "wasm32"))]
    region_cache: Mutex<HashMap<(i32, i32), RegionEntry>>,
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
        cache: &'a mut HashMap<(i32, i32), RegionEntry>,
        rx: i32,
        rz: i32,
    ) -> Result<Option<&'a mut Region<BufReader<File>>>> {
        use std::collections::hash_map::Entry;

        match cache.entry((rx, rz)) {
            Entry::Occupied(e) => Ok(match e.into_mut() {
                RegionEntry::Missing => None,
                RegionEntry::Open(r) => Some(r),
            }),
            Entry::Vacant(v) => {
                let path = self
                    .world_path
                    .join("region")
                    .join(format!("r.{rx}.{rz}.mca"));
                let file = match File::open(&path) {
                    Ok(f) => f,
                    Err(e) if e.kind() == ErrorKind::NotFound => {
                        v.insert(RegionEntry::Missing);
                        return Ok(None);
                    }
                    Err(e) => {
                        return Err(e).with_context(|| {
                            format!("failed opening region file {}", path.display())
                        });
                    }
                };
                let region = Region::from_stream(BufReader::new(file))
                    .with_context(|| format!("failed parsing region file {}", path.display()))?;
                let entry = v.insert(RegionEntry::Open(region));
                match entry {
                    RegionEntry::Open(r) => Ok(Some(r)),
                    RegionEntry::Missing => unreachable!(),
                }
            }
        }
    }
}

impl WorldSource for AnvilSource {
    fn load_chunk(&self, coord: (i32, i32)) -> Result<Chunk> {
        #[cfg(target_arch = "wasm32")]
        {
            let _ = coord;
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
            let Some(region) = self.region_mut(&mut cache, rx, rz)? else {
                return Ok(Chunk::new(coord));
            };
            let Some(raw_chunk) = region
                .read_chunk(local_x, local_z)
                .with_context(|| format!("failed reading chunk {cx},{cz} from region {rx},{rz}"))?
            else {
                return Ok(Chunk::new(coord));
            };
            decode_java_chunk(coord, &raw_chunk)
        }
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
        "minecraft:snow_block" | "minecraft:snow" => BlockId::SNOW_BLOCK,
        "minecraft:netherrack" => BlockId::NETHERRACK,
        "minecraft:end_stone" => BlockId::END_STONE,
        "minecraft:ice" | "minecraft:packed_ice" | "minecraft:blue_ice" | "minecraft:frosted_ice" => {
            BlockId::ICE
        }
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn make_zip(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut buffer = Cursor::new(Vec::new());
        let mut writer = zip::ZipWriter::new(&mut buffer);
        let options = zip::write::SimpleFileOptions::default();
        for (path, data) in entries {
            writer.start_file(*path, options).unwrap();
            writer.write_all(data).unwrap();
        }
        writer.finish().unwrap();
        buffer.into_inner()
    }

    #[test]
    fn from_zip_accepts_nested_region_paths() {
        let zip = make_zip(&[
            ("Basic_World/region/r.0.0.mca", b"region"),
            ("Basic_World/level.dat", b"level"),
        ]);
        MemoryAnvilSource::from_zip(&zip).unwrap();
    }

    #[test]
    fn from_zip_accepts_flat_region_paths() {
        let zip = make_zip(&[("region/r.0.0.mca", b"region")]);
        MemoryAnvilSource::from_zip(&zip).unwrap();
    }

    #[test]
    fn from_zip_is_case_insensitive() {
        let zip = make_zip(&[("World/Region/R.0.0.MCA", b"region")]);
        MemoryAnvilSource::from_zip(&zip).unwrap();
    }

    #[test]
    fn from_zip_errors_with_samples_when_missing_regions() {
        let zip = make_zip(&[("Basic_World/level.dat", b"level")]);
        let result = MemoryAnvilSource::from_zip(&zip);
        assert!(result.is_err());
        assert!(
            result
                .err()
                .expect("missing regions error")
                .to_string()
                .contains("Sample zip paths")
        );
    }
}
