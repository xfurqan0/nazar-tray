//! The refresh loop, driven by a clock the test moves rather than by waiting.
//!
//! Every test here except the last one calls [`Engine::tick`] directly. That is the point
//! of splitting the loop from its thread: a debounce, a sixty-second tick and an eight-hour
//! sleep are one line each, and the suite finishes in milliseconds instead of sleeping
//! through the behaviour it is checking.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::*;
use crate::clock::Clock;
use crate::limits::{Provider, Source, Window};
use crate::lock::{LockRecord, STALE_AFTER};
use crate::testutil::{ManualClock, TempDir};

/// A reader the test drives: it hands back whatever provider block it is told to.
struct FakeReader {
    key: &'static str,
    provider: Arc<Mutex<Provider>>,
    reads: Arc<AtomicU64>,
    warnings: Arc<AtomicU64>,
    watched: Vec<PathBuf>,
}

impl FakeReader {
    fn new(key: &'static str, provider: Provider) -> Self {
        FakeReader {
            key,
            provider: Arc::new(Mutex::new(provider)),
            reads: Arc::new(AtomicU64::new(0)),
            warnings: Arc::new(AtomicU64::new(0)),
            watched: Vec::new(),
        }
    }

    fn watching(mut self, path: impl Into<PathBuf>) -> Self {
        self.watched.push(path.into());
        self
    }

    /// Handles the test keeps so it can change what the reader returns mid-run.
    fn handle(&self) -> (Arc<Mutex<Provider>>, Arc<AtomicU64>, Arc<AtomicU64>) {
        (
            Arc::clone(&self.provider),
            Arc::clone(&self.reads),
            Arc::clone(&self.warnings),
        )
    }
}

impl Reader for FakeReader {
    fn key(&self) -> &'static str {
        self.key
    }

    fn read(&mut self, _clock: &dyn Clock, _now: &str) -> Provider {
        self.reads.fetch_add(1, Ordering::Relaxed);
        self.provider
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    fn warnings(&self) -> u64 {
        self.warnings.load(Ordering::Relaxed)
    }

    fn watched(&self) -> Vec<PathBuf> {
        self.watched.clone()
    }
}

/// A provider block with one window at `percent`.
fn reading(percent: f64) -> Provider {
    let mut windows = BTreeMap::new();
    windows.insert(
        "primary".to_owned(),
        Window::ok(percent)
            .with_window_minutes(300)
            .with_resets_at("2026-09-07T12:24:52Z"),
    );
    Provider {
        configured: true,
        source: Some(Source::Rollout),
        source_at: Some("2026-09-07T09:59:00Z".to_owned()),
        binding: Some("primary".to_owned()),
        windows,
        ..Provider::default()
    }
}

/// A provider block that says it could read nothing at all.
fn unreadable() -> Provider {
    let mut windows = BTreeMap::new();
    windows.insert(
        "primary".to_owned(),
        Window::error("no rollout log in the session directory").with_window_minutes(300),
    );
    Provider {
        configured: true,
        source: Some(Source::Rollout),
        windows,
        ..Provider::default()
    }
}

/// Everything one test needs: a throwaway directory, a clock and the events that fired.
struct Harness {
    dir: TempDir,
    clock: ManualClock,
    events: Arc<Mutex<Vec<Event>>>,
}

impl Harness {
    fn new(label: &str) -> Self {
        Harness {
            dir: TempDir::new(label),
            // The real instant, not a pinned one: `take_request` ages the request marker by
            // comparing this clock with the file's modification time, and the filesystem
            // stamps that from the system clock. See `ManualClock::at_real_now`.
            clock: ManualClock::at_real_now(),
            events: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn limits_path(&self) -> PathBuf {
        self.dir.join("limits.json")
    }

    fn lock_path(&self) -> PathBuf {
        self.dir.join("limits.lock")
    }

    fn lock(&self) -> LimitsLock {
        LimitsLock::acquire_as(
            &self.lock_path(),
            std::process::id(),
            "2026-09-07T10:00:00Z",
            &self.clock.now(),
            STALE_AFTER,
        )
        .unwrap()
        .held()
        .expect("the harness must get the lock")
    }

    fn engine(&self, readers: ReaderSet, lock: Option<LimitsLock>) -> Engine {
        let events = Arc::clone(&self.events);
        Engine::new(
            readers,
            self.limits_path(),
            lock,
            crate::state::Rules::default(),
            &self.clock.now(),
        )
        .on_event(move |event| {
            events
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(event);
        })
    }

    fn events(&self) -> Vec<Event> {
        self.events
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    fn document(&self) -> Option<Limits> {
        std::fs::read_to_string(self.limits_path())
            .ok()
            .and_then(|text| Limits::from_json(&text).ok())
    }
}

// ---------------------------------------------------------------- the basic cycle

#[test]
fn the_first_pass_reads_and_writes() {
    let harness = Harness::new("refresh-first");
    let codex = FakeReader::new("codex", reading(54.0));
    let (_, reads, _) = codex.handle();
    let mut engine = harness.engine(ReaderSet::new().with(Box::new(codex)), Some(harness.lock()));

    let cycle = engine.tick(&harness.clock);

    assert_eq!(cycle.cause, Some(Cause::Startup));
    assert!(cycle.changed);
    assert!(cycle.written.changed());
    assert_eq!(reads.load(Ordering::Relaxed), 1);
    assert_eq!(
        harness.document().unwrap().providers.codex.windows["primary"].percent,
        Some(54.0)
    );
    assert_eq!(
        harness.events(),
        vec![Event::Refreshed, Event::SnapshotChanged],
        "every pass is announced; only a pass that moved the numbers is news"
    );
}

#[test]
fn nothing_happens_between_ticks() {
    let harness = Harness::new("refresh-idle");
    let codex = FakeReader::new("codex", reading(54.0));
    let (_, reads, _) = codex.handle();
    let mut engine = harness.engine(ReaderSet::new().with(Box::new(codex)), Some(harness.lock()));

    engine.tick(&harness.clock);
    for _ in 0..10 {
        harness.clock.advance(Duration::from_secs(5));
        let cycle = engine.tick(&harness.clock);
        assert_eq!(cycle.cause, None, "only the tick and a change may refresh");
    }
    assert_eq!(reads.load(Ordering::Relaxed), 1);
}

#[test]
fn the_tick_refreshes_after_a_minute() {
    let harness = Harness::new("refresh-tick");
    let codex = FakeReader::new("codex", reading(54.0));
    let (_, reads, _) = codex.handle();
    let mut engine = harness.engine(ReaderSet::new().with(Box::new(codex)), Some(harness.lock()));

    engine.tick(&harness.clock);
    harness.clock.advance(Duration::from_secs(60));
    let cycle = engine.tick(&harness.clock);

    assert_eq!(cycle.cause, Some(Cause::Tick));
    assert_eq!(reads.load(Ordering::Relaxed), 2);
}

#[test]
fn an_unchanged_document_is_read_again_and_written_once() {
    let harness = Harness::new("refresh-unchanged");
    let codex = FakeReader::new("codex", reading(54.0));
    let (_, reads, _) = codex.handle();
    let mut engine = harness.engine(ReaderSet::new().with(Box::new(codex)), Some(harness.lock()));

    engine.tick(&harness.clock);
    let stamp = harness.document().unwrap().updated_at;

    for _ in 0..5 {
        harness.clock.advance(Duration::from_secs(60));
        let cycle = engine.tick(&harness.clock);
        assert_eq!(cycle.cause, Some(Cause::Tick));
        assert!(!cycle.changed, "the numbers did not move");
        assert!(!cycle.written.changed());
    }

    assert_eq!(reads.load(Ordering::Relaxed), 6, "it kept reading");
    assert_eq!(engine.warnings().writes, 1, "and wrote once");
    assert_eq!(
        harness.document().unwrap().updated_at,
        stamp,
        "updatedAt moves with the content, not with the clock"
    );
    assert_eq!(
        harness
            .events()
            .into_iter()
            .filter(|event| *event == Event::SnapshotChanged)
            .count(),
        1,
        "and told the panel once"
    );
    assert_eq!(
        harness
            .events()
            .into_iter()
            .filter(|event| *event == Event::Refreshed)
            .count(),
        6,
        "but announced every pass, because the notifications look at each reading and          not only at the ones that moved"
    );
}

#[test]
fn a_changed_number_is_written_and_announced() {
    let harness = Harness::new("refresh-changed");
    let codex = FakeReader::new("codex", reading(54.0));
    let (provider, _, _) = codex.handle();
    let mut engine = harness.engine(ReaderSet::new().with(Box::new(codex)), Some(harness.lock()));

    engine.tick(&harness.clock);
    let first_stamp = harness.document().unwrap().updated_at;

    *provider.lock().unwrap() = reading(70.0);
    harness.clock.advance(Duration::from_secs(60));
    let cycle = engine.tick(&harness.clock);

    assert!(cycle.changed);
    let document = harness.document().unwrap();
    assert_eq!(
        document.providers.codex.windows["primary"].percent,
        Some(70.0)
    );
    assert_ne!(document.updated_at, first_stamp);
    assert_eq!(
        harness.events(),
        vec![
            Event::Refreshed,
            Event::SnapshotChanged,
            Event::Refreshed,
            Event::SnapshotChanged
        ]
    );
}

#[test]
fn the_snapshot_in_memory_matches_the_file() {
    let harness = Harness::new("refresh-snapshot");
    let codex = FakeReader::new("codex", reading(54.0));
    let mut engine = harness.engine(ReaderSet::new().with(Box::new(codex)), Some(harness.lock()));
    let snapshot = engine.snapshot();

    engine.tick(&harness.clock);
    let held = snapshot.lock().unwrap().limits().clone();
    assert_eq!(held, harness.document().unwrap());
}

#[test]
fn a_restart_shows_the_last_known_numbers_before_it_reads_anything() {
    let harness = Harness::new("refresh-seeded");
    let codex = FakeReader::new("codex", reading(54.0));
    let mut first = harness.engine(ReaderSet::new().with(Box::new(codex)), Some(harness.lock()));
    first.tick(&harness.clock);
    drop(first);

    // A new process over the same file, before its first tick.
    let second = harness.engine(ReaderSet::new(), None);
    let snapshot = second.snapshot();
    let held = snapshot.lock().unwrap().limits().clone();
    assert_eq!(
        held.providers.codex.windows["primary"].percent,
        Some(54.0),
        "the panel should show the last known numbers rather than a blank"
    );
}

// ---------------------------------------------------------------- triggers

#[test]
fn a_file_change_refreshes_after_the_debounce_and_only_once() {
    let harness = Harness::new("refresh-watch");
    let watched = harness.dir.join("statusline");
    std::fs::create_dir_all(&watched).unwrap();

    let claude = FakeReader::new("claude", reading(12.0)).watching(&watched);
    let (provider, reads, _) = claude.handle();
    let mut engine = harness.engine(
        ReaderSet::new().with(Box::new(claude)),
        Some(harness.lock()),
    );

    engine.tick(&harness.clock); // startup; the watch set learns its target
    harness.clock.advance(Duration::from_secs(5));
    assert_eq!(engine.tick(&harness.clock).cause, None, "the baseline scan");

    // A status line wrote a capture. Three of them, in a burst.
    for index in 0..3 {
        std::fs::write(watched.join(format!("session-{index}.json")), "{}").unwrap();
    }
    *provider.lock().unwrap() = reading(13.0);

    harness.clock.advance(Duration::from_secs(5));
    let cycle = engine.tick(&harness.clock);
    assert_eq!(cycle.cause, None, "still inside the debounce");
    assert_eq!(cycle.sleep_for, schedule::DEBOUNCE);

    harness.clock.advance(schedule::DEBOUNCE);
    assert_eq!(
        engine.tick(&harness.clock).cause,
        Some(Cause::FileChange),
        "and then exactly one refresh for the whole burst"
    );
    assert_eq!(reads.load(Ordering::Relaxed), 2);

    // Nothing more from the same burst.
    harness.clock.advance(Duration::from_secs(5));
    assert_eq!(engine.tick(&harness.clock).cause, None);
}

#[test]
fn an_explicit_request_refreshes() {
    let harness = Harness::new("refresh-requested");
    let codex = FakeReader::new("codex", reading(54.0));
    let mut engine = harness.engine(ReaderSet::new().with(Box::new(codex)), Some(harness.lock()));

    engine.tick(&harness.clock);
    engine.request_refresh(&harness.clock);
    assert_eq!(engine.tick(&harness.clock).cause, None);

    harness.clock.advance(schedule::DEBOUNCE);
    assert_eq!(engine.tick(&harness.clock).cause, Some(Cause::Requested));
}

#[test]
fn waking_from_a_long_sleep_refreshes_at_once() {
    let harness = Harness::new("refresh-wake");
    let codex = FakeReader::new("codex", reading(54.0));
    let (_, reads, _) = codex.handle();
    let mut engine = harness.engine(ReaderSet::new().with(Box::new(codex)), Some(harness.lock()));

    engine.tick(&harness.clock);
    harness.clock.advance(Duration::from_secs(5));
    engine.tick(&harness.clock);

    // Eight hours of wall clock, a hundred milliseconds of monotonic: the machine was
    // suspended. Audit scenario S7 — the retired prototype noticed nothing.
    harness.clock.sleep_through(Duration::from_secs(8 * 3_600));
    let cycle = engine.tick(&harness.clock);

    assert_eq!(cycle.cause, Some(Cause::Woke));
    assert_eq!(
        reads.load(Ordering::Relaxed),
        2,
        "waking up does not wait out a debounce"
    );
}

#[test]
fn a_request_file_asks_for_the_panel_and_a_refresh() {
    let harness = Harness::new("refresh-request-file");
    let request = harness.dir.join("tray.request");
    let codex = FakeReader::new("codex", reading(54.0));
    let mut engine = harness
        .engine(ReaderSet::new().with(Box::new(codex)), Some(harness.lock()))
        .with_request_file(&request);

    engine.tick(&harness.clock);
    place_request(&request, &harness.clock.now()).unwrap();

    harness.clock.advance(Duration::from_secs(5));
    engine.tick(&harness.clock);
    assert!(
        harness.events().contains(&Event::ShowRequested),
        "a second launch asks the running tray to show itself"
    );
    assert!(!request.exists(), "the marker is consumed, not left behind");

    harness.clock.advance(schedule::DEBOUNCE);
    assert_eq!(engine.tick(&harness.clock).cause, Some(Cause::Requested));
}

#[test]
fn a_request_file_left_behind_by_a_crash_is_swept_up_not_obeyed() {
    let harness = Harness::new("refresh-request-stale");
    let request = harness.dir.join("tray.request");
    let codex = FakeReader::new("codex", reading(54.0));
    let mut engine = harness
        .engine(ReaderSet::new().with(Box::new(codex)), Some(harness.lock()))
        .with_request_file(&request);

    engine.tick(&harness.clock);
    place_request(&request, &harness.clock.now()).unwrap();

    // Five minutes later, nobody having noticed.
    harness.clock.advance(Duration::from_secs(300));
    engine.tick(&harness.clock);

    assert!(
        !harness.events().contains(&Event::ShowRequested),
        "a marker older than a minute is litter, not an instruction"
    );
    assert!(!request.exists());
}

// ---------------------------------------------------------------- isolation

#[test]
fn a_reader_that_can_read_nothing_does_not_take_the_other_down_with_it() {
    let harness = Harness::new("refresh-isolation");
    let codex = FakeReader::new("codex", unreadable());
    let claude = FakeReader::new("claude", reading(12.0));
    let (_, codex_reads, _) = codex.handle();
    let (_, claude_reads, _) = claude.handle();

    let readers = ReaderSet::new()
        .with(Box::new(codex))
        .with(Box::new(claude));
    let mut engine = harness.engine(readers, Some(harness.lock()));

    engine.tick(&harness.clock);
    harness.clock.advance(Duration::from_secs(60));
    engine.tick(&harness.clock);

    assert_eq!(codex_reads.load(Ordering::Relaxed), 2);
    assert_eq!(
        claude_reads.load(Ordering::Relaxed),
        2,
        "the loop kept going"
    );

    let document = harness.document().unwrap();
    assert_eq!(
        document.providers.codex.windows["primary"].percent, None,
        "a window that could not be read carries no percentage"
    );
    assert_eq!(
        document.providers.claude.windows["primary"].percent,
        Some(12.0),
        "and the other provider is untouched"
    );
    assert_eq!(document.providers.codex.binding, None);
}

#[test]
fn each_reader_has_its_own_warning_counter() {
    let harness = Harness::new("refresh-warnings");
    let codex = FakeReader::new("codex", reading(54.0));
    let claude = FakeReader::new("claude", reading(12.0));
    let (_, _, codex_warnings) = codex.handle();
    let (_, _, claude_warnings) = claude.handle();

    let readers = ReaderSet::new()
        .with(Box::new(codex))
        .with(Box::new(claude));
    let mut engine = harness.engine(readers, Some(harness.lock()));
    engine.tick(&harness.clock);

    codex_warnings.store(3, Ordering::Relaxed);
    claude_warnings.store(1, Ordering::Relaxed);

    let warnings = engine.warnings();
    assert_eq!(warnings.readers["codex"], 3);
    assert_eq!(warnings.readers["claude"], 1);
    assert_eq!(warnings.refreshes, 1);
    assert_eq!(warnings.writes, 1);
    assert!(!warnings.read_only);
}

#[test]
fn a_provider_this_build_has_no_field_for_still_reaches_the_document() {
    let harness = Harness::new("refresh-third-provider");
    let future = FakeReader::new("gemini", reading(7.0));
    let mut engine = harness.engine(
        ReaderSet::new().with(Box::new(future)),
        Some(harness.lock()),
    );
    engine.tick(&harness.clock);

    let document = harness.document().unwrap();
    assert!(
        document.providers.extra.contains_key("gemini"),
        "a reader for a provider added later lands in the forward-compatible slot"
    );
}

// ---------------------------------------------------------------- one writer

#[test]
fn a_second_instance_reads_and_writes_nothing() {
    let harness = Harness::new("refresh-read-only");
    let held = harness.lock();

    // A second process finds the lock taken and builds a reading engine.
    let taken = LimitsLock::acquire_as(
        &harness.lock_path(),
        99_999,
        "2026-09-07T10:00:05Z",
        &harness.clock.now(),
        STALE_AFTER,
    )
    .unwrap();
    assert!(taken.held().is_none(), "two writers is the thing forbidden");

    let codex = FakeReader::new("codex", reading(54.0));
    let (_, reads, _) = codex.handle();
    let mut engine = harness.engine(ReaderSet::new().with(Box::new(codex)), None);

    assert!(!engine.is_writing());
    for _ in 0..3 {
        engine.tick(&harness.clock);
        harness.clock.advance(Duration::from_secs(60));
    }

    assert_eq!(reads.load(Ordering::Relaxed), 3, "it still reads");
    assert!(
        !harness.limits_path().exists(),
        "and it never writes the file the other process owns"
    );
    assert!(engine.warnings().read_only);
    assert_eq!(engine.warnings().writes, 0);
    drop(held);
}

#[test]
fn the_writer_stops_the_moment_its_lock_is_taken_over() {
    let harness = Harness::new("refresh-lock-lost");
    let codex = FakeReader::new("codex", reading(54.0));
    let (provider, _, _) = codex.handle();
    let mut engine = harness.engine(ReaderSet::new().with(Box::new(codex)), Some(harness.lock()));

    engine.tick(&harness.clock);
    let stamp = harness.document().unwrap().updated_at;

    // Another instance decided this lock was stale and took it.
    let usurper = LockRecord::new(4242, "2026-09-07T10:30:00Z", harness.clock.now());
    std::fs::write(
        harness.lock_path(),
        serde_json::to_string_pretty(&usurper).unwrap(),
    )
    .unwrap();

    *provider.lock().unwrap() = reading(70.0);
    harness.clock.advance(Duration::from_secs(60));
    engine.tick(&harness.clock);

    assert!(!engine.is_writing(), "it noticed on the next heartbeat");
    assert!(engine.warnings().read_only);

    *provider.lock().unwrap() = reading(80.0);
    harness.clock.advance(Duration::from_secs(60));
    engine.tick(&harness.clock);
    let document = harness.document().unwrap();
    assert_eq!(
        document.updated_at, stamp,
        "nothing it read afterwards reached the file"
    );
    assert_eq!(
        document.providers.codex.windows["primary"].percent,
        Some(54.0)
    );
}

#[test]
fn a_write_that_failed_is_tried_again_rather_than_forgotten() {
    let harness = Harness::new("refresh-write-failure");
    let codex = FakeReader::new("codex", reading(54.0));
    let (provider, _, _) = codex.handle();
    let mut engine = harness.engine(ReaderSet::new().with(Box::new(codex)), Some(harness.lock()));

    engine.tick(&harness.clock);
    std::fs::remove_file(harness.limits_path()).unwrap();

    // A directory where the file should be: the write cannot succeed, and must not be
    // recorded as though it had.
    std::fs::create_dir(harness.limits_path()).unwrap();
    *provider.lock().unwrap() = reading(70.0);
    harness.clock.advance(Duration::from_secs(60));
    let cycle = engine.tick(&harness.clock);
    assert!(cycle.changed);
    assert!(!cycle.written.changed());
    assert_eq!(engine.warnings().write_failures, 1);
    assert!(engine.warnings().last_error.is_some());

    // The obstruction goes away; the next tick writes, without anything else having changed.
    std::fs::remove_dir(harness.limits_path()).unwrap();
    harness.clock.advance(Duration::from_secs(60));
    let cycle = engine.tick(&harness.clock);
    assert!(
        cycle.written.changed(),
        "the document is still different from the last one that landed"
    );
    assert_eq!(
        harness.document().unwrap().providers.codex.windows["primary"].percent,
        Some(70.0)
    );
}

// ---------------------------------------------------------------- the thread

#[test]
fn the_loop_runs_on_its_own_thread_and_gives_the_engine_back() {
    let harness = Harness::new("refresh-thread");
    let codex = FakeReader::new("codex", reading(54.0));
    let (_, reads, _) = codex.handle();

    // The real clock and a short schedule: this is the one test that actually waits, and
    // it waits for milliseconds.
    let engine = harness
        .engine(ReaderSet::new().with(Box::new(codex)), Some(harness.lock()))
        .with_schedule(Schedule::new(
            Duration::from_millis(30),
            Duration::from_millis(10),
            Duration::from_millis(1),
            Duration::from_secs(120),
        ));

    let handle = spawn(engine, Arc::new(crate::clock::SystemClock)).unwrap();
    assert!(handle.refresh_now());

    // Wait for the file rather than for a duration.
    let mut waited = 0;
    while harness.document().is_none() && waited < 200 {
        std::thread::sleep(Duration::from_millis(10));
        waited += 1;
    }

    let engine = handle.stop().expect("the loop thread must not panic");
    assert!(reads.load(Ordering::Relaxed) >= 1);
    assert!(engine.warnings().refreshes >= 1);
    assert_eq!(
        harness.document().unwrap().providers.codex.windows["primary"].percent,
        Some(54.0)
    );
}

#[test]
fn the_readers_this_build_ships_can_be_discovered() {
    // Nothing is read here: `discover` only resolves paths. It is checked because a machine
    // with no home directory is a real environment and must not panic.
    let readers = ReaderSet::discover(&crate::config::Config::default());
    assert!(readers.len() <= 2);
    for (key, count) in readers.warnings() {
        assert!(["claude", "codex"].contains(&key.as_str()), "got {key}");
        assert_eq!(count, 0);
    }
}

#[test]
fn changing_the_readers_replaces_them_and_refreshes_at_once() {
    let harness = Harness::new("refresh-reconfigure");
    let codex = FakeReader::new("codex", reading(54.0));
    let (_, codex_reads, _) = codex.handle();
    let mut engine = harness.engine(ReaderSet::new().with(Box::new(codex)), Some(harness.lock()));

    engine.tick(&harness.clock);
    assert_eq!(codex_reads.load(Ordering::Relaxed), 1);

    // The user switches Codex off and Claude on. The old reader has to be *gone*, not
    // merely ignored: that is what "not read" means in the settings.
    let claude = FakeReader::new("claude", reading(31.0)).watching(harness.dir.join("captures"));
    let (_, claude_reads, _) = claude.handle();
    engine.apply(
        ReaderSet::new().with(Box::new(claude)),
        crate::state::Rules::default(),
        &harness.clock,
    );

    harness.clock.advance(Duration::from_millis(300));
    let cycle = engine.tick(&harness.clock);
    assert_eq!(
        cycle.cause,
        Some(Cause::Requested),
        "a settings change refreshes now, not at the next minute"
    );
    assert_eq!(
        codex_reads.load(Ordering::Relaxed),
        1,
        "the reader that was switched off is not called again"
    );
    assert_eq!(claude_reads.load(Ordering::Relaxed), 1);

    let document = harness.document().unwrap();
    assert!(
        !document.providers.codex.configured,
        "and its block goes back to `configured: false` rather than keeping stale numbers"
    );
    assert_eq!(
        document.providers.claude.windows["primary"].percent,
        Some(31.0)
    );
    assert_eq!(
        engine.watched(),
        [harness.dir.join("captures")],
        "the watch list follows the readers: nobody lists a directory nobody reads"
    );
}

#[test]
fn a_provider_switched_off_in_the_settings_has_no_reader_at_all() {
    use crate::config::{Config, ProviderSwitches};

    let both_off = ReaderSet::discover(&Config {
        providers: ProviderSwitches {
            claude: false,
            codex: false,
        },
        ..Config::default()
    });
    assert_eq!(
        both_off.len(),
        0,
        "off must mean the reader is never built, not that its answer is discarded"
    );
    assert!(
        both_off.watched().is_empty(),
        "and nothing is watched either"
    );

    let codex_only = ReaderSet::discover(&Config {
        providers: ProviderSwitches {
            claude: false,
            codex: true,
        },
        ..Config::default()
    });
    assert!(!codex_only.keys().contains(&"claude"));
}
