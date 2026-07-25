use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet, VecDeque};
use std::rc::Rc;

use wasm_bindgen::{JsCast, prelude::*};
use web_sys::{MessageEvent, Worker, WorkerOptions, WorkerType};

use crate::{
    streamer::MeshedChunk,
    worker_protocol::{
        decode_reply, encode_job, SectionMeshWire, WorkerJob, WorkerReply,
    },
};

const WORKER_SCRIPT: &str = "/web/chunk-worker.js";

thread_local! {
    static WORKER_INBOX: RefCell<Vec<WorkerReply>> = RefCell::new(Vec::new());
    static WORKER_FAILURES: RefCell<Vec<WorkerFailure>> = RefCell::new(Vec::new());
    static WORKER_EVENTS: RefCell<Vec<InboundWorkerEvent>> = RefCell::new(Vec::new());
}

#[derive(Clone, Copy, Debug)]
pub struct WorkerFailure {
    pub coord: (i32, i32),
    pub is_remesh: bool,
}

enum InboundWorkerEvent {
    Ready { worker_id: usize },
    Completed {
        expected_job_id: u32,
        reply: WorkerReply,
    },
    Failed { job_id: u32, message: String },
}

fn push_reply(reply: WorkerReply) {
    WORKER_INBOX.with(|inbox| inbox.borrow_mut().push(reply));
}

fn take_replies() -> Vec<WorkerReply> {
    WORKER_INBOX.with(|inbox| std::mem::take(&mut *inbox.borrow_mut()))
}

fn push_failure(failure: WorkerFailure) {
    WORKER_FAILURES.with(|failures| failures.borrow_mut().push(failure));
}

pub fn take_failures() -> Vec<WorkerFailure> {
    WORKER_FAILURES.with(|failures| std::mem::take(&mut *failures.borrow_mut()))
}

fn push_event(event: InboundWorkerEvent) {
    WORKER_EVENTS.with(|events| events.borrow_mut().push(event));
}

fn take_events() -> Vec<InboundWorkerEvent> {
    WORKER_EVENTS.with(|events| std::mem::take(&mut *events.borrow_mut()))
}

fn wasm_urls() -> Result<(String, String), String> {
    let window = web_sys::window().ok_or("missing window")?;
    if let Ok(urls) = js_sys::Reflect::get(&window, &JsValue::from_str("__voxelWasmUrls")) {
        if !urls.is_undefined() && !urls.is_null() {
            let js = js_sys::Reflect::get(&urls, &JsValue::from_str("js"))
                .ok()
                .and_then(|v| v.as_string())
                .filter(|s| !s.is_empty());
            let wasm = js_sys::Reflect::get(&urls, &JsValue::from_str("wasm"))
                .ok()
                .and_then(|v| v.as_string())
                .filter(|s| !s.is_empty());
            if let (Some(js), Some(wasm)) = (js, wasm) {
                return Ok((js, wasm));
            }
        }
    }

    let document = window.document().ok_or("missing document")?;
    let js = document
        .query_selector("link[rel=\"modulepreload\"]")
        .map_err(|_| "query_selector failed")?
        .and_then(|el| el.get_attribute("href"))
        .filter(|s| !s.is_empty())
        .ok_or("missing modulepreload link")?;
    let wasm = document
        .query_selector("link[rel=\"preload\"][as=\"fetch\"]")
        .map_err(|_| "query_selector failed")?
        .and_then(|el| el.get_attribute("href"))
        .filter(|s| !s.is_empty())
        .ok_or("missing wasm preload link")?;
    Ok((js, wasm))
}

struct WorkerHandle {
    worker: Worker,
    ready: Rc<Cell<bool>>,
}

#[derive(Clone, Copy)]
enum JobKind {
    Load,
    Remesh,
}

struct JobMeta {
    coord: (i32, i32),
    kind: JobKind,
}

struct WorkerBridgeInner {
    workers: Vec<WorkerHandle>,
    next_worker: usize,
    next_job_id: u32,
    job_queue: VecDeque<WorkerJob>,
    pending_jobs: HashMap<u32, JobMeta>,
    load_busy_coords: HashSet<(i32, i32)>,
    busy_coords: HashSet<(i32, i32)>,
}

impl WorkerBridgeInner {
    fn job_coord(job: &WorkerJob) -> (i32, i32) {
        match job {
            WorkerJob::LoadChunk { cx, cz, .. } | WorkerJob::RemeshChunk { cx, cz, .. } => (*cx, *cz),
        }
    }

    fn job_kind(job: &WorkerJob) -> JobKind {
        match job {
            WorkerJob::LoadChunk { .. } => JobKind::Load,
            WorkerJob::RemeshChunk { .. } => JobKind::Remesh,
        }
    }

    fn ready_worker_indices(&self) -> Vec<usize> {
        self.workers
            .iter()
            .enumerate()
            .filter(|(_, handle)| handle.ready.get())
            .map(|(index, _)| index)
            .collect()
    }

    fn max_in_flight(&self) -> usize {
        self.ready_worker_indices().len() * crate::platform::max_worker_jobs_in_flight()
    }

    fn dispatch_next_jobs(&mut self) {
        let ready_workers = self.ready_worker_indices();
        if ready_workers.is_empty() {
            return;
        }
        while self.pending_jobs.len() < self.max_in_flight() {
            let Some(job) = self.job_queue.pop_front() else {
                break;
            };
            let worker_index = ready_workers[self.next_worker % ready_workers.len()];
            self.next_worker = self.next_worker.wrapping_add(1);
            let payload = encode_job(&job);
            let payload_array = js_sys::Uint8Array::from(payload.as_slice());
            let payload_buffer = payload_array.buffer();

            let job_id = match &job {
                WorkerJob::LoadChunk { id, .. } | WorkerJob::RemeshChunk { id, .. } => *id,
            };
            let coord = Self::job_coord(&job);
            let kind = Self::job_kind(&job);

            let message = js_sys::Object::new();
            let _ = js_sys::Reflect::set(&message, &JsValue::from_str("type"), &JsValue::from_str("job"));
            let _ = js_sys::Reflect::set(
                &message,
                &JsValue::from_str("jobId"),
                &JsValue::from_f64(job_id as f64),
            );
            let _ = js_sys::Reflect::set(&message, &JsValue::from_str("payload"), &payload_buffer);
            if self.workers[worker_index]
                .worker
                .post_message(&message)
                .is_err()
            {
                self.job_queue.push_front(job);
                break;
            }
            self.pending_jobs.insert(job_id, JobMeta { coord, kind });
            if matches!(kind, JobKind::Load) {
                self.load_busy_coords.insert(coord);
            }
            self.busy_coords.insert(coord);
        }
    }

    fn enqueue_job(&mut self, job: WorkerJob) {
        if self.job_queue.len() >= crate::platform::max_worker_job_queue() {
            return;
        }
        let coord = Self::job_coord(&job);
        if matches!(Self::job_kind(&job), JobKind::Load) {
            self.load_busy_coords.insert(coord);
        }
        self.busy_coords.insert(coord);
        self.job_queue.push_back(job);
        self.dispatch_next_jobs();
    }

    fn finish_job(&mut self, job_id: u32) -> Option<JobMeta> {
        let meta = self.pending_jobs.remove(&job_id)?;
        if !self
            .pending_jobs
            .values()
            .any(|pending| pending.coord == meta.coord)
            && !self
                .job_queue
                .iter()
                .any(|job| Self::job_coord(job) == meta.coord)
        {
            self.busy_coords.remove(&meta.coord);
            if matches!(meta.kind, JobKind::Load) {
                self.load_busy_coords.remove(&meta.coord);
            }
        }
        Some(meta)
    }

    fn fail_job(&mut self, job_id: u32, message: &str) {
        let Some(meta) = self.finish_job(job_id) else {
            web_sys::console::error_1(&JsValue::from_str(message));
            return;
        };
        push_failure(WorkerFailure {
            coord: meta.coord,
            is_remesh: matches!(meta.kind, JobKind::Remesh),
        });
        web_sys::console::error_1(&JsValue::from_str(message));
    }

    fn process_inbound_events(&mut self) {
        for event in take_events() {
            match event {
                InboundWorkerEvent::Ready { worker_id } => {
                    if worker_id < self.workers.len() {
                        self.workers[worker_id].ready.set(true);
                    }
                }
                InboundWorkerEvent::Completed {
                    expected_job_id,
                    reply,
                } => {
                    let reply_id = match &reply {
                        WorkerReply::LoadChunk { id, .. }
                        | WorkerReply::RemeshChunk { id, .. }
                        | WorkerReply::Error { id, .. } => *id,
                    };
                    if reply_id != expected_job_id {
                        self.fail_job(
                            expected_job_id,
                            &format!(
                                "worker reply id mismatch (expected {expected_job_id}, got {reply_id})"
                            ),
                        );
                        continue;
                    }
                    match reply {
                        WorkerReply::Error { message, id } => {
                            self.fail_job(id, &message);
                        }
                        reply => {
                            let _ = self.finish_job(expected_job_id);
                            push_reply(reply);
                        }
                    }
                }
                InboundWorkerEvent::Failed { job_id, message } => {
                    self.fail_job(job_id, &message);
                }
            }
        }
        self.dispatch_next_jobs();
    }
}

pub struct WorkerBridge {
    inner: Rc<RefCell<WorkerBridgeInner>>,
}

/// What a worker needs to reproduce the world: raw zip bytes, or just a seed.
pub enum WorkerSource<'a> {
    Zip(&'a [u8]),
    Seed(i64),
}

impl WorkerBridge {
    pub fn start(source: WorkerSource<'_>, worker_count: usize) -> Result<Self, String> {
        let (wasm_js, wasm_module) = wasm_urls()?;
        let worker_count = worker_count.clamp(1, 4);
        let inner = Rc::new(RefCell::new(WorkerBridgeInner {
            workers: Vec::with_capacity(worker_count),
            next_worker: 0,
            next_job_id: 1,
            job_queue: VecDeque::new(),
            pending_jobs: HashMap::new(),
            load_busy_coords: HashSet::new(),
            busy_coords: HashSet::new(),
        }));

        // Build the zip payload once. It used to be re-allocated per worker, which for a
        // 46 MB save meant several full copies before postMessage even cloned it.
        let zip_buffer = match &source {
            WorkerSource::Zip(bytes) => {
                let array = js_sys::Uint8Array::new_with_length(bytes.len() as u32);
                array.copy_from(bytes);
                Some(array.buffer())
            }
            WorkerSource::Seed(_) => None,
        };

        for worker_id in 0..worker_count {
            inner.borrow_mut().workers.push(WorkerHandle {
                worker: {
                    let options = WorkerOptions::new();
                    options.set_type(WorkerType::Module);
                    Worker::new_with_options(WORKER_SCRIPT, &options)
                        .map_err(|err| format!("worker spawn failed: {err:?}"))?
                },
                ready: Rc::new(Cell::new(false)),
            });

            let worker = inner.borrow().workers[worker_id].worker.clone();
            let onmessage = Closure::<dyn FnMut(MessageEvent)>::new(move |event: MessageEvent| {
                let data_value = event.data();
                let Some(data) = data_value.dyn_ref::<js_sys::Object>() else {
                    return;
                };
                let ty = js_sys::Reflect::get(data, &JsValue::from_str("type"))
                    .ok()
                    .and_then(|v| v.as_string());
                match ty.as_deref() {
                    // Proof the worker is alive and received the message, logged before it
                    // has done any wasm work. Distinguishes "never arrived" from "still
                    // starting up".
                    Some("ack") => {
                        let for_type = js_sys::Reflect::get(data, &JsValue::from_str("forType"))
                            .ok()
                            .and_then(|v| v.as_string())
                            .unwrap_or_else(|| "?".to_string());
                        let worker_id = js_sys::Reflect::get(data, &JsValue::from_str("workerId"))
                            .ok()
                            .and_then(|v| v.as_f64())
                            .map(|v| v as usize);
                        web_sys::console::log_1(&JsValue::from_str(&format!(
                            "[worker] ack from {worker_id:?} for '{for_type}'"
                        )));
                    }
                    Some("ready") => {
                        let worker_id = js_sys::Reflect::get(data, &JsValue::from_str("workerId"))
                            .ok()
                            .and_then(|v| v.as_f64())
                            .map(|v| v as usize);
                        if let Some(worker_id) = worker_id {
                            push_event(InboundWorkerEvent::Ready { worker_id });
                        }
                        crate::web_api::kick_event_loop();
                    }
                    Some("result") => {
                        // Log the first few round trips so a reply that never arrives is
                        // distinguishable from one that arrives and is then discarded.
                        {
                            use std::sync::atomic::{AtomicU32, Ordering};
                            static SEEN: AtomicU32 = AtomicU32::new(0);
                            let n = SEEN.fetch_add(1, Ordering::Relaxed);
                            if n < 10 {
                                web_sys::console::log_1(&JsValue::from_str(&format!(
                                    "[worker] result #{n} received by main thread"
                                )));
                            }
                        }
                        let job_id = js_sys::Reflect::get(data, &JsValue::from_str("jobId"))
                            .ok()
                            .and_then(|v| v.as_f64())
                            .map(|v| v as u32);
                        // The worker posts `payload: out.buffer`, i.e. an ArrayBuffer, and
                        // transfers it. An ArrayBuffer is not `instanceof Uint8Array`, so
                        // casting straight to Uint8Array silently yielded None and dropped
                        // every reply on the floor — which pinned pending_jobs at
                        // max_in_flight and deadlocked dispatch. Accept both shapes.
                        let payload = js_sys::Reflect::get(data, &JsValue::from_str("payload"))
                            .ok()
                            .and_then(|value| {
                                if value.is_instance_of::<js_sys::Uint8Array>() {
                                    value.dyn_into::<js_sys::Uint8Array>().ok()
                                } else if value.is_instance_of::<js_sys::ArrayBuffer>() {
                                    Some(js_sys::Uint8Array::new(&value))
                                } else {
                                    web_sys::console::error_1(&JsValue::from_str(
                                        "worker reply payload was neither Uint8Array nor ArrayBuffer",
                                    ));
                                    None
                                }
                            });
                        if let (Some(job_id), Some(bytes)) = (job_id, payload) {
                            let mut vec = vec![0u8; bytes.length() as usize];
                            bytes.copy_to(&mut vec);
                            match decode_reply(&vec) {
                                Ok(reply) => {
                                    push_event(InboundWorkerEvent::Completed {
                                        expected_job_id: job_id,
                                        reply,
                                    });
                                }
                                Err(err) => {
                                    push_event(InboundWorkerEvent::Failed {
                                        job_id,
                                        message: format!("worker reply decode failed: {err}"),
                                    });
                                }
                            }
                        } else {
                            // Dropping a reply here leaks a pending job forever, which
                            // eventually pins pending_jobs at max_in_flight and stalls all
                            // dispatch. Never let that happen quietly again.
                            web_sys::console::error_1(&JsValue::from_str(
                                "worker reply missing jobId or payload — job would leak",
                            ));
                        }
                        crate::web_api::kick_event_loop();
                    }
                    Some("error") => {
                        let job_id = js_sys::Reflect::get(data, &JsValue::from_str("jobId"))
                            .ok()
                            .and_then(|v| v.as_f64())
                            .map(|v| v as u32);
                        let message = js_sys::Reflect::get(data, &JsValue::from_str("message"))
                            .ok()
                            .and_then(|v| v.as_string())
                            .unwrap_or_else(|| "worker error".to_string());
                        if let Some(job_id) = job_id {
                            push_event(InboundWorkerEvent::Failed { job_id, message });
                        } else {
                            web_sys::console::error_1(&JsValue::from_str(&message));
                        }
                        crate::web_api::kick_event_loop();
                    }
                    _ => {}
                }
            });
            worker.set_onmessage(Some(onmessage.as_ref().unchecked_ref()));
            onmessage.forget();

            // Without this, a worker script that fails to load or throws during startup is
            // completely silent: the Worker object exists, messages queue forever, and
            // nothing ever reports ready.
            let onerror = Closure::<dyn FnMut(web_sys::ErrorEvent)>::new(
                move |event: web_sys::ErrorEvent| {
                    web_sys::console::error_1(&JsValue::from_str(&format!(
                        "chunk worker {worker_id} script error: {} ({}:{})",
                        event.message(),
                        event.filename(),
                        event.lineno()
                    )));
                },
            );
            worker.set_onerror(Some(onerror.as_ref().unchecked_ref()));
            onerror.forget();

            let init = js_sys::Object::new();
            let _ = js_sys::Reflect::set(&init, &JsValue::from_str("type"), &JsValue::from_str("init"));
            let _ = js_sys::Reflect::set(
                &init,
                &JsValue::from_str("workerId"),
                &JsValue::from_f64(worker_id as f64),
            );
            let _ = js_sys::Reflect::set(
                &init,
                &JsValue::from_str("wasmJs"),
                &JsValue::from_str(&wasm_js),
            );
            let _ = js_sys::Reflect::set(
                &init,
                &JsValue::from_str("wasmModule"),
                &JsValue::from_str(&wasm_module),
            );
            match (&source, &zip_buffer) {
                (WorkerSource::Seed(seed), _) => {
                    let _ = js_sys::Reflect::set(
                        &init,
                        &JsValue::from_str("seed"),
                        &js_sys::BigInt::from(*seed).into(),
                    );
                }
                (WorkerSource::Zip(_), Some(buffer)) => {
                    let _ = js_sys::Reflect::set(&init, &JsValue::from_str("zipBytes"), buffer);
                }
                (WorkerSource::Zip(_), None) => {}
            }
            if let Err(err) = inner.borrow().workers[worker_id].worker.post_message(&init) {
                return Err(format!(
                    "worker {worker_id} init post_message failed: {err:?}"
                ));
            }
        }

        Ok(Self { inner })
    }

    pub fn is_coord_busy(&self, coord: (i32, i32)) -> bool {
        self.inner.borrow().busy_coords.contains(&coord)
    }

    pub fn is_load_busy(&self, coord: (i32, i32)) -> bool {
        self.inner.borrow().load_busy_coords.contains(&coord)
    }

    pub fn release_load_coord(&mut self, coord: (i32, i32)) {
        let mut inner = self.inner.borrow_mut();
        inner.load_busy_coords.remove(&coord);
        inner.job_queue.retain(|job| {
            !matches!(job, WorkerJob::LoadChunk { cx, cz, .. } if (*cx, *cz) == coord)
        });
        let has_pending = inner
            .pending_jobs
            .values()
            .any(|meta| meta.coord == coord);
        let has_queued = inner
            .job_queue
            .iter()
            .any(|job| WorkerBridgeInner::job_coord(job) == coord);
        if !has_pending && !has_queued {
            inner.busy_coords.remove(&coord);
        }
    }

    pub fn can_accept_job(&self) -> bool {
        let inner = self.inner.borrow();
        inner.job_queue.len() + inner.pending_jobs.len() < crate::platform::max_worker_job_queue()
    }

    pub fn try_request_load(&mut self, coord: (i32, i32)) -> bool {
        if self.is_coord_busy(coord) || !self.can_accept_job() || self.workers_ready_count() == 0 {
            return false;
        }
        self.request_load(coord);
        true
    }

    pub fn request_load(&mut self, coord: (i32, i32)) {
        let mut inner = self.inner.borrow_mut();
        let id = inner.next_job_id;
        inner.next_job_id = inner.next_job_id.wrapping_add(1);
        inner.enqueue_job(WorkerJob::LoadChunk {
            id,
            cx: coord.0,
            cz: coord.1,
        });
    }

    pub fn request_remesh(&mut self, coord: (i32, i32)) {
        if !self.can_accept_job() {
            return;
        }
        let mut inner = self.inner.borrow_mut();
        let id = inner.next_job_id;
        inner.next_job_id = inner.next_job_id.wrapping_add(1);
        inner.enqueue_job(WorkerJob::RemeshChunk {
            id,
            cx: coord.0,
            cz: coord.1,
        });
    }

    pub fn poll_into(&mut self, out: &mut Vec<MeshedChunk>, limit: usize) {
        self.inner.borrow_mut().process_inbound_events();

        let replies = take_replies();
        for reply in replies {
            if out.len() >= limit {
                WORKER_INBOX.with(|inbox| inbox.borrow_mut().push(reply));
                continue;
            }
            let Some(meshed) = reply_to_meshed(reply) else {
                continue;
            };
            out.push(meshed);
        }
        self.inner.borrow_mut().dispatch_next_jobs();
    }

    pub fn workers_ready_count(&self) -> usize {
        self.inner
            .borrow()
            .workers
            .iter()
            .filter(|handle| handle.ready.get())
            .count()
    }

    pub fn worker_count(&self) -> usize {
        self.inner.borrow().workers.len()
    }

    pub fn pending_job_count(&self) -> usize {
        let inner = self.inner.borrow();
        inner.job_queue.len() + inner.pending_jobs.len()
    }

    pub fn queued_job_count(&self) -> usize {
        self.inner.borrow().job_queue.len()
    }

    pub fn in_flight_job_count(&self) -> usize {
        self.inner.borrow().pending_jobs.len()
    }
}

fn reply_to_meshed(reply: WorkerReply) -> Option<MeshedChunk> {
    match reply {
        WorkerReply::LoadChunk {
            chunk,
            section_meshes,
            ..
        } => {
            let section_meshes = section_meshes
                .into_iter()
                .filter_map(SectionMeshWire::into_mesh)
                .collect();
            Some(MeshedChunk {
                coord: (chunk.cx, chunk.cz),
                chunk: Some(chunk.into_chunk()),
                section_meshes,
                is_remesh: false,
                // Section connectivity is not on the worker wire yet. Absent visibility reads
                // as "fully transparent", which only ever draws too much — and section
                // occlusion culling is off on wasm regardless.
                visibility: None,
            })
        }
        WorkerReply::RemeshChunk {
            cx,
            cz,
            section_meshes,
            ..
        } => {
            let section_meshes = section_meshes
                .into_iter()
                .filter_map(SectionMeshWire::into_mesh)
                .collect();
            Some(MeshedChunk {
                coord: (cx, cz),
                chunk: None,
                section_meshes,
                is_remesh: true,
                visibility: None,
            })
        }
        WorkerReply::Error { .. } => None,
    }
}
