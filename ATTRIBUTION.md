# Attribution — block textures

Every block texture this engine renders comes from the resource pack in `resource_pack/`:

## Nebula (V12.2, for MC 1.20+)

Compiled and largely reworked by **@drathmorgh**.

- https://twitch.tv/drathmorgh
- https://patreon.com/drathmorgh
- https://streamelements.com/drathmorgh/tip

Used under the terms stated in the pack's own readme (`resource_pack/Don't read me!.rtf`):

> You are welcome to use assets from this pack! If you do tho, please provide credit and a link
> to the original creators in the readme document you use with your pack.

This file is that credit. If you fork this repository, keep it.

### Creators whose work the pack incorporates

The pack's author credits the following people for assets and inspiration, so the credit carries
through to here:

- **fWhip** — https://youtube.com/c/fWhip
- **jermsyboy** — https://twitch.tv/jermsyboy — shooting stars and sunflares
- **John Smith Legacy** — https://johnsmithlegacy.co.uk/ — podzol overlay, some double slabs,
  biome carpet colour
- **bowtiedaniels** — https://twitch.tv/bowtiedaniels — diamond armour UI display, brown golden
  carrot tops, sounds
- **stennos** — https://twitch.tv/stennos / https://stennos.com
- **Vader's Alternative Block Pack** — https://planetminecraft.com/texture-pack/ — saplings,
  vines and a few others
- **Randomised Undead Texture Pack (RUD)** — https://minecraftic.com/ — Poorly Squid, sealanterns
- **TheBlackCladWanderer** — help with understanding the code

### ⚠️ One unresolved provenance note

The pack's author states that the **RUD** textures are used *without* a reply to a permission
request:

> RUD textures [have] not been updated since [...] I have tried contacting the site to ask for
> permission. [...] years, not a response. There are other packs out there using the same files.
> If you are the creator and don't want me to use them PLEASE let me know and I'll remove them!

So that subset has unclear licensing, inherited from the pack. It is recorded here rather than
quietly relied upon. If this project is ever published, that is the part to resolve or replace —
the affected textures (Poorly Squid, sealanterns) are not ones this engine currently renders.

## What is *not* from the pack

The pack is texture-only and does not ship every texture the engine needs. Two things are
generated in code from the pack's own art rather than taken from anywhere else:

- **`grass_block_top`** — absent from the pack, so `texture::load_grass_top_layer` synthesises it
  from the green rows of `grass_block_side.png` (hue) modulated by `grass_block_side_overlay.png`
  (detail).
- **Biome tints for water and foliage** — the pack ships no `colormap/`, so
  `texture::apply_water_tint` / `apply_foliage_tint` multiply the pack's grayscale masks by
  constants in `texture.rs`.

Thirteen block textures the parity generator can emit are absent from the pack entirely
(`sandstone`, `red_sand`, `mycelium_top`, `calcite`, `packed_ice`, `powder_snow` and the seven
terracottas). Those render as flat fallback colours defined in `texture::create_block_textures`.

No Mojang assets are included in this repository.

## Project notice

Voxel Engine is an unofficial fan project and is not affiliated with, endorsed by, or a
substitute for Mojang Studios or Microsoft. Minecraft is a trademark of Mojang Synergies AB.
