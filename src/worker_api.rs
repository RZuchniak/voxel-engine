use wasm_bindgen::prelude::*;

use crate::{
    chunk_worker,
    worker_protocol::{decode_job, encode_reply},
};

#[wasm_bindgen]
pub fn worker_init(zip_bytes: &[u8]) -> Result<(), String> {
    console_error_panic_hook::set_once();
    chunk_worker::init_from_zip(zip_bytes)
}

#[wasm_bindgen]
pub fn worker_init_seed(seed: i64) -> Result<(), String> {
    console_error_panic_hook::set_once();
    chunk_worker::init_from_seed(seed)
}

#[wasm_bindgen]
pub fn worker_handle_job(payload: &[u8]) -> Result<Vec<u8>, String> {
    let job = decode_job(payload)?;
    let reply = chunk_worker::handle_job(job);
    Ok(encode_reply(&reply))
}
