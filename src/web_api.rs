#[cfg(target_arch = "wasm32")]
use std::cell::Cell;
#[cfg(target_arch = "wasm32")]
use std::sync::{Arc, Mutex, OnceLock};

#[cfg(target_arch = "wasm32")]
use wasm_bindgen::{JsCast, prelude::*};

#[cfg(target_arch = "wasm32")]
use crate::source::{MemoryAnvilSource, SeededProceduralSource, WorldSource};

#[cfg(target_arch = "wasm32")]
static INIT_ZIP: OnceLock<Mutex<Option<Vec<u8>>>> = OnceLock::new();

#[cfg(target_arch = "wasm32")]
fn init_zip_slot() -> &'static Mutex<Option<Vec<u8>>> {
    INIT_ZIP.get_or_init(|| Mutex::new(None))
}

#[cfg(target_arch = "wasm32")]
pub fn store_init_zip(bytes: &[u8]) {
    if let Ok(mut slot) = init_zip_slot().lock() {
        *slot = Some(bytes.to_vec());
    }
}

#[cfg(target_arch = "wasm32")]
pub fn take_init_zip() -> Option<Vec<u8>> {
    init_zip_slot().lock().ok()?.take()
}

#[cfg(target_arch = "wasm32")]
pub fn clone_init_zip() -> Option<Vec<u8>> {
    init_zip_slot().lock().ok()?.as_ref().cloned()
}

#[cfg(target_arch = "wasm32")]
static PENDING_WORLD: OnceLock<Mutex<Option<Arc<dyn WorldSource>>>> = OnceLock::new();

#[cfg(target_arch = "wasm32")]
fn pending_world() -> &'static Mutex<Option<Arc<dyn WorldSource>>> {
    PENDING_WORLD.get_or_init(|| Mutex::new(None))
}

#[cfg(target_arch = "wasm32")]
pub fn take_pending_world() -> Option<Arc<dyn WorldSource>> {
    pending_world().lock().ok()?.take()
}

#[cfg(target_arch = "wasm32")]
pub fn set_pending_world(source: Arc<dyn WorldSource>) {
    if let Ok(mut slot) = pending_world().lock() {
        *slot = Some(source);
    }
}

#[cfg(target_arch = "wasm32")]
fn document() -> Option<web_sys::Document> {
    web_sys::window()?.document()
}

#[cfg(target_arch = "wasm32")]
fn set_element_text(id: &str, message: &str) {
    if let Some(document) = document() {
        if let Some(el) = document.get_element_by_id(id) {
            el.set_text_content(Some(message));
        }
    }
}

#[cfg(target_arch = "wasm32")]
pub fn set_load_status(message: &str) {
    set_element_text("menu-status", message);
    set_element_text("load-status", message);
    show_load_overlay();
}

#[cfg(target_arch = "wasm32")]
pub fn set_load_error(message: &str) {
    set_element_text("menu-error", message);
    set_element_text("load-error", message);
    show_load_overlay();
}

#[cfg(target_arch = "wasm32")]
fn clear_load_error() {
    set_element_text("menu-error", "");
    set_element_text("load-error", "");
}

#[cfg(target_arch = "wasm32")]
fn show_load_overlay() {
    if let Some(document) = document() {
        if let Some(overlay) = document.get_element_by_id("load-overlay") {
            let _ = overlay.class_list().remove_1("hidden");
        }
    }
}

#[cfg(target_arch = "wasm32")]
pub fn hide_load_overlay() {
    if let Some(document) = document() {
        if let Some(overlay) = document.get_element_by_id("load-overlay") {
            let _ = overlay.class_list().add_1("hidden");
        }
    }
}

#[cfg(target_arch = "wasm32")]
pub fn canvas_pixel_size() -> Option<(u32, u32)> {
    let document = document()?;
    let canvas = document.get_element_by_id("voxel-canvas")?;
    let canvas = canvas.dyn_into::<web_sys::HtmlCanvasElement>().ok()?;
    let w = canvas.width();
    let h = canvas.height();
    if w > 0 && h > 0 {
        Some((w, h))
    } else {
        None
    }
}

#[cfg(target_arch = "wasm32")]
pub fn prepare_canvas_for_game() {
    if let Some(window) = web_sys::window() {
        if let Some(document) = window.document() {
            if let Some(canvas) = document.get_element_by_id("voxel-canvas") {
                let _ = canvas.class_list().remove_1("hidden");
                if let Ok(canvas) = canvas.dyn_into::<web_sys::HtmlCanvasElement>() {
                    let w = window
                        .inner_width()
                        .ok()
                        .and_then(|value| value.as_f64())
                        .unwrap_or(800.0)
                        .max(1.0) as u32;
                    let h = window
                        .inner_height()
                        .ok()
                        .and_then(|value| value.as_f64())
                        .unwrap_or(600.0)
                        .max(1.0) as u32;
                    canvas.set_width(w);
                    canvas.set_height(h);
                }
            }
        }
    }
}

#[cfg(target_arch = "wasm32")]
fn show_game_ui() {
    prepare_canvas_for_game();
    if let Some(document) = document() {
        if let Some(menu) = document.get_element_by_id("startup-menu") {
            let _ = menu.class_list().add_1("hidden");
        }
        if let Some(overlay) = document.get_element_by_id("click-to-play") {
            let _ = overlay.class_list().remove_1("hidden");
        }
    }
}

#[cfg(target_arch = "wasm32")]
pub fn show_startup_menu(message: &str) {
    if let Some(document) = document() {
        if let Some(menu) = document.get_element_by_id("startup-menu") {
            let _ = menu.class_list().remove_1("hidden");
        }
        if let Some(overlay) = document.get_element_by_id("click-to-play") {
            let _ = overlay.class_list().add_1("hidden");
        }
        if let Some(canvas) = document.get_element_by_id("voxel-canvas") {
            let _ = canvas.class_list().add_1("hidden");
        }
    }
    set_load_status(message);
}

#[cfg(target_arch = "wasm32")]
pub fn kick_event_loop() {
    if let Some(window) = web_sys::window() {
        let closure = wasm_bindgen::closure::Closure::once(|| {});
        let _ = window.request_animation_frame(closure.as_ref().unchecked_ref());
        closure.forget();
    }
}

#[cfg(target_arch = "wasm32")]
thread_local! {
    static RESET_RENDERER_LOADING: Cell<bool> = Cell::new(false);
    static PROCEDURAL_WORLD: Cell<bool> = Cell::new(false);
}

#[cfg(target_arch = "wasm32")]
pub fn signal_renderer_failed(message: &str) {
    RESET_RENDERER_LOADING.set(true);
    set_load_error(message);
    show_startup_menu("Ready.");
}

#[cfg(target_arch = "wasm32")]
pub fn take_reset_renderer_loading() -> bool {
    RESET_RENDERER_LOADING.replace(false)
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn init_web_logging() {
    console_error_panic_hook::set_once();
}

#[cfg(target_arch = "wasm32")]
pub fn is_procedural_world() -> bool {
    PROCEDURAL_WORLD.with(|flag| flag.get())
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn set_world_from_zip(bytes: &[u8]) -> Result<(), String> {
    clear_load_error();
    PROCEDURAL_WORLD.with(|flag| flag.set(false));
    set_load_status("Reading world zip…");
    store_init_zip(bytes);
    let source = MemoryAnvilSource::from_zip(bytes).map_err(|e| e.to_string())?;
    set_pending_world(Arc::new(source));
    set_load_status("World loaded — starting engine…");
    show_game_ui();
    kick_event_loop();
    Ok(())
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn set_world_from_seed(seed: i64) -> Result<(), String> {
    clear_load_error();
    PROCEDURAL_WORLD.with(|flag| flag.set(true));
    if let Ok(mut slot) = init_zip_slot().lock() {
        *slot = None;
    }
    set_load_status(&format!("Generating world (seed {seed})…"));
    set_pending_world(Arc::new(SeededProceduralSource::new(seed)));
    set_load_status("World ready — starting engine…");
    show_game_ui();
    kick_event_loop();
    Ok(())
}

#[cfg(target_arch = "wasm32")]
pub fn hide_click_to_play() {
    if let Some(document) = document() {
        if let Some(overlay) = document.get_element_by_id("click-to-play") {
            let _ = overlay.class_list().add_1("hidden");
        }
    }
    hide_load_overlay();
}

/// Selectable streaming stats. `None` hides the panel.
#[cfg(target_arch = "wasm32")]
pub fn set_perf_overlay(text: Option<&str>) {
    let Some(document) = document() else {
        return;
    };
    let Some(el) = document.get_element_by_id("perf-overlay") else {
        return;
    };
    match text {
        Some(text) => {
            el.set_text_content(Some(text));
            let _ = el.class_list().remove_1("hidden");
        }
        None => {
            let _ = el.class_list().add_1("hidden");
        }
    }
}
