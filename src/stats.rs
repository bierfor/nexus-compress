//! Process-wide statistics exposed to the UI.
//!
//! Sprint 5.7.2 hotfix #44 (Opción D — transparency): the walk phase
//! can skip large build caches (`.next`, `node_modules`, etc.). The
//! UI should show a small note in the success card explaining how
//! many bytes were skipped, so a 50% ratio on a corpus like
//! `secretaria/` (50% of bytes are `.next` cache) doesn't look like
//! a bug.
//!
//! We use a global atomic because Tauri commands run on multiple
//! threads and the front-end reads the snapshot when the success
//! card renders. A mutex would be overkill for a single u64 read.

use std::sync::atomic::{AtomicU64, Ordering};

static SKIPPED_BYTES: AtomicU64 = AtomicU64::new(0);

/// Record the number of bytes skipped during the most recent walk.
/// Called from `walk_dir_for_solid` once we know how much was
/// filtered out by the build-cache skip-list.
pub fn record_skipped_bytes(bytes: u64) {
    SKIPPED_BYTES.store(bytes, Ordering::SeqCst);
}

/// Read the most recent skipped-bytes value. The UI uses this to
/// show the "X MiB of dev cache skipped" tooltip.
pub fn last_skipped_bytes() -> u64 {
    SKIPPED_BYTES.load(Ordering::SeqCst)
}

/// Convenience: read + clear in one call (so a second compression
/// run starts with a clean state).
pub fn take_skipped_bytes() -> u64 {
    SKIPPED_BYTES.swap(0, Ordering::SeqCst)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_and_take() {
        record_skipped_bytes(1024);
        assert_eq!(last_skipped_bytes(), 1024);
        assert_eq!(take_skipped_bytes(), 1024);
        assert_eq!(last_skipped_bytes(), 0);
    }

    #[test]
    fn overwrite_takes_latest() {
        record_skipped_bytes(100);
        record_skipped_bytes(200);
        assert_eq!(take_skipped_bytes(), 200);
    }
}