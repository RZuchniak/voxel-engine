# Voxel Engine

A WebGPU voxel flyer written in Rust. Generate procedural overworld-style terrain from a seed, or import an Anvil-format world zip and fly through it in the browser or natively.

**Unofficial fan project.** Not affiliated with, endorsed by, or a substitute for Mojang Studios or Microsoft. Minecraft is a trademark of Mojang Synergies AB.

## Quick start

### Native

```bash
# Optional: pick a seed (demo seed is used when unset)
VOXEL_SEED=12345 cargo run --release
```

PowerShell:

```powershell
$env:VOXEL_SEED="12345"; cargo run --release
```

### Web

See [WEB.md](WEB.md) for prerequisites. Release build recommended:

```bash
trunk serve --release --open
```

Then generate from a seed in the startup menu, or upload a world zip.

## Controls

- **WASD** — move
- **Space / Shift** — up / down
- **Mouse** — look (click the canvas on web to capture)
- **P** — release mouse
- **[** / **]** — adjust draw distance

## Textures

Block textures come from the **Nebula** resource pack by [@drathmorgh](https://www.planetminecraft.com/texture-pack/nebula-5609045/). Full credit and licence notes: [ATTRIBUTION.md](ATTRIBUTION.md).

## Licence / notice

This repository does not include Mojang assets. Keep third-party texture attribution when forking.
