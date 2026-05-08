#[cfg(not(target_arch = "wasm32"))]
use std::path::PathBuf;

#[cfg(not(target_arch = "wasm32"))]
use anyhow::Context;

pub struct BlockTextureSet {
    pub bind_group_layout: wgpu::BindGroupLayout,
    pub bind_group: wgpu::BindGroup,
}

pub const TEXTURE_NAMES: &[&str] = &[
    "missing",
    "stone",
    "dirt",
    "grass_top",
    "grass_side",
    "water",
    "log",
    "leaves",
    "sand",
    "cobblestone",
    "oak_planks",
    "bedrock",
    "deepslate",
    "gravel",
    "snow",
    "netherrack",
    "end_stone",
];

fn load_or_fallback_rgba(name: &str, fallback: [u8; 4]) -> Vec<u8> {
    #[cfg(target_arch = "wasm32")]
    {
        let _ = name;
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
    let path = PathBuf::from("assets").join("blocks").join(format!("{name}.png"));
    if path.exists()
        && let Ok(img) = image::open(&path).context("failed to open texture")
    {
        let rgba = img.to_rgba8();
        let (w, h) = rgba.dimensions();
        if w == 16 && h == 16 {
            return rgba.into_raw();
        }
    }
    }

    // Fallback to solid 16x16 color if texture file doesn't exist yet.
    let mut out = vec![0u8; 16 * 16 * 4];
    for px in out.chunks_exact_mut(4) {
        px.copy_from_slice(&fallback);
    }
    out
}

pub fn create_block_textures(device: &wgpu::Device, queue: &wgpu::Queue) -> BlockTextureSet {
    let mut layers = Vec::with_capacity(TEXTURE_NAMES.len());
    for name in TEXTURE_NAMES {
        let fallback = match *name {
            "stone" => [128, 128, 128, 255],
            "dirt" => [120, 86, 58, 255],
            "grass_top" => [87, 168, 71, 255],
            "grass_side" => [96, 131, 75, 255],
            "water" => [45, 95, 220, 255],
            "log" => [110, 85, 58, 255],
            "leaves" => [60, 130, 60, 255],
            "sand" => [219, 211, 160, 255],
            "cobblestone" => [113, 113, 113, 255],
            "oak_planks" => [171, 141, 89, 255],
            "bedrock" => [69, 69, 69, 255],
            "deepslate" => [84, 84, 90, 255],
            "gravel" => [134, 131, 128, 255],
            "snow" => [236, 240, 246, 255],
            "netherrack" => [109, 52, 52, 255],
            "end_stone" => [220, 220, 171, 255],
            _ => [255, 0, 255, 255],
        };
        layers.push(load_or_fallback_rgba(name, fallback));
    }

    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("Block Texture Array"),
        size: wgpu::Extent3d {
            width: 16,
            height: 16,
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
                bytes_per_row: Some(16 * 4),
                rows_per_image: Some(16),
            },
            wgpu::Extent3d {
                width: 16,
                height: 16,
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
