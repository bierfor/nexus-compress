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

use crate::api::CorpusBreakdown;

static SKIPPED_BYTES: AtomicU64 = AtomicU64::new(0);
static LAST_BREAKDOWN_BUILD: AtomicU64 = AtomicU64::new(0);
static LAST_BREAKDOWN_BUILD_BYTES: AtomicU64 = AtomicU64::new(0);
static LAST_BREAKDOWN_SOURCE: AtomicU64 = AtomicU64::new(0);
static LAST_BREAKDOWN_SOURCE_BYTES: AtomicU64 = AtomicU64::new(0);
static LAST_BREAKDOWN_OTHER: AtomicU64 = AtomicU64::new(0);
static LAST_BREAKDOWN_OTHER_BYTES: AtomicU64 = AtomicU64::new(0);

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

/// Sprint 5.7.9 part 6: record a snapshot of the corpus
/// breakdown (source bytes / build-artifact bytes / other
/// bytes). The UI uses this to warn the user when their
/// archive is dominated by build artifacts (where no
/// codec can do much).
pub fn record_corpus_breakdown(b: &CorpusBreakdown) {
    LAST_BREAKDOWN_BUILD.store(b.build_artifact_files, Ordering::SeqCst);
    LAST_BREAKDOWN_BUILD_BYTES.store(b.build_artifact_bytes, Ordering::SeqCst);
    LAST_BREAKDOWN_SOURCE.store(b.source_files, Ordering::SeqCst);
    LAST_BREAKDOWN_SOURCE_BYTES.store(b.source_bytes, Ordering::SeqCst);
    LAST_BREAKDOWN_OTHER.store(b.other_files, Ordering::SeqCst);
    LAST_BREAKDOWN_OTHER_BYTES.store(b.other_bytes, Ordering::SeqCst);
}

/// Read the last corpus breakdown. Returns zeros if no
/// compression has been run yet, or if the input was a
/// single file (the breakdown is only computed for
/// directories).
pub fn last_corpus_breakdown() -> CorpusBreakdown {
    CorpusBreakdown {
        source_files: LAST_BREAKDOWN_SOURCE.load(Ordering::SeqCst),
        source_bytes: LAST_BREAKDOWN_SOURCE_BYTES.load(Ordering::SeqCst),
        build_artifact_files: LAST_BREAKDOWN_BUILD.load(Ordering::SeqCst),
        build_artifact_bytes: LAST_BREAKDOWN_BUILD_BYTES.load(Ordering::SeqCst),
        other_files: LAST_BREAKDOWN_OTHER.load(Ordering::SeqCst),
        other_bytes: LAST_BREAKDOWN_OTHER_BYTES.load(Ordering::SeqCst),
    }
}

/// Read-and-reset variant. Used by the compress command
/// to read the breakdown after a successful walk and
/// clear it for the next run.
pub fn take_corpus_breakdown() -> CorpusBreakdown {
    CorpusBreakdown {
        source_files: LAST_BREAKDOWN_SOURCE.swap(0, Ordering::SeqCst),
        source_bytes: LAST_BREAKDOWN_SOURCE_BYTES.swap(0, Ordering::SeqCst),
        build_artifact_files: LAST_BREAKDOWN_BUILD.swap(0, Ordering::SeqCst),
        build_artifact_bytes: LAST_BREAKDOWN_BUILD_BYTES.swap(0, Ordering::SeqCst),
        other_files: LAST_BREAKDOWN_OTHER.swap(0, Ordering::SeqCst),
        other_bytes: LAST_BREAKDOWN_OTHER_BYTES.swap(0, Ordering::SeqCst),
    }
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