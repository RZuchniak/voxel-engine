/**
 * Chunk load + mesh worker. Runs the same WASM module as the main thread but
 * only executes worker_init / worker_handle_job (no winit / WebGPU).
 *
 * WASM instances are not re-entrant — jobs must run strictly one at a time.
 */
let initPromise = null;
let wasmModule = null;
const jobQueue = [];
let drainingJobs = false;

async function ensureWasm(wasmJs, wasmModuleUrl) {
    if (wasmModule) {
        return wasmModule;
    }
    if (!initPromise) {
        initPromise = (async () => {
            const wasm = await import(wasmJs);
            await wasm.default({ module_or_path: wasmModuleUrl });
            wasmModule = wasm;
            return wasm;
        })();
    }
    return initPromise;
}

async function runJob(msg) {
    if (!wasmModule) {
        throw new Error("worker wasm not initialized");
    }
    const payload = new Uint8Array(msg.payload);
    const result = wasmModule.worker_handle_job(payload);
    const out = result instanceof Uint8Array ? result : new Uint8Array(result);
    self.postMessage({ type: "result", jobId: msg.jobId, payload: out.buffer }, [out.buffer]);
}

async function drainJobQueue() {
    if (drainingJobs) {
        return;
    }
    drainingJobs = true;
    while (jobQueue.length > 0) {
        const msg = jobQueue.shift();
        try {
            await runJob(msg);
        } catch (err) {
            self.postMessage({
                type: "error",
                jobId: msg.jobId,
                message: String(err),
            });
        }
    }
    drainingJobs = false;
}

self.onmessage = async (event) => {
    const msg = event.data;
    if (!msg || !msg.type) {
        return;
    }

    if (msg.type === "init") {
        try {
            const wasm = await ensureWasm(msg.wasmJs, msg.wasmModule);
            const zipBytes =
                msg.zipBytes instanceof Uint8Array
                    ? msg.zipBytes
                    : new Uint8Array(msg.zipBytes);
            wasm.worker_init(zipBytes);
            self.postMessage({ type: "ready", workerId: msg.workerId });
        } catch (err) {
            self.postMessage({
                type: "error",
                workerId: msg.workerId,
                message: String(err),
            });
        }
        return;
    }

    if (msg.type === "job") {
        jobQueue.push(msg);
        await drainJobQueue();
    }
};
