const statusEl = document.getElementById("menu-status");
const errorEl = document.getElementById("menu-error");
const loadStatusEl = document.getElementById("load-status");
const loadErrorEl = document.getElementById("load-error");
const loadOverlayEl = document.getElementById("load-overlay");
const fileInput = document.getElementById("world-file");
const urlInput = document.getElementById("world-url");
const loadUrlBtn = document.getElementById("load-url-btn");
const webgpuWarning = document.getElementById("webgpu-warning");

function setStatus(text) {
    statusEl.textContent = text;
    loadStatusEl.textContent = text;
    loadOverlayEl.classList.remove("hidden");
}

function setError(text) {
    errorEl.textContent = text || "";
    loadErrorEl.textContent = text || "";
    if (text) {
        loadOverlayEl.classList.remove("hidden");
    }
}

/** Trunk loads WASM in index.html and exposes bindings on window.wasmBindings. */
function awaitWasmBindings() {
    if (window.wasmBindings?.set_world_from_zip) {
        return Promise.resolve(window.wasmBindings);
    }

    return new Promise((resolve, reject) => {
        const timeout = setTimeout(() => {
            reject(
                new Error(
                    "WASM did not load. Run trunk serve and open http://127.0.0.1:8080 (not the file directly)."
                )
            );
        }, 60_000);

        window.addEventListener(
            "TrunkApplicationStarted",
            () => {
                clearTimeout(timeout);
                if (!window.wasmBindings?.set_world_from_zip) {
                    reject(new Error("WASM loaded but set_world_from_zip export is missing."));
                    return;
                }
                resolve(window.wasmBindings);
            },
            { once: true }
        );
    });
}

/** Real capability check — same path the renderer uses (not `"gpu" in navigator`). */
async function probeWebGpu() {
    if (!window.isSecureContext) {
        return {
            ok: false,
            reason:
                "This page is not a secure context. WebGPU only works on https:// or http://127.0.0.1 / http://localhost — not LAN IPs or custom hostnames over plain HTTP.",
        };
    }

    const gpu = navigator.gpu;
    if (!gpu) {
        return {
            ok: false,
            reason:
                "navigator.gpu is missing. Use Chrome/Edge 113+ with WebGPU enabled (chrome://flags → WebGPU).",
        };
    }

    try {
        const adapter = await gpu.requestAdapter();
        if (!adapter) {
            return {
                ok: false,
                reason:
                    "WebGPU adapter unavailable (blocklisted GPU, disabled flags, or no suitable adapter).",
            };
        }
        return { ok: true };
    } catch (err) {
        return { ok: false, reason: `WebGPU probe failed: ${err}` };
    }
}

function showWebGpuWarning(message) {
    webgpuWarning.textContent = message;
    webgpuWarning.classList.remove("hidden");
}

async function loadBytes(bytes, set_world_from_zip) {
    setError("");
    const mb = (bytes.byteLength / (1024 * 1024)).toFixed(1);
    setStatus(`Parsing world (${mb} MB)…`);
    await new Promise((resolve) => requestAnimationFrame(resolve));

    try {
        set_world_from_zip(bytes);
        setStatus("Starting renderer…");
    } catch (err) {
        setError(String(err));
        setStatus("Ready.");
        throw err;
    }
}

async function loadFile(file, set_world_from_zip) {
    if (!file) return;
    setError("");
    setStatus(`Reading ${file.name}…`);
    const buffer = await file.arrayBuffer();
    await loadBytes(new Uint8Array(buffer), set_world_from_zip);
}

async function loadFromUrl(set_world_from_zip) {
    const url = urlInput.value.trim();
    if (!url) {
        setError("Enter a URL to a .zip file.");
        return;
    }
    setError("");
    setStatus("Downloading world…");
    loadUrlBtn.disabled = true;
    try {
        const response = await fetch(url);
        if (!response.ok) {
            throw new Error(`HTTP ${response.status}`);
        }
        const buffer = await response.arrayBuffer();
        await loadBytes(new Uint8Array(buffer), set_world_from_zip);
    } catch (err) {
        setError(`Download failed: ${err}. URL must allow CORS.`);
        setStatus("Ready.");
    } finally {
        loadUrlBtn.disabled = false;
    }
}

async function main() {
    setStatus("Loading engine…");
    const { set_world_from_zip } = await awaitWasmBindings();

    const modulePreload = document.querySelector('link[rel="modulepreload"]');
    const wasmPreload =
        document.querySelector('link[rel="preload"][as="fetch"]') ||
        document.querySelector('link[rel="preload"][type="application/wasm"]');
    if (modulePreload && wasmPreload) {
        window.__voxelWasmUrls = {
            js: modulePreload.href,
            wasm: wasmPreload.href,
        };
    }

    const webgpu = await probeWebGpu();
    if (!webgpu.ok) {
        showWebGpuWarning(webgpu.reason);
    }

    setStatus("Choose a world zip to begin.");
    fileInput.addEventListener("change", () => {
        loadFile(fileInput.files[0], set_world_from_zip).catch((err) => {
            console.error(err);
            setError(String(err));
            setStatus("Ready.");
        });
        fileInput.value = "";
    });
    loadUrlBtn.addEventListener("click", () => {
        loadFromUrl(set_world_from_zip).catch((err) => {
            console.error(err);
            setError(String(err));
            setStatus("Ready.");
        });
    });
}

main().catch((err) => {
    console.error(err);
    setError(String(err));
    setStatus("Failed to start.");
});
