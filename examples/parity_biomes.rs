//! Exact cross-check of the ported `OverworldBiomeBuilder` against Minecraft's own datagen
//! dump — every one of the 7594 climate boxes must match, in order.
//!
//! This is the strongest validation in the project: unlike the block-level parity harnesses
//! (which are statistical), this is all-or-nothing. If it passes, `mc::biome` reproduces
//! Mojang's biome layout exactly, which is why the table can be *generated* rather than
//! copied into the repo.
//!
//! Get the dump by running Minecraft's data generator (see HANDOFF.md "Re-run datagen"):
//! it lands at `<out>/reports/biome_parameters/minecraft/overworld.json`. That file is
//! Mojang's data — keep it local, never commit it.
//!
//! Run: `cargo run --release --example parity_biomes -- "<path to overworld.json>"`

use std::fs;

use serde_json::Value;
use voxel_engine::mc::biome::overworld_biomes;
use voxel_engine::mc::climate::{quantize, Parameter, ParameterPoint};

fn main() {
    let path = std::env::args().nth(1).expect("usage: parity_biomes <path to overworld.json>");
    let raw = fs::read_to_string(&path).expect("read the datagen dump");
    let json: Value = serde_json::from_str(&raw).expect("parse the datagen dump");
    let expected: Vec<(ParameterPoint, String)> = json["biomes"]
        .as_array()
        .expect("`biomes` array")
        .iter()
        .map(parse_entry)
        .collect();

    let actual = overworld_biomes();

    println!("mojang entries: {}", expected.len());
    println!("ported entries: {}", actual.len());
    if expected.len() != actual.len() {
        println!("\nFAIL: entry count differs");
        std::process::exit(1);
    }

    let mut mismatches = 0usize;
    for (i, ((want_point, want_biome), (got_point, got_biome))) in
        expected.iter().zip(actual.iter()).enumerate()
    {
        let got_name = format!("minecraft:{got_biome}");
        if want_point != got_point || *want_biome != got_name {
            mismatches += 1;
            if mismatches <= 10 {
                println!("\nmismatch at index {i}:");
                println!("  mojang: {want_biome} {want_point:?}");
                println!("  ported: {got_name} {got_point:?}");
            }
        }
    }

    if mismatches == 0 {
        println!("\nPASS: all {} entries match exactly (box + biome + order)", actual.len());
    } else {
        println!("\nFAIL: {mismatches} of {} entries differ", actual.len());
        std::process::exit(1);
    }
}

fn parse_entry(entry: &Value) -> (ParameterPoint, String) {
    let p = &entry["parameters"];
    (
        ParameterPoint {
            temperature: parse_param(&p["temperature"]),
            humidity: parse_param(&p["humidity"]),
            continentalness: parse_param(&p["continentalness"]),
            erosion: parse_param(&p["erosion"]),
            depth: parse_param(&p["depth"]),
            weirdness: parse_param(&p["weirdness"]),
            offset: quantize(as_f32(&p["offset"])),
        },
        entry["biome"].as_str().expect("biome name").to_string(),
    )
}

/// A parameter is serialised either as `[min, max]` or, when zero-width, as a bare scalar.
fn parse_param(v: &Value) -> Parameter {
    match v.as_array() {
        Some(pair) => Parameter::span(as_f32(&pair[0]), as_f32(&pair[1])),
        None => Parameter::point(as_f32(v)),
    }
}

fn as_f32(v: &Value) -> f32 {
    v.as_f64().expect("numeric parameter") as f32
}
