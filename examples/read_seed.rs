//! One-off helper: read a Minecraft world seed out of `level.dat`.
//!
//! `level.dat` is gzip-compressed NBT. In 1.18+ the seed lives at
//! `Data > WorldGenSettings > seed` (a long). Older versions kept it at
//! `Data > RandomSeed`. We print whatever we find so the parity oracle can
//! be pinned to the real world.
//!
//! Run: `cargo run --example read_seed -- "<path to level.dat>"`

use std::io::Read;

use flate2::read::GzDecoder;
use fastnbt::Value;

fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("usage: read_seed <path to level.dat>");
    let bytes = std::fs::read(&path).expect("read level.dat");

    // level.dat is gzip-compressed; fall back to raw if it isn't.
    let mut decoded = Vec::new();
    if GzDecoder::new(&bytes[..]).read_to_end(&mut decoded).is_err() {
        decoded = bytes;
    }

    let root: Value = fastnbt::from_bytes(&decoded).expect("parse NBT");

    if std::env::args().any(|a| a == "--dump") {
        dump(&root, "", 0);
        return;
    }

    let data = get(&root, "Data").expect("Data compound");

    if let Some(wgs) = get(data, "WorldGenSettings") {
        if let Some(Value::Long(seed)) = get(wgs, "seed") {
            println!("seed (WorldGenSettings): {seed}");
        }
    }
    if let Some(Value::Long(seed)) = get(data, "RandomSeed") {
        println!("seed (RandomSeed): {seed}");
    }

    // Also surface the version so we know we're reading the right world.
    if let Some(Value::Compound(v)) = get(data, "Version") {
        if let Some(Value::String(name)) = v.get("Name") {
            println!("version name: {name}");
        }
        if let Some(Value::Int(id)) = v.get("Id") {
            println!("version id: {id}");
        }
    }
    if let Some(Value::String(name)) = get(data, "LevelName") {
        println!("level name: {name}");
    }
}

fn get<'a>(v: &'a Value, key: &str) -> Option<&'a Value> {
    match v {
        Value::Compound(m) => m.get(key),
        _ => None,
    }
}

fn dump(v: &Value, name: &str, depth: usize) {
    let pad = "  ".repeat(depth);
    match v {
        Value::Compound(m) => {
            println!("{pad}{name}: Compound");
            if depth < 7 {
                for (k, val) in m {
                    dump(val, k, depth + 1);
                }
            }
        }
        Value::Long(n) => println!("{pad}{name}: Long = {n}"),
        Value::Int(n) => println!("{pad}{name}: Int = {n}"),
        Value::String(s) => println!("{pad}{name}: String = {s:?}"),
        other => println!("{pad}{name}: {}", type_name(other)),
    }
}

fn type_name(v: &Value) -> &'static str {
    match v {
        Value::Byte(_) => "Byte",
        Value::Short(_) => "Short",
        Value::Int(_) => "Int",
        Value::Long(_) => "Long",
        Value::Float(_) => "Float",
        Value::Double(_) => "Double",
        Value::String(_) => "String",
        Value::ByteArray(_) => "ByteArray",
        Value::IntArray(_) => "IntArray",
        Value::LongArray(_) => "LongArray",
        Value::List(_) => "List",
        Value::Compound(_) => "Compound",
    }
}
