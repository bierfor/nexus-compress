//! IPC event throttling for the Tauri → WebView channel.
//!
//! Sprint 5.7 hotfix #20: compression / decompression progress events
//! fire per chunk (potentially hundreds per second on fast disks:
//! 49 MB/s @ 64 KiB chunks = ~750 events/sec). At that rate the
//! WebView IPC channel saturates and the UI freezes on lower-end
//! laptops (especially Windows). We instead:
//!
//!   1. Buffer incoming events into a "latest" slot.
//!   2. Emit at most every `THROTTLE_MS` milliseconds.
//!   3. Force-emit on completion/done/error so the UI never sees
//!      a stale value.
//!
//! The implementation uses an `Arc<Mutex<...>>` shared between the
//! blocking compression thread (which calls `feed()`) and an
//! internal worker task that owns the timer. The worker lives for
//! the lifetime of `ThrottledEmitter` (which is the lifetime of one
//! command invocation).

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::{AppHandle, Emitter};

/// Minimum interval between flushes to the IPC channel.
pub const THROTTLE_MS: u64 = 100;

/// Maximum time we buffer a "fresh" event before flushing even
/// without a new write. Without this, a one-shot write (e.g. the
/// very first progress event) could be delayed indefinitely if the
/// throttler never re-fires. 200ms is comfortably above 60 Hz
/// refresh rates (16ms) so we never sleep through a frame boundary.
const MAX_BUFFER_MS: u64 = 200;

/// What the worker thread emits. We store only the LATEST pending
/// event per channel — earlier events are replaced by newer ones.
/// Progress events are monotonic-additive, so the last value is
/// always the most informative.
struct Pending {
    name: &'static str,
    value: serde_json::Value,
    emitted_at: Option<Instant>,
    last_input_at: Instant,
}

pub struct ThrottledEmitter {
    pending: Arc<Mutex<Option<Pending>>>,
    app: AppHandle,
    /// Sprint 5.7.2 hotfix #37: shutdown signal for the drain thread.
    /// The drain loop checks this between sleeps; when the
    /// ThrottledEmitter is dropped we flip it to true so the worker
    /// exits cleanly instead of being leaked.
    shutdown: Arc<std::sync::atomic::AtomicBool>,
}

impl ThrottledEmitter {
    pub fn new(app: AppHandle) -> Self {
        let pending = Arc::new(Mutex::new(None::<Pending>));
        let shutdown = Arc::new(std::sync::atomic::AtomicBool::new(false));
        // Spawn a worker that drains the buffer every MAX_BUFFER_MS.
        // The worker now exits cleanly when `shutdown` is flipped to
        // true (see Drop impl) — no more zombie threads after every
        // compress/decompress call.
        let pending_for_thread = pending.clone();
        let app_for_thread = app.clone();
        let shutdown_for_thread = shutdown.clone();
        std::thread::Builder::new()
            .name("throttle-emitter".into())
            .spawn(move || drain_loop(pending_for_thread, app_for_thread, shutdown_for_thread))
            .expect("failed to spawn throttle-emitter thread");
        Self { pending, app, shutdown }
    }

    /// Push a new event. If the throttle window has elapsed since
    /// the last emit, this event is flushed immediately; otherwise
    /// it overwrites the buffer (latest-wins for monotonic counters).
    ///
    /// Sprint 5.7.2 hotfix #37c: once shutdown is signaled (Drop),
    /// feed() becomes a no-op. Without this, a slow codec still
    /// running its last chunks would keep refilling `pending` and
    /// the drain thread would emit the same stale "bytes_done=N"
    /// event forever, making the UI appear stuck.
    pub fn feed<T: Serialize>(&self, name: &'static str, value: &T) {
        if self.shutdown.load(std::sync::atomic::Ordering::SeqCst) {
            return;
        }
        let json = match serde_json::to_value(value) {
            Ok(v) => v,
            Err(_) => return, // serialization failure: silently skip
                              // (it's just progress, never a real error).
        };
        let now = Instant::now();

        let mut guard = match self.pending.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(), // poisoned mutex: still usable
        };
        match guard.as_mut() {
            // No pending event — just record the input time and value.
            None => {
                *guard = Some(Pending {
                    name,
                    value: json,
                    emitted_at: None,
                    last_input_at: now,
                });
            }
            Some(p) => {
                let elapsed_since_last_emit = p
                    .emitted_at
                    .map(|t| now.duration_since(t).as_millis() as u64)
                    .unwrap_or(u64::MAX);
                if elapsed_since_last_emit >= THROTTLE_MS {
                    // Throttle window elapsed AND no thread is
                    // currently scheduled to emit (we're in feed,
                    // not the timer). Flush synchronously so the
                    // caller doesn't accumulate stale events.
                    let _ = self.app.emit(name, json.clone());
                    p.emitted_at = Some(now);
                    p.value = json;
                    p.name = name;
                    p.last_input_at = now;
                } else {
                    // Buffer the latest value.
                    p.value = json;
                    p.name = name;
                    p.last_input_at = now;
                }
            }
        }
    }

    /// Force-flush whatever is pending. Used on completion / abort.
    pub fn flush(&self) {
        let mut guard = match self.pending.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        if let Some(mut p) = guard.take() {
            // TEMP DEBUG: dump the buffered event so we can see
            // whether elapsed_ms is actually populated correctly
            // when it reaches the IPC channel.
            eprintln!("[THROTTLE:FLUSH] name={} elapsed_ms={} phase={}",
                p.name,
                p.value.get("elapsed_ms").map(|v| v.to_string()).unwrap_or_else(|| "?".into()),
                p.value.get("phase").map(|v| v.to_string()).unwrap_or_else(|| "?".into()));
            let _ = self.app.emit(p.name, p.value);
            p.emitted_at = Some(Instant::now());
        }
    }
}

impl Drop for ThrottledEmitter {
    fn drop(&mut self) {
        // Sprint 5.7.2 hotfix #37: signal the drain thread to exit
        // BEFORE flushing, so the worker doesn't race the flush()
        // for the pending mutex. Without this, the worker would
        // emit stale events to the GUI even after the codec
        // finished, making the UI appear to "stuck at 100%".
        self.shutdown.store(true, std::sync::atomic::Ordering::SeqCst);
        // On drop, make sure we don't leave buffered events behind.
        self.flush();
    }
}

/// Background drainer. Wakes every MAX_BUFFER_MS milliseconds and
/// emits whatever's in the buffer (if its last_input_at is older
/// than THROTTLE_MS). Exits cleanly when the parent ThrottledEmitter
/// is dropped (shutdown flag set to true).
fn drain_loop(
    pending: Arc<Mutex<Option<Pending>>>,
    app: AppHandle,
    shutdown: Arc<std::sync::atomic::AtomicBool>,
) {
    let interval = Duration::from_millis(MAX_BUFFER_MS);
    loop {
        // Check shutdown BEFORE sleeping so the Drop path is fast.
        if shutdown.load(std::sync::atomic::Ordering::SeqCst) {
            return;
        }
        std::thread::sleep(interval);
        // Check shutdown AGAIN after waking up — the Drop path may
        // have signaled shutdown while we were sleeping. Without
        // this second check, we'd run one more iteration and emit
        // a stale event. Sprint 5.7.2 hotfix #37b.
        if shutdown.load(std::sync::atomic::Ordering::SeqCst) {
            return;
        }
        // Atomically take the pending event IF it has been
        // sitting in the buffer longer than THROTTLE_MS.
        // Taking it (instead of cloning) ensures we don't
        // re-emit the same stale JSON on every tick — the
        // next tick sees None and `continue`s. This matches
        // what `flush()` does, so trailing-edge behavior is
        // consistent regardless of which one wins the race.
        let to_emit: Option<(String, serde_json::Value)> = {
            let mut guard = match pending.lock() {
                Ok(g) => g,
                Err(p) => p.into_inner(),
            };
            let p = match guard.as_mut() {
                Some(p) => p,
                None => continue,
            };
            let now = Instant::now();
            let since_input = now.duration_since(p.last_input_at).as_millis() as u64;
            if since_input < THROTTLE_MS {
                continue;
            }
            // Atomically take ownership of the event out of
            // the slot. The slot becomes None — any subsequent
            // feed() will write a fresh Pending struct.
            let owned = guard.take();
            owned.map(|p| (p.name.to_string(), p.value))
        };
        if let Some((name, value)) = to_emit {
            // TEMP DEBUG: log elapsed_ms of emitted event
            eprintln!("[THROTTLE:DRAIN-EMIT] name={} elapsed_ms={} phase={} bytes_done={}",
                name,
                value.get("elapsed_ms").map(|v| v.to_string()).unwrap_or_else(|| "?".into()),
                value.get("phase").map(|v| v.to_string()).unwrap_or_else(|| "?".into()),
                value.get("bytes_done").map(|v| v.to_string()).unwrap_or_else(|| "?".into()));
            let _ = app.emit(&name, value);
        }
    }
}
