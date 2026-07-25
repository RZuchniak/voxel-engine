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
        _ => return None,
    })
}

#[cfg(target_arch = "wasm32")]
fn load_raw_layer(pack_file: &str, fallback: [u8; 4]) -> Vec<u8> {
    embedded_png(pack_file)
        .map(|bytes| load_raw_bytes(bytes, fallback))
        .unwrap_or_else(|| fallback_rgba(fallback))
}

fn load_grass_top_layer(fallback: [u8; 4]) -> Vec<u8> {
    let side = load_raw_layer("grass_block_side.png", fallback);
    let mut top_color = fallback;
    for y in 0..TEX_SIZE {
        let i = (y * TEX_SIZE * 4) as usize;
        if side[i + 3] > 200 {
            top_color = [side[i], side[i + 1], side[i + 2], 255];
            break;
        }
    }
    let overlay = load_raw_layer("grass.png", [255, 255, 255, 255]);
    let mut out = vec![0u8; (TEX_SIZE * TEX_SIZE * 4) as usize];
    for (i, px) in out.chunks_exact_mut(4).enumerate() {
        let o = &overlay[i * 4..(i + 1) * 4];
        if o[3] < 10 {
            px.copy_from_slice(&top_color);
            continue;
        }
        let lum = (o[0] as f32 * 0.299 + o[1] as f32 * 0.587 + o[2] as f32 * 0.114) / 255.0;
        let shade = 0.82 + lum * 0.28;
        px[0] = (top_color[0] as f32 * shade).min(255.0) as u8;
        px[1] = (top_color[1] as f32 * shade).min(255.0) as u8;
        px[2] = (top_color[2] as f32 * shade).min(255.0) as u8;
        px[3] = 255;
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
