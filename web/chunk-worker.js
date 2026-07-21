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

/**
 * Startup progress, reported through the ack channel so the bridge logs it.
 *
 * Fires ~5 times per worker during init and never again, so it costs nothing at steady
 * state. Worth keeping: worker console output is invisible to page-level tooling, and
 * without these stages a worker that is merely slow is indistinguishable from one that
 * is wedged.
 */
function trace(workerId, stage) {
    self.postMessage({ type: "ack", workerId, forType: stage });
}

async function ensureWasm(wasmJs, wasmModuleUrl, workerId) {
    if (wasmModule) {
        return wasmModule;
    }
    if (!initPromise) {
        initPromise = (async () => {
            trace(workerId, "import:start");
            const wasm = await import(wasmJs);
            trace(workerId, "import:done");
            trace(workerId, "instantiate:start");
            await wasm.default({ module_or_path: wasmModuleUrl });
            trace(workerId, "instantiate:done");
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

    // Acknowledge before touching wasm. Loading the module takes seconds, and without
    // this there is no way to tell "the message never arrived" apart from "the worker is
    // still starting up" — a distinction that previously cost a long debugging session.
    self.postMessage({ type: "ack", workerId: msg.workerId, forType: msg.type });

    if (msg.type === "init") {
        try {
            const wasm = await ensureWasm(msg.wasmJs, msg.wasmModule, msg.workerId);
            trace(msg.workerId, "wasm:ready");
            if (msg.seed !== undefined && msg.seed !== null) {
                // Seed worlds ship a seed, not chunk data — generation is deterministic,
                // so each worker reproduces the same terrain independently.
                wasm.worker_init_seed(BigInt(msg.seed));
            } else {
                const zipBytes =
                    msg.zipBytes instanceof Uint8Array
                        ? msg.zipBytes
                        : new Uint8Array(msg.zipBytes);
                wasm.worker_init(zipBytes);
            }
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
