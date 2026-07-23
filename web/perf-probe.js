/**
 * Browser frame-time probe.
 *
 * Measures whole-frame time via requestAnimationFrame. Deliberately does NOT try to
 * measure per-phase timings: browsers clamp `performance.now()` to ~100µs (some to 1ms)
 * unless the page is cross-origin isolated, so sub-millisecond phase splits would
 * quantise to noise. Whole-frame deltas averaged over many frames are trustworthy.
 *
 * The point of this probe is to make stutter a NUMBER. Averages and even p99 hide the
 * ~1s hitches that worker-side meshing introduced, because a handful of catastrophic
 * frames barely move a percentile computed over thousands of frames. So beyond the
 * summary stats we capture every long frame with its timestamp, bucket them by severity,
 * and report the worst offenders and cumulative time lost. Read `steady_hitches` first.
 *
 * Usage — from the devtools console on http://127.0.0.1:8080:
 *
 *     await import("/web/perf-probe.js");
 *     await voxelProbe();                                  // seed 12345, defaults
 *     await voxelProbe({ seed: "42", sampleMs: 30000 });
 *     await voxelProbe({ hitchMs: 33 });                   // stricter hitch threshold
 *
 * Results are printed and also left on `window.__voxelProbeResult` for copy-out.
 */

const OVERLAY_ID = "load-overlay";
const SEED_INPUT_ID = "world-seed";
const GENERATE_BTN_ID = "generate-seed-btn";

/** Default long-frame threshold — a frame this slow reads as a visible hitch. */
const HITCH_MS = 50;

function percentile(sorted, p) {
    if (sorted.length === 0) return NaN;
    const idx = Math.min(sorted.length - 1, Math.max(0, Math.ceil((p / 100) * sorted.length) - 1));
    return sorted[idx];
}

function summarise(deltas, hitchMs = HITCH_MS) {
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
        hitches: deltas.filter((d) => d >= hitchMs).length,
    };
}

/**
 * Characterise the long frames. This is the part that turns "it stutters" into evidence:
 * how many, how bad, when, and how much wall-clock time was lost to jank overall.
 */
function summariseHitches(hitches, sampleMs, hitchMs) {
    const bands = { "50-100": 0, "100-250": 0, "250-500": 0, "500-1000": 0, "1000+": 0 };
    let totalMs = 0;
    for (const h of hitches) {
        totalMs += h.ms;
        if (h.ms < 100) bands["50-100"]++;
        else if (h.ms < 250) bands["100-250"]++;
        else if (h.ms < 500) bands["250-500"]++;
        else if (h.ms < 1000) bands["500-1000"]++;
        else bands["1000+"]++;
    }
    const worst = [...hitches]
        .sort((a, b) => b.ms - a.ms)
        .slice(0, 5)
        .map((h) => ({ ms: +h.ms.toFixed(0), at_s: h.at_s }));
    return {
        threshold_ms: hitchMs,
        count: hitches.length,
        per_min: sampleMs ? +(hitches.length / (sampleMs / 60000)).toFixed(1) : 0,
        total_ms: +totalMs.toFixed(0),
        pct_of_sample: sampleMs ? +((totalMs / sampleMs) * 100).toFixed(1) : 0,
        bands,
        worst,
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

/**
 * Collect rAF deltas for `durationMs`, bucketed into `bucketMs` windows. Every frame at
 * or above `hitchMs` is also recorded individually with its timestamp so long frames can
 * be located in time (e.g. correlated with chunk arrivals) rather than just counted.
 */
function collect(durationMs, bucketMs, hitchMs) {
    return new Promise((resolve) => {
        const deltas = [];
        const buckets = [];
        const hitches = [];
        let current = [];
        let last = performance.now();
        const started = last;
        let bucketStart = last;

        const tick = (now) => {
            const dt = now - last;
            last = now;
            deltas.push(dt);
            current.push(dt);
            if (dt >= hitchMs) {
                hitches.push({ at_s: +((now - started) / 1000).toFixed(2), ms: dt });
            }

            if (now - bucketStart >= bucketMs) {
                buckets.push({ at_s: +((now - started) / 1000).toFixed(1), ...summarise(current, hitchMs) });
                current = [];
                bucketStart = now;
            }
            if (now - started >= durationMs) {
                if (current.length) {
                    buckets.push({ at_s: +((now - started) / 1000).toFixed(1), ...summarise(current, hitchMs) });
                }
                resolve({ deltas, buckets, hitches });
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
        hitchMs = HITCH_MS,
        startTimeoutMs = 120000,
    } = opts;

    const result = { seed, hitchMs, startedAt: new Date().toISOString() };

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
    const warmup = await collect(warmupMs, bucketMs, hitchMs);
    result.warmup = summarise(warmup.deltas, hitchMs);
    result.warmup_hitches = summariseHitches(warmup.hitches, warmupMs, hitchMs);

    // 3. Steady-state sample.
    console.log(`[probe] sampling ${sampleMs}ms…`);
    const sample = await collect(sampleMs, bucketMs, hitchMs);
    result.steady = summarise(sample.deltas, hitchMs);
    result.steady_hitches = summariseHitches(sample.hitches, sampleMs, hitchMs);
    result.buckets = sample.buckets;

    result.userAgent = navigator.userAgent;
    result.devicePixelRatio = window.devicePixelRatio;
    const canvas = document.getElementById("voxel-canvas");
    if (canvas) result.canvas = { w: canvas.width, h: canvas.height };

    window.__voxelProbeResult = result;
    console.log("[probe] warmup :", result.warmup);
    console.log("[probe] steady :", result.steady);
    console.log(`[probe] steady hitches (>=${hitchMs}ms):`, result.steady_hitches);
    if (result.steady_hitches.count) {
        console.table(result.steady_hitches.worst);
    }
    console.table(result.buckets);
    console.log("[probe] full result on window.__voxelProbeResult");
    console.log(JSON.stringify(result, null, 2));
    return result;
};

console.log("[probe] loaded — call `await voxelProbe()`");
