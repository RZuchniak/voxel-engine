/**
 * Browser frame-time probe.
 *
 * Measures whole-frame time via requestAnimationFrame. Deliberately does NOT try to
 * measure per-phase timings: browsers clamp `performance.now()` to ~100µs (some to 1ms)
 * unless the page is cross-origin isolated, so sub-millisecond phase splits would
 * quantise to noise. Whole-frame deltas averaged over many frames are trustworthy.
 *
 * Usage — from the devtools console on http://127.0.0.1:8080:
 *
 *     await import("/web/perf-probe.js");
 *     await voxelProbe();                                  // seed 12345, defaults
 *     await voxelProbe({ seed: "42", sampleMs: 30000 });
 *
 * Results are printed and also left on `window.__voxelProbeResult` for copy-out.
 */

const OVERLAY_ID = "load-overlay";
const SEED_INPUT_ID = "world-seed";
const GENERATE_BTN_ID = "generate-seed-btn";

/** Long-frame threshold — a frame this slow reads as a visible hitch. */
const HITCH_MS = 50;

function percentile(sorted, p) {
    if (sorted.length === 0) return NaN;
    const idx = Math.min(sorted.length - 1, Math.max(0, Math.ceil((p / 100) * sorted.length) - 1));
    return sorted[idx];
}

function summarise(deltas) {
    if (deltas.length === 0) {
        return { frames: 0 };
    }
    const sorted = [...deltas].sort((a, b) => a - b);
    const mean = deltas.reduce((a, b) => a + b, 0) / deltas.length;
    return {
        frames: deltas.length,
        mean_ms: +mean.toFixed(2),
        fps_mean: +(1000 / mean).toFixed(1),
        p50_ms: +percentile(sorted, 50).toFixed(2),
        p95_ms: +percentile(sorted, 95).toFixed(2),
        p99_ms: +percentile(sorted, 99).toFixed(2),
        max_ms: +sorted[sorted.length - 1].toFixed(2),
        fps_p50: +(1000 / percentile(sorted, 50)).toFixed(1),
        hitches: deltas.filter((d) => d >= HITCH_MS).length,
    };
}

function overlayHidden() {
    const el = document.getElementById(OVERLAY_ID);
    return !el || el.classList.contains("hidden");
}

function waitFor(predicate, timeoutMs, label) {
    return new Promise((resolve, reject) => {
        const started = performance.now();
        const tick = () => {
            if (predicate()) {
                resolve(performance.now() - started);
                return;
            }
            if (performance.now() - started > timeoutMs) {
                reject(new Error(`timed out waiting for ${label} after ${timeoutMs}ms`));
                return;
            }
            requestAnimationFrame(tick);
        };
        tick();
    });
}

/** Collect rAF deltas for `durationMs`, bucketed into `bucketMs` windows. */
function collect(durationMs, bucketMs) {
    return new Promise((resolve) => {
        const deltas = [];
        const buckets = [];
        let current = [];
        let last = performance.now();
        const started = last;
        let bucketStart = last;

        const tick = (now) => {
            const dt = now - last;
            last = now;
            deltas.push(dt);
            current.push(dt);

            if (now - bucketStart >= bucketMs) {
                buckets.push({ at_s: +((now - started) / 1000).toFixed(1), ...summarise(current) });
                current = [];
                bucketStart = now;
            }
            if (now - started >= durationMs) {
                if (current.length) {
                    buckets.push({ at_s: +((now - started) / 1000).toFixed(1), ...summarise(current) });
                }
                resolve({ deltas, buckets });
                return;
            }
            requestAnimationFrame(tick);
        };
        requestAnimationFrame(tick);
    });
}

window.voxelProbe = async function voxelProbe(opts = {}) {
    const {
        seed = "12345",
        warmupMs = 5000,
        sampleMs = 20000,
        bucketMs = 2000,
        startTimeoutMs = 120000,
    } = opts;

    const result = { seed, startedAt: new Date().toISOString() };

    // 1. Kick off world generation through the normal menu path, unless already running.
    if (!overlayHidden()) {
        const input = document.getElementById(SEED_INPUT_ID);
        const btn = document.getElementById(GENERATE_BTN_ID);
        if (!input || !btn) throw new Error("startup menu not found — is the page loaded?");
        input.value = seed;
        const clickedAt = performance.now();
        btn.click();
        console.log(`[probe] generating seed ${seed}…`);
        await waitFor(overlayHidden, startTimeoutMs, "renderer start");
        result.time_to_first_frame_ms = +(performance.now() - clickedAt).toFixed(0);
        console.log(`[probe] renderer started in ${result.time_to_first_frame_ms}ms`);
    } else {
        console.log("[probe] renderer already running — skipping world generation");
    }

    // 2. Warmup. Bootstrap streaming is still running here; these frames are not
    //    representative of steady state, but the curve is worth seeing.
    console.log(`[probe] warmup ${warmupMs}ms…`);
    const warmup = await collect(warmupMs, bucketMs);
    result.warmup = summarise(warmup.deltas);

    // 3. Steady-state sample.
    console.log(`[probe] sampling ${sampleMs}ms…`);
    const sample = await collect(sampleMs, bucketMs);
    result.steady = summarise(sample.deltas);
    result.buckets = sample.buckets;

    result.userAgent = navigator.userAgent;
    result.devicePixelRatio = window.devicePixelRatio;
    const canvas = document.getElementById("voxel-canvas");
    if (canvas) result.canvas = { w: canvas.width, h: canvas.height };

    window.__voxelProbeResult = result;
    console.log("[probe] warmup :", result.warmup);
    console.log("[probe] steady :", result.steady);
    console.table(result.buckets);
    console.log("[probe] full result on window.__voxelProbeResult");
    console.log(JSON.stringify(result, null, 2));
    return result;
};

console.log("[probe] loaded — call `await voxelProbe()`");
