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
    // Tree species (`mc::tree`). This pack ships no `birch_log.png` / `spruce_log.png` side
    // texture — only `_top` and `stripped_` variants — so those two fall back to flat bark
    // colours. The leaves it does ship, and they are cutouts like oak's.
    "birch_log.png",             // 36
    "birch_leaves.png",          // 37
    "spruce_log.png",            // 38
    "spruce_leaves.png",         // 39
];

pub const BLOCK_TEXTURE_DIR: &str = "resource_pack/assets/minecraft/textures/block";

const TEX_SIZE: u32 = 16;
/// Mip chain for 16×16 tiles (16 → 8 → 4 → 2 → 1).
const BLOCK_MIP_LEVELS: u32 = 5;

/// Is alpha a binary stencil here, rather than absent or a constant?
///
/// Leaves are the case that matters: 0 or 255 only, ~60% covered. Water is *not* — `apply_water_tint`
/// forces every visible texel to 255 — and ice is a uniform 136, so neither takes the cutout path.
fn is_cutout(rgba: &[u8]) -> bool {
    let mut saw_clear = false;
    let mut saw_opaque = false;
    for px in rgba.chunks_exact(4) {
        match px[3] {
            0 => saw_clear = true,
            255 => saw_opaque = true,
            _ => return false,
        }
    }
    saw_clear && saw_opaque
}

/// A block texture layer plus how its alpha channel is to be read.
///
/// The flag cannot be recovered by inspecting pixels, which is why it is carried explicitly.
/// After [`neutralise_tintable_layer`] a grass side has alpha 0 on its dirt texels and 255 on its
/// grass texels — indistinguishable from a cutout stencil, and treating it as one would run the
/// `discard` test and punch the dirt rows out of every grass block in the world. Alpha means
/// *coverage* for a cutout and *tint mask* for a tintable layer, and only the loader knows which
/// it just wrote.
pub struct LoadedLayer {
    pub pixels: Vec<u8>,
    /// True when alpha is an opacity stencil (leaves); false when it is a per-texel tint mask.
    pub alpha_is_coverage: bool,
}

/// Downsample a 16×16 RGBA layer into mip levels (wgpu 26 has no CommandEncoder::generate_mipmap).
///
/// `alpha_is_coverage` selects the filter, and the distinction is load-bearing: a coverage
/// stencil must be downsampled by [`build_cutout_mip_chain`] to keep its holes open, while a tint
/// mask is a continuous quantity that *should* be averaged — a texel straddling a grass/dirt
/// boundary is genuinely half-tinted.
fn build_mip_chain(base: &[u8], alpha_is_coverage: bool) -> Vec<Vec<u8>> {
    if alpha_is_coverage && is_cutout(base) {
        return build_cutout_mip_chain(base);
    }
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

/// Mip chain for an alpha-tested layer, preserving opaque **coverage** at every level.
///
/// Averaging alpha destroys a cutout. A mip-1 texel covering one opaque and three clear texels
/// averages to alpha 64, which passes any sane alpha test — so *every hole closes at the first mip
/// transition*, which is a hard line at whatever distance minification begins. That is the reported
/// "the distance at which leaves become opaque is too close, and I can clearly see a line".
///
/// Instead, each level keeps exactly the same fraction of texels opaque as the base, choosing the
/// most-covered ones, and re-quantises alpha to 0/255 (coverage-preserving mipmapping, after
/// Castano). Staying binary also makes the result independent of the shader's alpha threshold, so
/// the two cannot drift apart.
///
/// Colour is averaged over **covered texels only**: including the clear ones would pull whatever is
/// behind the cutout into the average and ring a dark halo around every leaf edge at range.
fn build_cutout_mip_chain(base: &[u8]) -> Vec<Vec<u8>> {
    let texel_count = (TEX_SIZE * TEX_SIZE) as usize;
    let coverage =
        base.chunks_exact(4).filter(|px| px[3] > 0).count() as f32 / texel_count as f32;

    let mut mips = vec![base.to_vec()];
    let mut current = base.to_vec();
    let mut size = TEX_SIZE;

    while size > 1 {
        let next = size / 2;
        let total = (next * next) as usize;
        let mut level = vec![0u8; total * 4];
        let mut covered_fraction = vec![0f32; total];
        let mut mean = [0u32; 3];
        let mut mean_n = 0u32;

        for y in 0..next {
            for x in 0..next {
                let mut acc = [0u32; 3];
                let mut opaque = 0u32;
                for dy in 0..2 {
                    for dx in 0..2 {
                        let i = (((y * 2 + dy) * size + (x * 2 + dx)) * 4) as usize;
                        if current[i + 3] > 0 {
                            opaque += 1;
                            for c in 0..3 {
                                acc[c] += current[i + c] as u32;
                            }
                        }
                    }
                }
                let o = (y * next + x) as usize;
                covered_fraction[o] = opaque as f32 / 4.0;
                if opaque > 0 {
                    for c in 0..3 {
                        level[o * 4 + c] = (acc[c] / opaque) as u8;
                        mean[c] += acc[c] / opaque;
                    }
                    mean_n += 1;
                }
            }
        }

        // Rank by coverage and keep the base level's share. Ties break on index so the chain is
        // deterministic — a texture that changes between runs would be maddening to debug.
        let keep = ((coverage * total as f32).round() as usize).clamp(1, total);
        let mut order: Vec<usize> = (0..total).collect();
        order.sort_by(|&a, &b| {
            covered_fraction[b]
                .partial_cmp(&covered_fraction[a])
                .expect("coverage is never NaN")
                .then(a.cmp(&b))
        });
        for (rank, &o) in order.iter().enumerate() {
            let kept = rank < keep;
            level[o * 4 + 3] = if kept { 255 } else { 0 };
            // Kept but with no covered source: give it the level's mean colour rather than the
            // black it was initialised to.
            if kept && covered_fraction[o] == 0.0 && mean_n > 0 {
                for c in 0..3 {
                    level[o * 4 + c] = (mean[c] / mean_n) as u8;
                }
            }
        }

        mips.push(level.clone());
        current = level;
        size = next;
    }
    mips
}

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

/// Target mean brightness of a neutralised layer.
///
/// A tinted layer must be a *modulation field around 1.0*, not a colour: the biome tint supplies
/// the colour and the texture supplies only the detail. Normalising to exactly 1.0 would clip
/// every above-average texel, so the mean lands slightly below white — which is also where
/// vanilla's own grayscale grass art sits.
const NEUTRAL_MEAN: f32 = 0.85;

#[inline]
fn luminance_of(px: &[u8]) -> f32 {
    px[0] as f32 * 0.299 + px[1] as f32 * 0.587 + px[2] as f32 * 0.114
}

/// Strip a tintable layer's baked-in colour, leaving a grayscale detail field, and record in
/// **alpha** which texels the biome tint applies to.
///
/// Two things forced this. First, the pack's grass and foliage art is *already green*, so the old
/// code baked a fixed green in at load (`apply_foliage_tint`/`apply_water_tint`, now gone) —
/// multiplying that by a biome colour would apply the hue twice and come out dark and muddy.
/// Second, `grass_block_side.png` is a single baked tile of grass over dirt, so tinting whole
/// texels would turn its dirt rows green.
///
/// Both are solved by one encoding: RGB becomes normalised luminance, and alpha becomes a
/// per-texel **tint mask** (255 = take the biome colour, 0 = keep the texture's own colour). The
/// shader blends between them, so a grass side's dirt rows stay brown while its grass rows follow
/// the biome.
///
/// ⚠️ Repurposing alpha is only safe because nothing blends (`BlendState::REPLACE`) and these
/// layers are not cutouts. Leaves *are* a cutout — alpha there is coverage and drives `discard` —
/// so they take [`neutralise_cutout_colour`] and are tinted uniformly instead.
///
/// `tint_green_only` restricts the mask to green-dominant texels, which is what separates a grass
/// side's grass from its dirt. Layers that are wholly tintable (water, the synthesised grass top)
/// pass `false` and get a full mask.
fn neutralise_tintable_layer(pixels: &mut [u8], tint_green_only: bool) {
    let is_tintable = |px: &[u8]| -> bool {
        if !tint_green_only {
            return true;
        }
        // Green-dominant by a clear margin — the per-texel form of the row test
        // `load_grass_top_layer` uses to find this same pack's grass rows.
        px[1] as i32 > px[0] as i32 + 12 && px[1] as i32 > px[2] as i32 + 12
    };

    // Mean over the texels that will actually be tinted. Including the untinted ones would let a
    // grass side's dark dirt rows drag its grass rows brighter to compensate.
    let mut sum = 0f32;
    let mut count = 0u32;
    for px in pixels.chunks_exact(4) {
        if px[3] >= 10 && is_tintable(px) {
            sum += luminance_of(px);
            count += 1;
        }
    }
    if count == 0 {
        return;
    }
    let mean = sum / count as f32;
    if mean <= 0.0 {
        return;
    }

    for px in pixels.chunks_exact_mut(4) {
        if px[3] < 10 {
            px.copy_from_slice(&[0, 0, 0, 0]);
            continue;
        }
        if !is_tintable(px) {
            px[3] = 0; // untinted: the shader keeps this texel's own colour
            continue;
        }
        let level = ((luminance_of(px) / mean) * NEUTRAL_MEAN * 255.0).clamp(0.0, 255.0) as u8;
        px[0] = level;
        px[1] = level;
        px[2] = level;
        px[3] = 255;
    }
}

/// Neutralise a cutout layer's colour while leaving its alpha stencil intact.
///
/// Same idea as [`neutralise_tintable_layer`] minus the mask: a cutout's alpha is already spoken
/// for by the `discard` test, so these layers are tinted uniformly and the shader is told which
/// they are via [`cutout_layer_mask`].
fn neutralise_cutout_colour(pixels: &mut [u8]) {
    let mut sum = 0f32;
    let mut count = 0u32;
    for px in pixels.chunks_exact(4) {
        if px[3] > 0 {
            sum += luminance_of(px);
            count += 1;
        }
    }
    if count == 0 {
        return;
    }
    let mean = sum / count as f32;
    if mean <= 0.0 {
        return;
    }
    for px in pixels.chunks_exact_mut(4) {
        if px[3] == 0 {
            continue;
        }
        let level = ((luminance_of(px) / mean) * NEUTRAL_MEAN * 255.0).clamp(0.0, 255.0) as u8;
        px[0] = level;
        px[1] = level;
        px[2] = level;
    }
}

/// Which texture layers are alpha-tested cutouts, as a bitmask the shader can index.
///
/// The shader needs this to know how to read a layer's alpha: for a cutout it is coverage (run
/// the `discard`, tint the whole texel), for everything else it is the per-texel tint mask (no
/// discard, blend towards the biome colour). Derived from the loaded pixels rather than
/// hardcoding layer 7, so a pack shipping a cutout for some other block cannot desync the two.
///
/// Four `u32`s rather than a `u64` because WGSL uniforms pad to 16 bytes anyway; 128 bits covers
/// the 36 layers with room to spare.
/// The fragment shader's `TintUniform`: the biome palette followed by the cutout bitmask.
///
/// Laid out by hand rather than through a `#[repr(C)]` struct because the palette is a 166-entry
/// array and the two members are both 16-byte aligned, so a flat concatenation already matches
/// WGSL's uniform layout rules for `array<vec4<f32>, N>` + `vec4<u32>`.
///
/// Cost: **one ~2.7 KB uniform buffer for the whole world**, written once at startup. Biome
/// colour adds no per-chunk, per-frame or per-vertex allocation anywhere — a quad carries an
/// 8-bit index into this, packed into vertex bits that were already being paid for.
fn create_tint_uniform(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    layers: &[LoadedLayer],
) -> wgpu::Buffer {
    let palette = crate::biome_tint::palette();
    let mut bytes: Vec<u8> = Vec::with_capacity(palette.len() * 16 + 16);
    for entry in &palette {
        bytes.extend_from_slice(bytemuck::cast_slice(entry));
    }
    bytes.extend_from_slice(bytemuck::cast_slice(&cutout_layer_mask(layers)));

    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("Biome Tint Uniform"),
        size: bytes.len() as u64,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    queue.write_buffer(&buffer, 0, &bytes);
    buffer
}

pub fn cutout_layer_mask(layers: &[LoadedLayer]) -> [u32; 4] {
    let mut mask = [0u32; 4];
    for (index, layer) in layers.iter().enumerate().take(128) {
        if layer.alpha_is_coverage {
            mask[index / 32] |= 1 << (index % 32);
        }
    }
    mask
}

fn load_layer_by_index(layer: usize, pack_file: &str, fallback: [u8; 4]) -> LoadedLayer {
    let mut pixels = match layer {
        3 => load_grass_top_layer(fallback),
        _ => load_raw_layer(pack_file, fallback),
    };
    // Whether alpha is a coverage stencil is decided from the layer as loaded, *before* any
    // neutralisation rewrites it — afterwards a tint mask looks exactly like a stencil.
    let mut alpha_is_coverage = is_cutout(&pixels);
    match layer {
        // Grass top: wholly grass, so the whole tile takes the biome colour.
        3 => {
            neutralise_tintable_layer(&mut pixels, false);
            alpha_is_coverage = false;
        }
        // Grass side: grass over dirt in one tile, so only its green texels are tinted and the
        // rest keep their own colour. This is the layer that makes the flag necessary.
        4 => {
            neutralise_tintable_layer(&mut pixels, true);
            alpha_is_coverage = false;
        }
        // Water: wholly tintable.
        5 => {
            neutralise_tintable_layer(&mut pixels, false);
            alpha_is_coverage = false;
        }
        // Leaves: a cutout, so the mask cannot live in alpha — tinted uniformly instead.
        7 => neutralise_cutout_colour(&mut pixels),
        _ => {}
    }
    LoadedLayer { pixels, alpha_is_coverage }
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

    /// A cutout's mip chain must stay binary and keep its coverage at every level.
    ///
    /// The bug: the shared chain averages alpha, so a mip-1 texel covering one opaque and three
    /// clear texels became alpha 64 — solid under any sane alpha test. Every hole therefore closed
    /// at the *first* mip transition, giving a hard line at whatever distance minification starts.
    #[test]
    fn a_cutout_mip_chain_keeps_binary_alpha_and_its_coverage() {
        // A synthetic checkerboard rather than the pack, so this holds without `resource_pack/`
        // and pins the property rather than one texture's numbers.
        let mut base = vec![0u8; (TEX_SIZE * TEX_SIZE * 4) as usize];
        for y in 0..TEX_SIZE {
            for x in 0..TEX_SIZE {
                let i = ((y * TEX_SIZE + x) * 4) as usize;
                base[i] = 40;
                base[i + 1] = 160;
                base[i + 2] = 60;
                base[i + 3] = if (x + y) % 2 == 0 { 255 } else { 0 };
            }
        }
        assert!(is_cutout(&base), "the fixture must take the cutout path");

        let mips = build_mip_chain(&base, true);
        assert_eq!(mips.len(), BLOCK_MIP_LEVELS as usize);
        let base_coverage = 0.5; // exact, by construction

        for (level, data) in mips.iter().enumerate() {
            let size = (TEX_SIZE >> level).max(1);
            let texels = (size * size) as usize;
            assert_eq!(data.len(), texels * 4, "mip {level} is the wrong size");

            let mut opaque = 0usize;
            for px in data.chunks_exact(4) {
                assert!(
                    px[3] == 0 || px[3] == 255,
                    "mip {level} has alpha {} — averaging alpha is what filled the holes in",
                    px[3]
                );
                if px[3] == 255 {
                    opaque += 1;
                }
            }
            // Exact where the count divides evenly; rounding costs at most one texel.
            let expected = (base_coverage * texels as f32).round() as usize;
            assert!(
                opaque.abs_diff(expected) <= 1,
                "mip {level} keeps {opaque}/{texels} opaque, expected ~{expected}"
            );
        }
    }

    /// A cutout mip must not tint towards whatever sits behind the holes.
    #[test]
    fn a_cutout_mip_takes_its_colour_only_from_covered_texels() {
        // Opaque texels are pure green; the clear ones are black. Averaging all four would drag the
        // green down by half at every level and ring dark halos around the leaves at range.
        let mut base = vec![0u8; (TEX_SIZE * TEX_SIZE * 4) as usize];
        for y in 0..TEX_SIZE {
            for x in 0..TEX_SIZE {
                let i = ((y * TEX_SIZE + x) * 4) as usize;
                if (x + y) % 2 == 0 {
                    base[i + 1] = 200;
                    base[i + 3] = 255;
                }
            }
        }
        let mips = build_mip_chain(&base, true);
        let mut checked = 0usize;
        for (level, data) in mips.iter().enumerate().skip(1) {
            for px in data.chunks_exact(4).filter(|px| px[3] == 255) {
                checked += 1;
                assert_eq!(
                    px[1], 200,
                    "mip {level} green dropped to {} — the clear texels are polluting the average",
                    px[1]
                );
            }
        }
        // Without this the test passes vacuously on the averaging chain: no texel comes out at
        // exactly 255 alpha there, so the filter matches nothing and the loop never runs.
        assert!(checked > 0, "no opaque mip texels were examined");
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

/// Load one block texture layer exactly as [`create_block_textures`] does — same file, same
/// fallback colour, same neutralisation.
///
/// `pub` for `examples/diag_texture_probe`, which reproduces the fragment shader's colour maths
/// on these bytes. Biome tint is generated art plus a colour table, and the only way to judge
/// either is to look at the result; going through the real loader is what makes the dump
/// trustworthy rather than an approximation of it.
pub fn load_block_layer(layer: usize) -> LoadedLayer {
    let pack_file = BLOCK_TEXTURE_FILES
        .get(layer)
        .copied()
        .unwrap_or(BLOCK_TEXTURE_FILES[0]);
    load_layer_by_index(layer, pack_file, fallback_for(pack_file))
}

/// Flat colour used when a pack does not ship a texture. Several of the parity generator's
/// blocks are absent from this pack entirely and render as these.
fn fallback_for(pack_file: &str) -> [u8; 4] {
    match pack_file {
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
        // Birch/spruce bark: absent from this pack, so these are the operative colours rather
        // than a last-resort magenta.
        "birch_log.png" => [216, 214, 207, 255],
        "spruce_log.png" => [88, 63, 34, 255],
        "birch_leaves.png" => [128, 167, 85, 255],
        "spruce_leaves.png" => [97, 130, 97, 255],
        _ => [255, 0, 255, 255],
    }
}

pub fn create_block_textures(device: &wgpu::Device, queue: &wgpu::Queue) -> BlockTextureSet {
    let mut layers = Vec::with_capacity(BLOCK_TEXTURE_FILES.len());
    for layer in 0..BLOCK_TEXTURE_FILES.len() {
        layers.push(load_block_layer(layer));
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

    for (layer, loaded) in layers.iter().enumerate() {
        for (mip_level, mip_data) in build_mip_chain(&loaded.pixels, loaded.alpha_is_coverage)
            .into_iter()
            .enumerate()
        {
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
            wgpu::BindGroupLayoutEntry {
                binding: 2,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
        ],
    });

    let tint_buffer = create_tint_uniform(device, queue, &layers);

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
            wgpu::BindGroupEntry {
                binding: 2,
                resource: tint_buffer.as_entire_binding(),
            },
        ],
    });

    BlockTextureSet {
        bind_group_layout,
        bind_group,
    }
}
