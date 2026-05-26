# Web build

The web build runs in the browser via **WebGPU** (Chrome/Edge recommended).

## Prerequisites

- Rust toolchain with `wasm32-unknown-unknown` target:
  ```bash
  rustup target add wasm32-unknown-unknown
  ```
- [Trunk](https://trunkrs.dev/):
  ```bash
  cargo install trunk --locked
  ```

## Local dev

```bash
trunk serve --open
```

Open `http://127.0.0.1:8080`. Use the startup menu to:

1. **Generate from seed** (recommended) — enter a number or text seed (text uses Java's `String.hashCode`, like Minecraft) and click **Generate world**. Same seed always produces the same terrain. Loads instantly; no zip or workers required.
2. **Upload** a `.zip` of a Java world save folder (must contain `region/r.*.*.mca` files)
3. **Load from URL** — the zip must be hosted with CORS enabled

Seed mode uses Minecraft-style Perlin noise (low/high/selector octaves, sea level 63, oceans, beaches). It is **not** identical to Minecraft Java world generation — use zip import for real saves.

Example zip layout:

```
Basic_World/
  region/
    r.0.0.mca
    r.0.-1.mca
    ...
```

Zip your `saves/Basic_World` folder on Windows:

```powershell
Compress-Archive -Path saves/Basic_World -DestinationPath Basic_World.zip
```

## Production build

```bash
trunk build --release
```

Deploy the contents of `dist/` to any static host (GitHub Pages, Cloudflare Pages, etc.).

## Web tuning

On `wasm32`, the engine uses a reduced profile:

- **Seed worlds** — chunks generated on the main thread (fast bootstrap, no workers)
- **Zip import** — optional **2 Web Workers** load chunks off the main thread during streaming (after bootstrap)
- Stream distance: **16 chunks** (desktop: 40)
- Section draw distance: **16 chunks** (desktop: 40)
- Bootstrap radius: **3 chunks** (desktop: 8)

Use a **release build** for playable performance:

```bash
trunk serve --release --open
```

Adjust draw distance at runtime with `[` and `]` after the world loads.

## Controls

- Click the canvas to capture the mouse (web only)
- **WASD** — move, **Space/Shift** — up/down
- **P** — release mouse
- Mouse — look
