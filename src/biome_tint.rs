//! Per-biome grass / foliage / water colour — the thing that makes a Minecraft screenshot
//! readable at a glance.
//!
//! Until this existed the renderer had one green and one blue for the whole world, so a swamp,
//! a jungle, a savanna, a badlands and a snowy plains all came out the same colour. The
//! generator already knows the biome exactly (`mc::biome` matches Mojang's own datagen dump box
//! for box); it was simply discarded on the way to the mesher.
//!
//! # How a tint reaches a fragment
//!
//! A quad carries an 8-bit **palette index** packed into spare bits of its existing `light`
//! word (`mesh::TINT_SHIFT`), so this costs no extra vertex bandwidth. The index is
//! `1 + biome * 3 + kind`, with 0 reserved for [`NEUTRAL`] (white — untinted blocks, which is
//! most of them). The shader looks the colour up in a uniform array and multiplies.
//!
//! # ⚠️ These colours are hand-authored, not ported
//!
//! Everything in `mc::` is validated against decompiled 26.2 and is bit-exact. **This table is
//! not**, and it is the only part of the render path that claims a Minecraft number without a
//! parity harness behind it. The values below are the widely-published vanilla colours.
//!
//! Getting these bit-exact would mean porting two more things:
//!
//! 1. `Biome.ClimateSettings` (`temperature` / `downfall` per biome) plus
//!    `BiomeSpecialEffects.grassColorModifier` — the `SWAMP`, `DARK_FOREST` and `BADLANDS`
//!    branches, and the explicit `grass_color` / `foliage_color` / `water_color` overrides.
//!    That part is *code*, so it ports the same way the rest of `mc::` did.
//! 2. `grass.png` / `foliage.png` under `assets/minecraft/textures/colormap/` — indexed by
//!    `(temperature, downfall)`. Those are **Mojang art**, which this project deliberately does
//!    not ship (see `ATTRIBUTION.md`), so they would have to be approximated by a fitted
//!    function rather than copied.
//!
//! Because of (2) a fully exact port is not reachable on the current terms, which is why this
//! is a table. Structured so it can be replaced wholesale: swap the array, keep the indices.

/// What a face's colour is modulated by. Mirrors `block::BlockInfo::tints`, which is per-face —
/// a grass block's top and side are tinted while its bottom (plain dirt) is not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TintKind {
    /// No modulation; the texture's own colour is final.
    None,
    Grass,
    Foliage,
    Water,
}

impl TintKind {
    /// Slot within a biome's three palette entries.
    #[inline]
    fn slot(self) -> Option<u8> {
        match self {
            TintKind::None => None,
            TintKind::Grass => Some(0),
            TintKind::Foliage => Some(1),
            TintKind::Water => Some(2),
        }
    }
}

/// The palette entry for "multiply by nothing" — pure white. Every untinted face uses it, which
/// is the overwhelming majority, so it is deliberately index 0: a zeroed light word is neutral.
pub const NEUTRAL: u8 = 0;

/// `(name, grass, foliage, water)` as sRGB hex, in the order `mc::biome` can emit.
///
/// Ordering is not load-bearing — [`biome_index`] resolves by name — but keeping it alphabetical
/// makes it diffable against the biome list.
const BIOME_TINTS: &[(&str, u32, u32, u32)] = &[
    // name                        grass     foliage   water
    ("badlands",                   0x90814D, 0x9E814D, 0x3F76E4),
    ("bamboo_jungle",              0x59C93C, 0x30BB0B, 0x3F76E4),
    ("beach",                      0x91BD59, 0x77AB2F, 0x3F76E4),
    ("birch_forest",               0x88BB67, 0x6BA941, 0x3F76E4),
    ("cherry_grove",               0xB6DB61, 0xB6DB61, 0x5DB7EF),
    ("cold_ocean",                 0x8EB971, 0x71A74D, 0x3D57D6),
    // `DARK_FOREST` grass modifier: `(c & 0xFEFEFE) + 0x28340A >> 1`, applied to 0x79C05A.
    ("dark_forest",                0x507A32, 0x59AE30, 0x3F76E4),
    ("deep_cold_ocean",            0x8EB971, 0x71A74D, 0x3D57D6),
    ("deep_dark",                  0x91BD59, 0x77AB2F, 0x3F76E4),
    ("deep_frozen_ocean",          0x80B497, 0x60A17B, 0x3938C9),
    ("deep_lukewarm_ocean",        0x8EB971, 0x71A74D, 0x45ADF2),
    ("deep_ocean",                 0x8EB971, 0x71A74D, 0x3F76E4),
    ("desert",                     0xBFB755, 0xAEA42A, 0x3F76E4),
    ("dripstone_caves",            0x91BD59, 0x77AB2F, 0x3F76E4),
    ("eroded_badlands",            0x90814D, 0x9E814D, 0x3F76E4),
    ("flower_forest",              0x79C05A, 0x59AE30, 0x3F76E4),
    ("forest",                     0x79C05A, 0x59AE30, 0x3F76E4),
    ("frozen_ocean",               0x80B497, 0x60A17B, 0x3938C9),
    ("frozen_peaks",               0x80B497, 0x60A17B, 0x3F76E4),
    ("frozen_river",               0x80B497, 0x60A17B, 0x3938C9),
    ("grove",                      0x80B497, 0x60A17B, 0x3F76E4),
    ("ice_spikes",                 0x80B497, 0x60A17B, 0x3F76E4),
    ("jagged_peaks",               0x80B497, 0x60A17B, 0x3F76E4),
    ("jungle",                     0x59C93C, 0x30BB0B, 0x3F76E4),
    ("lukewarm_ocean",             0x8EB971, 0x71A74D, 0x45ADF2),
    ("lush_caves",                 0x8EB971, 0x71A74D, 0x3F76E4),
    // Swamp overrides grass *and* foliage outright, and is the most recognisable tint in the game.
    ("mangrove_swamp",             0x6A7039, 0x8DB127, 0x3A7A6A),
    ("meadow",                     0x83BB6D, 0x63A948, 0x3F76E4),
    ("mushroom_fields",            0x55C93F, 0x2BBB0F, 0x3F76E4),
    ("ocean",                      0x8EB971, 0x71A74D, 0x3F76E4),
    ("old_growth_birch_forest",    0x88BB67, 0x6BA941, 0x3F76E4),
    ("old_growth_pine_taiga",      0x86B87F, 0x68A55F, 0x3F76E4),
    ("old_growth_spruce_taiga",    0x86B783, 0x68A464, 0x3F76E4),
    ("pale_garden",                0x778272, 0x878D76, 0x3F76E4),
    ("plains",                     0x91BD59, 0x77AB2F, 0x3F76E4),
    ("river",                      0x8EB971, 0x71A74D, 0x3F76E4),
    ("savanna",                    0xBFB755, 0xAEA42A, 0x3F76E4),
    ("savanna_plateau",            0xBFB755, 0xAEA42A, 0x3F76E4),
    ("snowy_beach",                0x83B593, 0x64A278, 0x3F76E4),
    ("snowy_plains",               0x80B497, 0x60A17B, 0x3F76E4),
    ("snowy_slopes",               0x80B497, 0x60A17B, 0x3F76E4),
    ("snowy_taiga",                0x80B497, 0x60A17B, 0x205E78),
    ("sparse_jungle",              0x64C73F, 0x3EB80F, 0x3F76E4),
    ("stony_peaks",                0x9ABE4B, 0x82AC1E, 0x3F76E4),
    ("stony_shore",                0x8AB689, 0x6DA36B, 0x3F76E4),
    // 26.x-era cave biome; no published colours found, so it inherits the cave default. If it
    // ever renders visibly wrong, this row is the reason.
    ("sulfur_caves",               0x91BD59, 0x77AB2F, 0x3F76E4),
    ("sunflower_plains",           0x91BD59, 0x77AB2F, 0x3F76E4),
    ("swamp",                      0x6A7039, 0x6A7039, 0x617B64),
    ("taiga",                      0x86B783, 0x68A464, 0x3F76E4),
    ("warm_ocean",                 0x8EB971, 0x71A74D, 0x43D5EE),
    ("windswept_forest",           0x8AB689, 0x6DA36B, 0x3F76E4),
    ("windswept_gravelly_hills",   0x8AB689, 0x6DA36B, 0x3F76E4),
    ("windswept_hills",            0x8AB689, 0x6DA36B, 0x3F76E4),
    ("windswept_savanna",          0xBFB755, 0xAEA42A, 0x3F76E4),
    ("wooded_badlands",            0x90814D, 0x9E814D, 0x3F76E4),
];

/// Biome used when a name is not in the table — plains, the least surprising default.
/// A miss is a bug (the generator's biome set is closed and known), so it is also asserted
/// against in the tests.
const FALLBACK_BIOME: &str = "plains";

/// Biome assumed for chunks that carry no biome grid at all — the Anvil zip-import path.
///
/// This must not be "no tint". The tintable texture layers are neutralised to grayscale at load
/// (`texture::neutralise_tintable_layer`) on the assumption that a biome supplies their colour, so
/// an untinted grass block is a *grey* grass block. Defaulting to plains reproduces the single
/// global green and blue those layers used to have baked in, which is exactly how imported worlds
/// rendered before biome tint existed.
pub const DEFAULT_BIOME: u8 = {
    // `position` is not const-callable, so this is a hand-rolled const scan. Pinned by
    // `the_default_biome_is_plains`.
    let mut i = 0;
    let mut found = 0;
    while i < BIOME_TINTS.len() {
        let name = BIOME_TINTS[i].0.as_bytes();
        if name.len() == 6
            && name[0] == b'p'
            && name[1] == b'l'
            && name[2] == b'a'
            && name[3] == b'i'
            && name[4] == b'n'
            && name[5] == b's'
        {
            found = i;
        }
        i += 1;
    }
    found as u8
};

/// Entries in the uniform palette: one neutral, then three per biome.
pub const PALETTE_LEN: usize = 1 + BIOME_TINTS.len() * 3;

/// Index of a biome within [`BIOME_TINTS`], by registry name (no `minecraft:` prefix).
pub fn biome_index(name: &str) -> u8 {
    match BIOME_TINTS.iter().position(|(n, ..)| *n == name) {
        Some(index) => index as u8,
        None => BIOME_TINTS
            .iter()
            .position(|(n, ..)| *n == FALLBACK_BIOME)
            .expect("fallback biome is in the table") as u8,
    }
}

/// The palette slot a face takes, given the biome it sits in and what it is made of.
///
/// Untinted faces collapse to [`NEUTRAL`] regardless of biome — which is what keeps greedy
/// meshing from breaking runs of stone at every biome boundary (see `mesh::mesh_direction`).
#[inline]
pub fn palette_index(biome: u8, kind: TintKind) -> u8 {
    match kind.slot() {
        None => NEUTRAL,
        Some(slot) => 1 + biome * 3 + slot,
    }
}

/// sRGB byte → linear float.
///
/// The block texture array is `Rgba8UnormSrgb`, so a sample is already decoded to linear by the
/// time the shader multiplies by a tint. Feeding it a raw sRGB constant would over-brighten every
/// tinted face; the exact transfer function is cheap enough to just do properly here, once, at
/// load. (Alpha is *not* sRGB-encoded in that format, which is why the per-texel tint mask can
/// live there untouched.)
fn srgb_to_linear(channel: u8) -> f32 {
    let c = channel as f32 / 255.0;
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

fn unpack(hex: u32) -> [f32; 4] {
    [
        srgb_to_linear(((hex >> 16) & 0xFF) as u8),
        srgb_to_linear(((hex >> 8) & 0xFF) as u8),
        srgb_to_linear((hex & 0xFF) as u8),
        1.0,
    ]
}

/// The palette as the shader consumes it: linear RGBA, indexed by [`palette_index`].
pub fn palette() -> Vec<[f32; 4]> {
    let mut out = Vec::with_capacity(PALETTE_LEN);
    out.push([1.0, 1.0, 1.0, 1.0]); // NEUTRAL
    for (_, grass, foliage, water) in BIOME_TINTS {
        out.push(unpack(*grass));
        out.push(unpack(*foliage));
        out.push(unpack(*water));
    }
    debug_assert_eq!(out.len(), PALETTE_LEN);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The whole scheme rests on the index fitting in the 8 bits `mesh` reserves for it. If a
    /// future Minecraft version adds biomes past 85, this fails here rather than silently
    /// wrapping and tinting grass with someone else's water colour.
    #[test]
    fn every_palette_index_fits_in_a_byte() {
        assert!(
            PALETTE_LEN <= 256,
            "palette has {PALETTE_LEN} entries but the light word reserves 8 bits for it"
        );
        for biome in 0..BIOME_TINTS.len() as u8 {
            for kind in [TintKind::Grass, TintKind::Foliage, TintKind::Water] {
                let index = palette_index(biome, kind);
                assert!(
                    (index as usize) < PALETTE_LEN,
                    "biome {biome} {kind:?} -> {index}, past the palette"
                );
            }
        }
    }

    /// Every biome the generator can emit must have a row here. `mc::biome` produces a closed
    /// set of names, so a miss is a typo, not a missing feature — and it would show as one biome
    /// silently rendering with plains' colours, which is exactly the bug this feature exists to
    /// remove.
    #[test]
    fn every_biome_the_generator_emits_has_a_tint() {
        let mut missing: Vec<&str> = Vec::new();
        for (point, name) in crate::mc::biome::overworld_biomes() {
            let _ = point;
            if !BIOME_TINTS.iter().any(|(n, ..)| *n == name) {
                if !missing.contains(&name) {
                    missing.push(name);
                }
            }
        }
        assert!(missing.is_empty(), "biomes with no tint row: {missing:?}");
    }

    /// Untinted faces must land on NEUTRAL whatever biome they are in — otherwise a run of stone
    /// would break into a separate quad at every biome boundary for no visual gain.
    #[test]
    fn untinted_faces_are_biome_independent() {
        for biome in 0..BIOME_TINTS.len() as u8 {
            assert_eq!(palette_index(biome, TintKind::None), NEUTRAL);
        }
    }

    /// Neutral must be exactly white, or every untinted block in the world shifts colour.
    #[test]
    fn the_neutral_entry_does_not_change_a_texture() {
        assert_eq!(palette()[NEUTRAL as usize], [1.0, 1.0, 1.0, 1.0]);
    }

    /// The const scan behind [`DEFAULT_BIOME`] must actually find plains.
    ///
    /// It backs the Anvil import path, where getting it wrong means grey grass and grey water in
    /// every imported world rather than an obvious crash.
    #[test]
    fn the_default_biome_is_plains() {
        assert_eq!(DEFAULT_BIOME, biome_index("plains"));
        assert_eq!(BIOME_TINTS[DEFAULT_BIOME as usize].0, "plains");
    }

    /// Guards the sRGB→linear step. Skipping it is an easy mistake that looks *almost* right —
    /// the tints simply come out too bright — so pin a known value rather than trusting the eye.
    #[test]
    fn tints_are_converted_to_linear() {
        let plains = palette()[palette_index(biome_index("plains"), TintKind::Grass) as usize];
        // 0x91BD59: sRGB 0x91 = 145/255 = 0.569 → linear ≈ 0.2831.
        assert!(
            (plains[0] - 0.2831).abs() < 0.001,
            "plains grass red channel {} is not the linear form of 0x91",
            plains[0]
        );
        assert!(plains[1] > plains[0] && plains[1] > plains[2], "grass must be green-dominant");
    }

    /// Swamp is the strongest visual signature in the table and the one most likely to be lost
    /// to a bad merge — it is the only biome whose grass is *browner* than it is green.
    #[test]
    fn swamp_is_not_the_default_green() {
        let swamp = palette()[palette_index(biome_index("swamp"), TintKind::Grass) as usize];
        let plains = palette()[palette_index(biome_index("plains"), TintKind::Grass) as usize];
        assert_ne!(swamp, plains, "swamp grass must not equal plains grass");
    }
}
