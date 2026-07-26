//! What is actually *in* a pack texture? Prints per-row opacity and mean colour.
//!
//! Exists because the resource pack's PNGs are awkward to inspect with anything else: every file
//! carries a `zTXt` chunk that Pillow refuses to open, and several (`grass_0.png` and friends) are
//! Adam7-interlaced. The `image` crate the engine already depends on reads all of them, so the
//! cheapest reliable reader is this.
//!
//! ```
//! cargo run --release --example diag_texture_probe -- grass_0.png grass_block_side.png
//! ```
//! Paths are resolved under `texture::BLOCK_TEXTURE_DIR` unless they contain a separator.

use voxel_engine::texture::{load_grass_top_layer, BLOCK_TEXTURE_DIR};

/// 16x16 is too small to judge by eye, so dumps are nearest-neighbour upscaled.
const DUMP_SCALE: u32 = 16;

fn dump_layer(rgba: &[u8], out: &str) {
    let size = 16u32;
    let big = size * DUMP_SCALE;
    let mut img = image::RgbaImage::new(big, big);
    for y in 0..big {
        for x in 0..big {
            let i = (((y / DUMP_SCALE) * size + (x / DUMP_SCALE)) * 4) as usize;
            img.put_pixel(x, y, image::Rgba([rgba[i], rgba[i + 1], rgba[i + 2], rgba[i + 3]]));
        }
    }
    img.save(out).expect("write dump");
    println!("wrote {out}");
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        eprintln!("usage: diag_texture_probe <file.png> [more.png ...]");
        eprintln!("       diag_texture_probe --grass-top <out.png>");
        std::process::exit(2);
    }

    // The grass top is synthesised, not loaded, so it has no file to probe.
    if args[0] == "--grass-top" {
        let out = args.get(1).map(String::as_str).unwrap_or("grass_top.png");
        dump_layer(&load_grass_top_layer([120, 170, 80, 255]), out);
        return;
    }

    for arg in &args {
        let path = if arg.contains('/') || arg.contains('\\') {
            std::path::PathBuf::from(arg)
        } else {
            std::path::Path::new(BLOCK_TEXTURE_DIR).join(arg)
        };
        let img = match image::open(&path) {
            Ok(img) => img.to_rgba8(),
            Err(err) => {
                println!("== {} -- FAILED: {err}", path.display());
                continue;
            }
        };
        let (w, h) = img.dimensions();
        println!("== {} {w}x{h}", path.display());

        let mut fully_opaque_rows = 0;
        let mut fully_clear_rows = 0;
        for y in 0..h {
            let mut opaque = 0u32;
            let (mut r, mut g, mut b) = (0u32, 0u32, 0u32);
            for x in 0..w {
                let px = img.get_pixel(x, y).0;
                if px[3] > 200 {
                    opaque += 1;
                    r += px[0] as u32;
                    g += px[1] as u32;
                    b += px[2] as u32;
                }
            }
            if opaque == w {
                fully_opaque_rows += 1;
            }
            if opaque == 0 {
                fully_clear_rows += 1;
            }
            let n = opaque.max(1);
            let mean = if opaque == 0 {
                "        --      ".to_string()
            } else {
                format!("({:3},{:3},{:3})", r / n, g / n, b / n)
            };
            // A grayscale row is a tint mask; a coloured one is finished art.
            let gray = opaque > 0 && (r / n).abs_diff(g / n) < 6 && (g / n).abs_diff(b / n) < 6;
            println!(
                "  y={y:2} opaque={opaque:2}/{w} mean={mean}{}",
                if gray { "  grayscale" } else { "" }
            );
        }
        println!(
            "  -> {fully_opaque_rows}/{h} rows fully opaque, {fully_clear_rows}/{h} fully clear"
        );
    }
}
