//! Top-down preview of the MC-parity terrain: shade the `mc::overworld` surface height
//! over a square region and write a PNG. This is a quick way to *see* the generator's
//! output for a seed without wiring it into the live engine.
//!
//! Pre-cave / pre-surface-rules, so colouring is a height/sea-level approximation (ocean,
//! beach, grass, rock, snow) plus hillshading — not the game's exact block palette.
//!
//! Run: `cargo run --release --example preview -- [seed] [size] [blocks_per_pixel]`
//!   e.g. `cargo run --release --example preview -- 6954908675375307936 1024 2`

use image::{Rgb, RgbImage};
use rayon::prelude::*;
use voxel_engine::mc::overworld::Overworld;

const SEA_LEVEL: i32 = 63;

fn main() {
    let mut args = std::env::args().skip(1);
    let seed: i64 = args.next().map(|s| s.parse().unwrap()).unwrap_or(6954908675375307936);
    let size: u32 = args.next().map(|s| s.parse().unwrap()).unwrap_or(1024);
    let bpp: i32 = args.next().map(|s| s.parse().unwrap()).unwrap_or(2); // blocks per pixel

    let ow = Overworld::new(seed);
    // Center the region on the world origin (spawn).
    let half = size as i32 * bpp / 2;
    let x0 = -half;
    let z0 = -half;

    eprintln!("generating {size}x{size} px ({} blocks) for seed {seed}…", size as i32 * bpp);
    let t = std::time::Instant::now();

    // Surface height per pixel, rows computed in parallel.
    let heights: Vec<Vec<i32>> = (0..size)
        .into_par_iter()
        .map(|py| {
            (0..size)
                .map(|px| {
                    let wx = x0 + px as i32 * bpp;
                    let wz = z0 + py as i32 * bpp;
                    surface_height(&ow, wx, wz)
                })
                .collect()
        })
        .collect();

    eprintln!("  heights done in {:.1}s, colouring…", t.elapsed().as_secs_f64());

    let mut img = RgbImage::new(size, size);
    for py in 0..size {
        for px in 0..size {
            let h = heights[py as usize][px as usize];
            // Hillshade from the west/north neighbours (gradient toward a NW light).
            let hl = if px > 0 { heights[py as usize][px as usize - 1] } else { h };
            let hu = if py > 0 { heights[py as usize - 1][px as usize] } else { h };
            let slope = ((h - hl) + (h - hu)) as f32 / bpp as f32;
            let shade = (1.0 + 0.14 * slope).clamp(0.55, 1.45);
            img.put_pixel(px, py, shade_color(h, shade));
        }
    }

    let out = format!("terrain_preview_{seed}.png");
    img.save(&out).expect("save png");
    eprintln!("wrote {out} in {:.1}s total", t.elapsed().as_secs_f64());
    println!("{out}");
}

/// Highest solid Y (pre-cave). Coarse step of 4 from a sane ceiling, then refine by 1 —
/// `density_no_caves` is near-monotonic in Y above the caves, so this is safe for a heightmap
/// and ~4× cheaper than a full 1-by-1 scan.
fn surface_height(ow: &Overworld, x: i32, z: i32) -> i32 {
    let (xf, zf) = (x as f64, z as f64);
    let mut y = 256;
    while y > -64 && ow.density_no_caves(xf, y as f64, zf) <= 0.0 {
        y -= 4;
    }
    // Refine upward within the last coarse step.
    let mut best = y;
    for yy in (y..=(y + 4).min(256)).rev() {
        if ow.density_no_caves(xf, yy as f64, zf) > 0.0 {
            best = yy;
            break;
        }
    }
    best.max(-64)
}

/// Map a surface height to an approximate overworld colour, then apply hillshade.
fn shade_color(h: i32, shade: f32) -> Rgb<u8> {
    let base: [f32; 3] = if h < SEA_LEVEL - 12 {
        [30.0, 60.0, 130.0] // deep ocean
    } else if h < SEA_LEVEL {
        // shallow water: lighten toward the shore
        let t = (h - (SEA_LEVEL - 12)) as f32 / 12.0;
        lerp3([30.0, 60.0, 130.0], [70.0, 120.0, 180.0], t)
    } else if h <= SEA_LEVEL + 1 {
        [214.0, 200.0, 150.0] // beach sand
    } else if h < 85 {
        // grass: darken with altitude
        let t = ((h - 64) as f32 / 21.0).clamp(0.0, 1.0);
        lerp3([96.0, 150.0, 72.0], [72.0, 120.0, 58.0], t)
    } else if h < 100 {
        // rock
        let t = ((h - 85) as f32 / 15.0).clamp(0.0, 1.0);
        lerp3([120.0, 116.0, 108.0], [150.0, 146.0, 140.0], t)
    } else {
        [235.0, 238.0, 242.0] // snow
    };
    Rgb([
        (base[0] * shade).clamp(0.0, 255.0) as u8,
        (base[1] * shade).clamp(0.0, 255.0) as u8,
        (base[2] * shade).clamp(0.0, 255.0) as u8,
    ])
}

fn lerp3(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [a[0] + t * (b[0] - a[0]), a[1] + t * (b[1] - a[1]), a[2] + t * (b[2] - a[2])]
}
