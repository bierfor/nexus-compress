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
}

impl ThrottledEmitter {
    pub fn new(app: AppHandle) -> Self {
        let pending = Arc::new(Mutex::new(None::<Pending>));
        // Spawn a worker that drains the buffer every MAX_BUFFER_MS.
        // The worker runs until the app handle is dropped (which is
        // for the entire process lifetime in practice, since
        // ThrottledEmitter is created per command invocation but the
        // worker holds a clone of `pending`, not `app`).
        //
        // We leak a Thread on purpose — leaking a std::thread is
        // cheaper than coordinating a shutdown channel and the
        // thread only wakes up every MAX_BUFFER_MS.
        let pending_for_thread = pending.clone();
        let app_for_thread = app.clone();
        std::thread::Builder::new()
            .name("throttle-emitter".into())
            .spawn(move || drain_loop(pending_for_thread, app_for_thread))
            .expect("failed to spawn throttle-emitter thread");
        Self { pending, app }
    }

    /// Push a new event. If the throttle window has elapsed since
    /// the last emit, this event is flushed immediately; otherwise
    /// it overwrites the buffer (latest-wins for monotonic counters).
    pub fn feed<T: Serialize>(&self, name: &'static str, value: &T) {
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
            let _ = self.app.emit(p.name, p.value);
            p.emitted_at = Some(Instant::now());
        }
    }
}

impl Drop for ThrottledEmitter {
    fn drop(&mut self) {
        // On drop, make sure we don't leave buffered events behind.
        self.flush();
    }
}

/// Background drainer. Wakes every MAX_BUFFER_MS milliseconds and
/// emits whatever's in the buffer (if its last_input_at is older
/// than THROTTLE_MS). Idles out cleanly when the app shuts down.
fn drain_loop(pending: Arc<Mutex<Option<Pending>>>, app: AppHandle) {
    let interval = Duration::from_millis(MAX_BUFFER_MS);
    loop {
        std::thread::sleep(interval);
        // Snapshot under lock, then release before emit so the
        // IPC call doesn't block future feeds.
        let to_emit: Option<(&'static str, serde_json::Value)> = {
            let mut guard = match pending.lock() {
                Ok(g) => g,
                Err(p) => p.into_inner(),
            };
            // If no event is pending, continue (don't return —
            // the previous version's `return None` bug here
            // would kill the drainer thread on the first
            // empty tick, leaving any future events in the
            // buffer with no one to flush them).
            let pending = match guard.as_mut() {
                Some(p) => p,
                None => continue,
            };
            let now = Instant::now();
            let since_input = now.duration_since(pending.last_input_at).as_millis() as u64;
            // If the buffer has been touched recently, skip
            // this tick — the synchronous flush in `feed()`
            // handles the "events arriving faster than
            // THROTTLE_MS" case. We just need the drainer
            // to be alive for the "trailing edge" case
            // (the last event of a compress arrived less
            // than THROTTLE_MS ago and no more are coming).
            // Use `continue` here, NOT `return` — the
            // previous `return` bug would kill the drainer
            // thread the first time we hit this branch,
            // and then the trailing edge would never flush.
            if since_input < THROTTLE_MS {
                continue;
            }
            pending.emitted_at = Some(now);
            Some((pending.name, pending.value.clone()))
        };
        if let Some((name, value)) = to_emit {
            let _ = app.emit(name, value);
        }
    }
}
