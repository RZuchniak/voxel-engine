#[cfg(not(target_arch = "wasm32"))]
use std::path::Path;

#[cfg(not(target_arch = "wasm32"))]
use anyhow::Context;

pub struct BlockTextureSet {
    pub bind_group_layout: wgpu::BindGroupLayout,
    pub bind_group: wgpu::BindGroup,
}

/// Texture array layer index → resource-pack filename under `BLOCK_TEXTURE_DIR`.
/// Order must stay in sync with `block::BLOCK_TABLE` texture indices.
pub const BLOCK_TEXTURE_FILES: &[&str] = &[
    "stone.png",            // 0 missing — reuse stone until a dedicated texture exists
    "stone.png",            // 1
    "dirt.png",             // 2
    "grass.png",            // 3 top (pack uses grass.png, not grass_block_top.png)
    "grass_block_side.png", // 4 side
    "water_still.png",      // 5 (often 16×N animated strip; first frame used)
    "oak_log.png",          // 6
    "oak_leaves.png",       // 7
    "sandstone_top.png",    // 8 sand — pack has no sand.png
    "cobblestone.png",      // 9
    "oak_planks.png",       // 10
    "bedrock.png",          // 11
    "deepslate.png",        // 12
    "gravel.png",           // 13
    "snow.png",             // 14
    "netherrack.png",       // 15
    "end_stone.png",        // 16
];

pub const BLOCK_TEXTURE_DIR: &str = "resource_pack/assets/minecraft/textures/block";

const TEX_SIZE: u32 = 16;

fn fallback_rgba(color: [u8; 4]) -> Vec<u8> {
    let mut out = vec![0u8; (TEX_SIZE * TEX_SIZE * 4) as usize];
    for px in out.chunks_exact_mut(4) {
        px.copy_from_slice(&color);
    }
    out
}

#[cfg(not(target_arch = "wasm32"))]
fn to_16x16_rgba(img: image::DynamicImage) -> Vec<u8> {
    let rgba = img.to_rgba8();
    let (w, h) = rgba.dimensions();
    if w == TEX_SIZE && h == TEX_SIZE {
        return rgba.into_raw();
    }
    // Animated block textures (e.g. water_still 16×512): use the top frame.
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

fn load_layer_rgba(pack_file: &str, fallback: [u8; 4]) -> Vec<u8> {
    #[cfg(target_arch = "wasm32")]
    {
        let _ = pack_file;
        return fallback_rgba(fallback);
    }

    #[cfg(not(target_arch = "wasm32"))]
    {
        let path = Path::new(BLOCK_TEXTURE_DIR).join(pack_file);
        if path.exists()
            && let Ok(img) = image::open(&path).with_context(|| {
                format!("failed opening block texture {}", path.display())
            })
        {
            return to_16x16_rgba(img);
        }
        eprintln!(
            "block texture missing {}, using fallback color",
            path.display()
        );
        fallback_rgba(fallback)
    }
}

pub fn create_block_textures(device: &wgpu::Device, queue: &wgpu::Queue) -> BlockTextureSet {
    let mut layers = Vec::with_capacity(BLOCK_TEXTURE_FILES.len());
    for pack_file in BLOCK_TEXTURE_FILES {
        let fallback = match *pack_file {
            "stone.png" => [128, 128, 128, 255],
            "dirt.png" => [120, 86, 58, 255],
            "grass.png" => [87, 168, 71, 255],
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
            _ => [255, 0, 255, 255],
        };
        layers.push(load_layer_rgba(pack_file, fallback));
    }

    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("Block Texture Array"),
        size: wgpu::Extent3d {
            width: TEX_SIZE,
            height: TEX_SIZE,
            depth_or_array_layers: layers.len() as u32,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8UnormSrgb,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });

    for (layer, data) in layers.iter().enumerate() {
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d {
                    x: 0,
                    y: 0,
                    z: layer as u32,
                },
                aspect: wgpu::TextureAspect::All,
            },
            data,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(TEX_SIZE * 4),
                rows_per_image: Some(TEX_SIZE),
            },
            wgpu::Extent3d {
                width: TEX_SIZE,
                height: TEX_SIZE,
                depth_or_array_layers: 1,
            },
        );
    }

    let view = texture.create_view(&wgpu::TextureViewDescriptor {
        label: Some("Block Texture Array View"),
        format: Some(wgpu::TextureFormat::Rgba8UnormSrgb),
        dimension: Some(wgpu::TextureViewDimension::D2Array),
        usage: Some(wgpu::TextureUsages::TEXTURE_BINDING),
        aspect: wgpu::TextureAspect::All,
        base_mip_level: 0,
        mip_level_count: None,
        base_array_layer: 0,
        array_layer_count: None,
    });

    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("Block Sampler"),
        address_mode_u: wgpu::AddressMode::Repeat,
        address_mode_v: wgpu::AddressMode::Repeat,
        address_mode_w: wgpu::AddressMode::Repeat,
        mag_filter: wgpu::FilterMode::Nearest,
        min_filter: wgpu::FilterMode::Nearest,
        mipmap_filter: wgpu::FilterMode::Nearest,
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
