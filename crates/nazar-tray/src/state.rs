//! What the panel is allowed to ask for, and the state it asks.
//!
//! The refresh loop owns the readers and runs on its own thread; the panel runs in a
//! webview and knows nothing about any of that. Between them there is one shared snapshot
//! behind a mutex, one event, and three commands:
//!
//! | | |
//! |---|---|
//! | `get_snapshot` | the derived view, worked out **now** |
//! | `get_warnings` | the loop's counters, for a diagnostic line |
//! | `refresh_now` | ask the loop for a pass; the audit's finding B13 is why it exists |
//! | `snapshot-changed` (event) | the document moved; ask again |
//!
//! `get_snapshot` derives rather than returns: which window binds and how long is left both
//! change with the clock and neither is stored, so the answer is computed for the instant
//! the panel asked. That is what makes the countdown right after a machine wakes up without
//! anything having refreshed.

use std::sync::{Arc, Mutex, PoisonError};

use nazar_core::now_rfc3339;
use nazar_core::refresh::{LoopHandle, Warnings};
use nazar_core::state::{Rules, Snapshot, SnapshotView};

/// The event the panel listens for. Emitted only when the document actually changed.
pub const SNAPSHOT_CHANGED: &str = "snapshot-changed";

/// Everything the commands need.
pub struct AppState {
    snapshot: Arc<Mutex<Snapshot>>,
    warnings: Arc<Mutex<Warnings>>,
    rules: Rules,
    refresher: Mutex<Option<LoopHandle>>,
}

impl AppState {
    /// Wrap the handles the engine handed out.
    pub fn new(
        snapshot: Arc<Mutex<Snapshot>>,
        warnings: Arc<Mutex<Warnings>>,
        rules: Rules,
    ) -> Self {
        AppState {
            snapshot,
            warnings,
            rules,
            refresher: Mutex::new(None),
        }
    }

    /// Keep the loop's handle, so the panel can ask for a refresh and shutdown can stop it.
    pub fn attach(&self, handle: LoopHandle) {
        *self
            .refresher
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(handle);
    }

    /// The derived view, for right now.
    pub fn view(&self) -> SnapshotView {
        self.snapshot
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .view(&now_rfc3339(), &self.rules)
    }

    /// The loop's counters.
    pub fn warnings(&self) -> Warnings {
        self.warnings
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Ask the loop for a pass. `false` when there is no loop, which is what a machine with
    /// no home directory looks like.
    pub fn refresh(&self) -> bool {
        self.refresher
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .is_some_and(LoopHandle::refresh_now)
    }

    /// Stop the loop and let it release the advisory lock.
    ///
    /// Called when the application exits. Without it the lock file would be left behind for
    /// its five-minute grace period, and the next launch would start as a reader.
    pub fn shutdown(&self) {
        if let Some(handle) = self
            .refresher
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
        {
            handle.stop();
        }
    }
}

/// The panel's view of the numbers, derived for the moment it asked.
#[tauri::command]
pub fn get_snapshot(state: tauri::State<'_, AppState>) -> SnapshotView {
    state.view()
}

/// The loop's counters, for a diagnostic line in the panel.
#[tauri::command]
pub fn get_warnings(state: tauri::State<'_, AppState>) -> Warnings {
    state.warnings()
}

/// Ask for a refresh now.
#[tauri::command]
pub fn refresh_now(state: tauri::State<'_, AppState>) -> bool {
    state.refresh()
}
