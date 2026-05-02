//! Per-project filesystem watcher that emits `DirectiveChanged` when
//! `.galvolite/directive.md` is modified out-of-band.
//!
//! The HTTP `PUT /api/projects/:id/directive` route publishes the same
//! event directly — this watcher exists for raw editor saves, `git pull`,
//! and any other writer that doesn't go through our API.

use crate::api::events::{DomainEvent, EventBroadcaster};
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};
use uuid::Uuid;

/// Owns a `notify` watcher and the directory it monitors. Drop stops the
/// watcher (notify's own Drop tears down the OS-level handle).
pub struct WatcherHandle {
    _watcher: RecommendedWatcher,
    /// Kept for diagnostics / tests; not currently read in production code.
    pub watched_dir: PathBuf,
}

/// Spawn a watcher for `<project_path>/.galvolite/`. The directory is
/// created if it does not yet exist (notify cannot watch a non-existent
/// path on most platforms). Returns the handle the caller must keep alive.
///
/// When `directive.md` is created or modified inside that dir, publishes
/// `DomainEvent::DirectiveChanged { project_id }` on the bus, debounced
/// to one frame per 500 ms (FS writes commonly fire 2-3 events per save).
pub fn spawn(
    project_id: Uuid,
    project_path: &Path,
    bus: EventBroadcaster,
) -> notify::Result<WatcherHandle> {
    let galvolite_dir = project_path.join(".galvolite");
    // Ensure the dir exists so notify has something to watch. Ignored on
    // race with another creator.
    let _ = std::fs::create_dir_all(&galvolite_dir);

    // Single-slot debounce: if last fire was <500 ms ago, skip.
    // std::sync::Mutex is fine here because the critical section is sync
    // and we never hold it across .await.
    let last_fire: Arc<std::sync::Mutex<Option<Instant>>> =
        Arc::new(std::sync::Mutex::new(None));
    let target = galvolite_dir.join("directive.md");
    let bus_for_cb = bus.clone();
    let last_fire_cb = last_fire.clone();

    let mut watcher = RecommendedWatcher::new(
        move |res: notify::Result<notify::Event>| {
            let event = match res {
                Ok(ev) => ev,
                Err(e) => {
                    tracing::warn!(error = %e, "directive watcher error");
                    return;
                }
            };
            // Only react to creates / writes / data-modifies. Notify's
            // EventKind::Modify variant covers most editor saves; Create
            // covers `mv directive.md.tmp directive.md` (atomic-rename
            // pattern, which `write_atomic` itself uses).
            use notify::EventKind;
            let relevant = matches!(
                event.kind,
                EventKind::Create(_) | EventKind::Modify(_) | EventKind::Other
            );
            if !relevant {
                return;
            }
            let hits_target = event.paths.iter().any(|p| p == &target);
            if !hits_target {
                return;
            }
            // Debounce: 500 ms since last published frame for THIS project.
            let now = Instant::now();
            {
                let mut last = last_fire_cb.lock().unwrap();
                if let Some(prev) = *last {
                    if now.duration_since(prev) < Duration::from_millis(500) {
                        return;
                    }
                }
                *last = Some(now);
            }
            bus_for_cb.publish(DomainEvent::DirectiveChanged { project_id });
        },
        notify::Config::default(),
    )?;
    watcher.watch(&galvolite_dir, RecursiveMode::NonRecursive)?;

    Ok(WatcherHandle {
        _watcher: watcher,
        watched_dir: galvolite_dir,
    })
}
