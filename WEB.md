# Web build

The web build runs in the browser via **WebGPU** (Chrome/Edge recommended).

**Notice:** Voxel Engine is an unofficial, non-commercial fan project. It is not affiliated with, endorsed by, or a substitute for Mojang Studios or Microsoft. Minecraft is a trademark of Mojang Synergies AB. Do not present this app as an official Minecraft product.

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

1. **Generate from seed** (recommended) — enter a number or text seed (text seeds use a Java-style `String.hashCode`) and click **Generate world**. Same seed always produces the same terrain. Loads instantly; no zip or workers required.
2. **Upload** a `.zip` of an Anvil-format world save folder (must contain `region/r.*.*.mca` files)
3. **Load from URL** — the zip must be hosted with CORS enabled

Seed mode builds procedural overworld-style terrain for flying around. Prefer zip import when you want to view a specific saved world as stored on disk.

Example zip layout (folder name is arbitrary):

```
MyWorld/
  region/
    r.0.0.mca
    r.0.-1.mca
    ...
```

On Windows, zip a world folder that contains `region/`:

```powershell
Compress-Archive -Path path\to\MyWorld -DestinationPath MyWorld.zip
```

## Production build

```bash
trunk build --release
```

Deploy the contents of `dist/` to any static host (GitHub Pages, Cloudflare Pages, etc.).

## Web tuning

On `wasm32`, the engine uses a reduced profile:

- **Seed worlds** — chunks generated **and meshed** on Web Workers (up to 6)
- Stream distance: **12 chunks** (desktop: 20)
- Section draw distance: **12 chunks** (desktop: 20)
- Loading screen: **4 chunks** (81 tiles; desktop: 12 of 20). Override with `?loading_radius=<n>`
- Terrain: generate **down to bedrock**, skip empty sky. `?full_column=1` samples the whole `-64..320` column.

Use a **release build** for playable performance:

```bash
trunk serve --release --open
```

Adjust draw distance at runtime with `[` and `]` after the world loads.

### Browser debug overlay

Native is too fast to show streaming problems. In the tab:

- Open `http://127.0.0.1:8080/?debug=1` (or press **F3** in-game)
- Press **P** to release the mouse, then select the overlay text and paste it
- `?log=1` also prints the same `profile` line to the browser console once a second

The overlay's `new=/s` is the number to watch: that is how fast chunks actually appear. `workers queued / in_flight` stuck high with a low `new=/s` means generation, not FPS.

## Controls

- Click the canvas to capture the mouse (web only)
- **WASD** — move, **Space/Shift** — up/down
- **P** — release mouse
- **F3** — toggle the copyable debug overlay
- Mouse — look
