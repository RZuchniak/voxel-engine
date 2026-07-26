#[cfg(not(target_arch = "wasm32"))]
use std::path::Path;

pub struct BlockTextureSet {
    pub bind_group_layout: wgpu::BindGroupLayout,
    pub bind_group: wgpu::BindGroup,
}

/// Texture array layer index → resource-pack filename under `BLOCK_TEXTURE_DIR`.
/// Layer 3 is built from `grass.png` + side tint (see `load_grass_top_layer`).
/// Order must stay in sync with `block::BLOCK_TABLE` texture indices.
pub const BLOCK_TEXTURE_FILES: &[&str] = &[
    "stone.png",            // 0 missing
    "stone.png",            // 1
    "dirt.png",             // 2
    "grass.png",            // 3 top — biome-style overlay on green base
    "grass_block_side.png", // 4 side
    "water_still.png",      // 5 — grayscale in this pack; tinted at load
    "oak_log.png",          // 6
    "oak_leaves.png",       // 7 — grayscale + alpha; tinted at load
    "sandstone_top.png",    // 8 sand
    "cobblestone.png",      // 9
    "oak_planks.png",       // 10
    "bedrock.png",          // 11
    "deepslate.png",        // 12
    "gravel.png",           // 13
    "snow.png",             // 14
    "netherrack.png",       // 15
    "end_stone.png",        // 16
    "ice.png",              // 17
    // Added for the Minecraft-parity generator (`mc::chunk`). Several are absent from this
    // resource pack; each has a fallback colour below so the world still reads correctly.
    "lava_still.png",          // 18
    "sandstone.png",           // 19
    "red_sand.png",            // 20
    "red_sandstone.png",       // 21
    "podzol_top.png",          // 22
    "coarse_dirt.png",         // 23
    "mycelium_top.png",        // 24
    "mud.png",                 // 25
    "calcite.png",             // 26
    "packed_ice.png",          // 27
    "powder_snow.png",         // 28
    "terracotta.png",          // 29
    "white_terracotta.png",    // 30
    "orange_terracotta.png",   // 31
    "yellow_terracotta.png",   // 32
    "brown_terracotta.png",    // 33
    "red_terracotta.png",      // 34
    "light_gray_terracotta.png", // 35
];

pub const BLOCK_TEXTURE_DIR: &str = "resource_pack/assets/minecraft/textures/block";

const TEX_SIZE: u32 = 16;
/// Mip chain for 16×16 tiles (16 → 8 → 4 → 2 → 1).
const BLOCK_MIP_LEVELS: u32 = 5;

/// Downsample a 16×16 RGBA layer into mip levels (wgpu 26 has no CommandEncoder::generate_mipmap).
fn build_mip_chain(base: &[u8]) -> Vec<Vec<u8>> {
    let mut mips = vec![base.to_vec()];
    let mut current =
        image::RgbaImage::from_raw(TEX_SIZE, TEX_SIZE, base.to_vec()).expect("16x16 layer");
    while current.width() > 1 {
        let next_w = current.width() / 2;
        let next_h = current.height() / 2;
        current = image::imageops::resize(
            &current,
            next_w,
            next_h,
            image::imageops::FilterType::Triangle,
        );
        mips.push(current.as_raw().to_vec());
    }
    mips
}

/// Approximate default biome tints (pack expects `textures/colormap/*.png`).
const FOLIAGE_TINT: [f32; 3] = [0.45, 0.72, 0.22];
const WATER_TINT: [f32; 3] = [0.22, 0.48, 0.92];

fn fallback_rgba(color: [u8; 4]) -> Vec<u8> {
    let mut out = vec![0u8; (TEX_SIZE * TEX_SIZE * 4) as usize];
    for px in out.chunks_exact_mut(4) {
        px.copy_from_slice(&color);
    }
    out
}

#[cfg(not(target_arch = "wasm32"))]
fn open_pack_image(pack_file: &str) -> Option<image::DynamicImage> {
    let path = Path::new(BLOCK_TEXTURE_DIR).join(pack_file);
    if !path.exists() {
        eprintln!("block texture missing {}", path.display());
        return None;
    }
    match image::open(&path) {
        Ok(img) => Some(img),
        Err(err) => {
            eprintln!("failed opening {}: {err:#}", path.display());
            None
        }
    }
}

fn to_16x16_rgba(img: image::DynamicImage) -> Vec<u8> {
    let rgba = img.to_rgba8();
    let (w, h) = rgba.dimensions();
    if w == TEX_SIZE && h == TEX_SIZE {
        return rgba.into_raw();
    }
    if w == TEX_SIZE && h >= TEX_SIZE {
        let frame: Vec<u8> = rgba
            .chunks_exact(4)
            .take((TEX_SIZE * TEX_SIZE) as usize)
            .flat_map(|p| p.iter().copied())
            .collect();
        if frame.len() == (TEX_SIZE * TEX_SIZE * 4) as usize {
            return frame;
        }
    }
    image::imageops::resize(
        &rgba,
        TEX_SIZE,
        TEX_SIZE,
        image::imageops::FilterType::Nearest,
    )
    .into_raw()
}

fn load_raw_bytes(bytes: &[u8], fallback: [u8; 4]) -> Vec<u8> {
    match image::load_from_memory(bytes) {
        Ok(img) => to_16x16_rgba(img),
        Err(_) => fallback_rgba(fallback),
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn load_raw_layer(pack_file: &str, fallback: [u8; 4]) -> Vec<u8> {
    open_pack_image(pack_file)
        .map(to_16x16_rgba)
        .unwrap_or_else(|| fallback_rgba(fallback))
}

#[cfg(target_arch = "wasm32")]
fn embedded_png(pack_file: &str) -> Option<&'static [u8]> {
    Some(match pack_file {
        "stone.png" => include_bytes!("../resource_pack/assets/minecraft/textures/block/stone.png"),
        "dirt.png" => include_bytes!("../resource_pack/assets/minecraft/textures/block/dirt.png"),
        "grass.png" => include_bytes!("../resource_pack/assets/minecraft/textures/block/grass.png"),
        "grass_block_side.png" => {
            include_bytes!("../resource_pack/assets/minecraft/textures/block/grass_block_side.png")
        }
        "water_still.png" => {
            include_bytes!("../resource_pack/assets/minecraft/textures/block/water_still.png")
        }
        "oak_log.png" => include_bytes!("../resource_pack/assets/minecraft/textures/block/oak_log.png"),
        "oak_leaves.png" => {
            include_bytes!("../resource_pack/assets/minecraft/textures/block/oak_leaves.png")
        }
        "sandstone_top.png" => {
            include_bytes!("../resource_pack/assets/minecraft/textures/block/sandstone_top.png")
        }
        "cobblestone.png" => {
            include_bytes!("../resource_pack/assets/minecraft/textures/block/cobblestone.png")
        }
        "oak_planks.png" => {
            include_bytes!("../resource_pack/assets/minecraft/textures/block/oak_planks.png")
        }
        "bedrock.png" => include_bytes!("../resource_pack/assets/minecraft/textures/block/bedrock.png"),
        "deepslate.png" => {
            include_bytes!("../resource_pack/assets/minecraft/textures/block/deepslate.png")
        }
        "gravel.png" => include_bytes!("../resource_pack/assets/minecraft/textures/block/gravel.png"),
        "snow.png" => include_bytes!("../resource_pack/assets/minecraft/textures/block/snow.png"),
        "netherrack.png" => {
            include_bytes!("../resource_pack/assets/minecraft/textures/block/netherrack.png")
        }
        "end_stone.png" => {
            include_bytes!("../resource_pack/assets/minecraft/textures/block/end_stone.png")
        }
        "ice.png" => include_bytes!("../resource_pack/assets/minecraft/textures/block/ice.png"),
        // The grass top is synthesised from these two (see `load_grass_top_layer`), so the overlay
        // has to be embedded even though no layer maps to it directly.
        "grass_block_side_overlay.png" => include_bytes!(
            "../resource_pack/assets/minecraft/textures/block/grass_block_side_overlay.png"
        ),
        // These five exist in the pack but were never embedded, so they were real textures
        // natively and flat colours in the browser — the visible half of the reported
        // "looks different between wasm and native". The other thirteen `mc::` layers
        // (sandstone, red_sand, mycelium_top, calcite, packed_ice, powder_snow, the seven
        // terracottas) are absent from the pack entirely and so are flat on *both* targets;
        // adding them here would not compile.
        "lava_still.png" => {
            include_bytes!("../resource_pack/assets/minecraft/textures/block/lava_still.png")
        }
        "red_sandstone.png" => {
            include_bytes!("../resource_pack/assets/minecraft/textures/block/red_sandstone.png")
        }
        "podzol_top.png" => {
            include_bytes!("../resource_pack/assets/minecraft/textures/block/podzol_top.png")
        }
        "coarse_dirt.png" => {
            include_bytes!("../resource_pack/assets/minecraft/textures/block/coarse_dirt.png")
        }
        "mud.png" => include_bytes!("../resource_pack/assets/minecraft/textures/block/mud.png"),
        _ => return None,
    })
}

#[cfg(target_arch = "wasm32")]
fn load_raw_layer(pack_file: &str, fallback: [u8; 4]) -> Vec<u8> {
    embedded_png(pack_file)
        .map(|bytes| load_raw_bytes(bytes, fallback))
        .unwrap_or_else(|| fallback_rgba(fallback))
}

#[inline]
fn luminance(px: &[u8]) -> f32 {
    (px[0] as f32 * 0.299 + px[1] as f32 * 0.587 + px[2] as f32 * 0.114) / 255.0
}

/// Synthesise the grass-block top, which this resource pack does not ship.
///
/// `grass_block_top.png` is absent: the pack is texture-*only*, so it overrides what it wants and
/// inherits the rest from vanilla — and this engine has no vanilla layer to inherit from. The top
/// therefore has to be built from other art in the pack.
///
/// **The previous attempt used the wrong two sources.** Its detail mask was `grass.png`, which in a
/// pre-1.20.3 pack is the **short-grass plant sprite**, not a block texture: rows 0..=10 are fully
/// transparent and only rows 11..=15 carry a grayscale tuft. Since transparent pixels fell through
/// to a flat colour, the result was flat over its upper 11/16 and mottled across the lower 5/16 —
/// the horizontal banding that top faces visibly showed. Its base colour was also a single pixel
/// sampled from the side texture, so the whole face was one hue.
///
/// The two sources that *are* right for this:
/// - **`grass_block_side.png`'s green rows** are finished, fully-opaque green grass art. (This pack
///   is stylised: the side is green almost to the bottom with dirt only in the last rows, the
///   inverse of vanilla's thin fringe.) Their mean gives the hue.
/// - **`grass_block_side_overlay.png`** is the pack's real grayscale grass overlay, opaque for
///   rows 0..=12. Its luminance, normalised about its own mean, gives the detail.
///
/// Only the overlay contributes texture — modulating already-noisy green by a second noise field
/// reads as mush. The overlay is tiled rather than stretched, because resampling 13 rows to 16
/// smears vertically, and a top face is seen from directly above where that would be obvious.
///
/// `pub` so `examples/diag_texture_probe` can dump the result: this is generated art, and the only
/// way to judge it is to look at it.
pub fn load_grass_top_layer(fallback: [u8; 4]) -> Vec<u8> {
    let side = load_raw_layer("grass_block_side.png", fallback);
    let px_at = |data: &[u8], x: u32, y: u32| -> [u8; 4] {
        let i = ((y * TEX_SIZE + x) * 4) as usize;
        [data[i], data[i + 1], data[i + 2], data[i + 3]]
    };

    // Rows where green actually dominates, found rather than hardcoded — the dirt rows at the
    // bottom must not drag the hue brown, and where the boundary sits is a property of the pack.
    let mut hue = [0f32; 3];
    let mut green_rows = 0u32;
    for y in 0..TEX_SIZE {
        let mut row = [0f32; 3];
        let mut opaque = 0u32;
        for x in 0..TEX_SIZE {
            let p = px_at(&side, x, y);
            if p[3] > 200 {
                opaque += 1;
                for c in 0..3 {
                    row[c] += p[c] as f32;
                }
            }
        }
        if opaque == 0 {
            continue;
        }
        let mean = [row[0] / opaque as f32, row[1] / opaque as f32, row[2] / opaque as f32];
        if mean[1] > mean[0] + 20.0 && mean[1] > mean[2] + 20.0 {
            for c in 0..3 {
                hue[c] += mean[c];
            }
            green_rows += 1;
        }
    }
    if green_rows == 0 {
        // No green in the side texture at all — nothing sensible to derive a top from.
        return fallback_rgba(fallback);
    }
    for c in 0..3 {
        hue[c] /= green_rows as f32;
    }

    let overlay = load_raw_layer("grass_block_side_overlay.png", [255, 255, 255, 255]);
    let mut detail_rows = 0u32;
    let mut detail_mean = 0f32;
    for y in 0..TEX_SIZE {
        let mut row = 0f32;
        let mut opaque = 0u32;
        for x in 0..TEX_SIZE {
            let p = px_at(&overlay, x, y);
            if p[3] > 200 {
                opaque += 1;
                row += luminance(&p);
            }
        }
        // Only fully-opaque rows: the overlay's lower rows fade out into the fringe, and a
        // partly-transparent row would bias the mean and tile as a visible band.
        if opaque == TEX_SIZE {
            detail_mean += row / opaque as f32;
            detail_rows += 1;
        }
    }
    if detail_rows == 0 {
        return fallback_rgba(fallback);
    }
    detail_mean /= detail_rows as f32;

    let mut out = vec![0u8; (TEX_SIZE * TEX_SIZE * 4) as usize];
    for y in 0..TEX_SIZE {
        for x in 0..TEX_SIZE {
            let src = px_at(&overlay, x, y % detail_rows);
            // Normalised about the mean, so the overlay changes contrast without shifting overall
            // brightness, and clamped so a bright speckle cannot blow out to white.
            let shade = if detail_mean > 0.0 {
                (luminance(&src) / detail_mean).clamp(0.75, 1.25)
            } else {
                1.0
            };
            let i = ((y * TEX_SIZE + x) * 4) as usize;
            for c in 0..3 {
                out[i + c] = (hue[c] * shade).clamp(0.0, 255.0) as u8;
            }
            out[i + 3] = 255;
        }
    }
    out
}

/// Grayscale foliage masks → multiply by biome green (colormap not loaded).
fn apply_foliage_tint(pixels: &mut [u8]) {
    for px in pixels.chunks_exact_mut(4) {
        if px[3] < 10 {
            px[3] = 0;
            continue;
        }
        let lum = (px[0] as f32 * 0.299 + px[1] as f32 * 0.587 + px[2] as f32 * 0.114) / 255.0;
        let boost = 1.15 + lum * 0.35;
        px[0] = (lum * 255.0 * FOLIAGE_TINT[0] * boost).min(255.0) as u8;
        px[1] = (lum * 255.0 * FOLIAGE_TINT[1] * boost).min(255.0) as u8;
        px[2] = (lum * 255.0 * FOLIAGE_TINT[2] * boost).min(255.0) as u8;
        px[3] = 255;
    }
}

/// Grayscale water mask → multiply by water blue (colormap not loaded).
fn apply_water_tint(pixels: &mut [u8]) {
    for px in pixels.chunks_exact_mut(4) {
        if px[3] < 10 {
            px[3] = 0;
            continue;
        }
        let lum = (px[0] as f32 * 0.299 + px[1] as f32 * 0.587 + px[2] as f32 * 0.114) / 255.0;
        let boost = 1.1 + lum * 0.25;
        px[0] = (lum * 255.0 * WATER_TINT[0] * boost).min(255.0) as u8;
        px[1] = (lum * 255.0 * WATER_TINT[1] * boost).min(255.0) as u8;
        px[2] = (lum * 255.0 * WATER_TINT[2] * boost).min(255.0) as u8;
        px[3] = 255;
    }
}

fn load_layer_by_index(layer: usize, pack_file: &str, fallback: [u8; 4]) -> Vec<u8> {
    let mut data = match layer {
        3 => load_grass_top_layer(fallback),
        5 => load_raw_layer(pack_file, fallback),
        7 => load_raw_layer(pack_file, fallback),
        _ => load_raw_layer(pack_file, fallback),
    };
    match layer {
        5 => apply_water_tint(&mut data),
        7 => apply_foliage_tint(&mut data),
        _ => {}
    }
    data
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;

    /// The synthesised grass top must carry detail in **every** row.
    ///
    /// One uniform row is the exact signature of the bug this replaced: the old version masked a
    /// flat colour with `grass.png` — the short-grass *plant* sprite, whose rows 0..=10 are fully
    /// transparent — so those rows fell straight through to a single colour and the top face showed
    /// a hard horizontal band across its upper two-thirds.
    ///
    /// Skips rather than fails when the pack is absent: `resource_pack/` is not committed, and
    /// without it every loader returns a flat fallback, which would fail this for the wrong reason.
    #[test]
    fn the_synthesised_grass_top_has_detail_in_every_row() {
        if !Path::new(BLOCK_TEXTURE_DIR).is_dir() {
            eprintln!("skipping: {BLOCK_TEXTURE_DIR} is not present");
            return;
        }
        let top = load_grass_top_layer([120, 170, 80, 255]);
        assert_eq!(top.len(), (TEX_SIZE * TEX_SIZE * 4) as usize);

        for y in 0..TEX_SIZE {
            let mut distinct = std::collections::BTreeSet::new();
            for x in 0..TEX_SIZE {
                let i = ((y * TEX_SIZE + x) * 4) as usize;
                distinct.insert([top[i], top[i + 1], top[i + 2]]);
                assert_eq!(top[i + 3], 255, "the top face must be fully opaque");
            }
            assert!(
                distinct.len() >= 3,
                "row {y} of the grass top has only {} distinct colour(s) — the detail mask is not \
                 reaching it, which is how the old banding looked",
                distinct.len()
            );
        }
    }

    /// The hue must come from the pack's green, not from the caller's fallback.
    ///
    /// Cheap guard against the green-row detection silently finding nothing (in which case the
    /// function bails to `fallback_rgba` and the top becomes a flat colour again).
    #[test]
    fn the_synthesised_grass_top_is_green() {
        if !Path::new(BLOCK_TEXTURE_DIR).is_dir() {
            eprintln!("skipping: {BLOCK_TEXTURE_DIR} is not present");
            return;
        }
        // A fallback that is obviously *not* green, so falling back cannot pass this by accident.
        let top = load_grass_top_layer([200, 0, 200, 255]);
        for px in top.chunks_exact(4) {
            assert!(
                px[1] > px[0] && px[1] > px[2],
                "grass top pixel {px:?} is not green-dominant"
            );
        }
    }
}

pub fn create_block_textures(device: &wgpu::Device, queue: &wgpu::Queue) -> BlockTextureSet {
    let mut layers = Vec::with_capacity(BLOCK_TEXTURE_FILES.len());
    for (layer, pack_file) in BLOCK_TEXTURE_FILES.iter().enumerate() {
        let fallback = match *pack_file {
            "stone.png" => [128, 128, 128, 255],
            "dirt.png" => [120, 86, 58, 255],
            "grass_block_side.png" => [96, 131, 75, 255],
            "water_still.png" => [45, 95, 220, 255],
            "oak_log.png" => [110, 85, 58, 255],
            "oak_leaves.png" => [60, 130, 60, 255],
            "sandstone_top.png" => [219, 211, 160, 255],
            "cobblestone.png" => [113, 113, 113, 255],
            "oak_planks.png" => [171, 141, 89, 255],
            "bedrock.png" => [69, 69, 69, 255],
            "deepslate.png" => [84, 84, 90, 255],
            "gravel.png" => [134, 131, 128, 255],
            "snow.png" => [236, 240, 246, 255],
            "netherrack.png" => [109, 52, 52, 255],
            "end_stone.png" => [220, 220, 171, 255],
            "ice.png" => [145, 190, 255, 220],
            "grass.png" => [120, 170, 80, 255],
            "lava_still.png" => [207, 92, 22, 255],
            "sandstone.png" => [216, 208, 156, 255],
            "red_sand.png" => [190, 102, 33, 255],
            "red_sandstone.png" => [181, 97, 31, 255],
            "podzol_top.png" => [91, 63, 24, 255],
            "coarse_dirt.png" => [119, 85, 59, 255],
            "mycelium_top.png" => [111, 99, 105, 255],
            "mud.png" => [60, 55, 60, 255],
            "calcite.png" => [223, 222, 216, 255],
            "packed_ice.png" => [141, 180, 219, 255],
            "powder_snow.png" => [246, 250, 253, 255],
            "terracotta.png" => [152, 94, 67, 255],
            "white_terracotta.png" => [209, 178, 161, 255],
            "orange_terracotta.png" => [162, 84, 38, 255],
            "yellow_terracotta.png" => [186, 133, 35, 255],
            "brown_terracotta.png" => [77, 51, 36, 255],
            "red_terracotta.png" => [143, 61, 47, 255],
            "light_gray_terracotta.png" => [135, 107, 98, 255],
            _ => [255, 0, 255, 255],
        };
        layers.push(load_layer_by_index(layer, pack_file, fallback));
    }

    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("Block Texture Array"),
        size: wgpu::Extent3d {
            width: TEX_SIZE,
            height: TEX_SIZE,
            depth_or_array_layers: layers.len() as u32,
        },
        mip_level_count: BLOCK_MIP_LEVELS,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8UnormSrgb,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });

    for (layer, data) in layers.iter().enumerate() {
        for (mip_level, mip_data) in build_mip_chain(data).into_iter().enumerate() {
            let mip_size = (TEX_SIZE >> mip_level).max(1);
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &texture,
                    mip_level: mip_level as u32,
                    origin: wgpu::Origin3d {
                        x: 0,
                        y: 0,
                        z: layer as u32,
                    },
                    aspect: wgpu::TextureAspect::All,
                },
                &mip_data,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(mip_size * 4),
                    rows_per_image: Some(mip_size),
                },
                wgpu::Extent3d {
                    width: mip_size,
                    height: mip_size,
                    depth_or_array_layers: 1,
                },
            );
        }
    }

    let view = texture.create_view(&wgpu::TextureViewDescriptor {
        label: Some("Block Texture Array View"),
        format: Some(wgpu::TextureFormat::Rgba8UnormSrgb),
        dimension: Some(wgpu::TextureViewDimension::D2Array),
        usage: Some(wgpu::TextureUsages::TEXTURE_BINDING),
        aspect: wgpu::TextureAspect::All,
        base_mip_level: 0,
        mip_level_count: Some(BLOCK_MIP_LEVELS),
        base_array_layer: 0,
        array_layer_count: None,
    });

    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("Block Sampler"),
        address_mode_u: wgpu::AddressMode::Repeat,
        address_mode_v: wgpu::AddressMode::Repeat,
        address_mode_w: wgpu::AddressMode::Repeat,
        // Crisp up close, trilinear minification to stop far-block shimmer.
        mag_filter: wgpu::FilterMode::Nearest,
        min_filter: wgpu::FilterMode::Linear,
        mipmap_filter: wgpu::FilterMode::Linear,
        ..Default::default()
    });

    let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("Block Texture Bind Group Layout"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    multisampled: false,
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2Array,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
        ],
    });

    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("Block Texture Bind Group"),
        layout: &bind_group_layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(&sampler),
            },
        ],
    });

    BlockTextureSet {
        bind_group_layout,
        bind_group,
    }
}
