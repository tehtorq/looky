use std::path::PathBuf;

use iced::Task;

use crate::app::{Looky, Message};
use crate::duplicates;
use crate::thumbnail;

const THUMBNAIL_BATCH_SIZE: usize = 32;
const PREVIEW_BATCH_SIZE: usize = 16;
const MAX_UPGRADE_BATCHES_IN_FLIGHT: usize = 3;
const DUP_HASH_BATCH_SIZE: usize = 16;
/// How many neighbors to keep decoded at full resolution on each side of the
/// current photo. Each entry is a full-res RGBA frame, so keep this small.
pub const PRELOAD_RADIUS: usize = 1;

/// Run a blocking/CPU-heavy closure on its own thread instead of stalling the
/// iced executor, which also drives every other in-flight Task.
pub fn run_blocking<T: Send + 'static>(
    f: impl FnOnce() -> T + Send + 'static,
) -> impl Future<Output = T> + Send + 'static {
    let (tx, rx) = iced::futures::channel::oneshot::channel();
    std::thread::spawn(move || {
        let _ = tx.send(f());
    });
    async move { rx.await.expect("blocking task panicked") }
}

pub fn load_next_preview_batch(state: &mut Looky) -> Task<Message> {
    if state.pending_thumbnails.is_empty() {
        return Task::none();
    }

    let count = PREVIEW_BATCH_SIZE.min(state.pending_thumbnails.len());
    let batch: Vec<PathBuf> = state.pending_thumbnails.drain(..count).collect();
    let generation = state.scan_generation;

    Task::perform(
        run_blocking(move || thumbnail::extract_previews_parallel(&batch, 400)),
        move |results| Message::PreviewBatchReady(generation, results),
    )
}

pub fn load_upgrade_batches(state: &mut Looky) -> Task<Message> {
    let mut tasks = Vec::new();
    while state.upgrade_batches_in_flight < MAX_UPGRADE_BATCHES_IN_FLIGHT
        && !state.pending_upgrades.is_empty()
    {
        let count = THUMBNAIL_BATCH_SIZE.min(state.pending_upgrades.len());
        let batch: Vec<PathBuf> = state.pending_upgrades.drain(..count).collect();
        state.upgrade_batches_in_flight += 1;
        let generation = state.scan_generation;
        tasks.push(Task::perform(
            run_blocking(move || thumbnail::generate_thumbnails_parallel(&batch, 400)),
            move |results| Message::ThumbnailUpgradeReady(generation, results),
        ));
    }
    Task::batch(tasks)
}

pub fn load_next_dup_batch(state: &mut Looky) -> Task<Message> {
    if state.dup_pending.is_empty() {
        return Task::none();
    }

    let count = DUP_HASH_BATCH_SIZE.min(state.dup_pending.len());
    let batch: Vec<(usize, PathBuf)> = state.dup_pending.drain(..count).collect();
    let generation = state.dup_generation;

    Task::perform(
        run_blocking(move || duplicates::compute_hashes_batch(&batch)),
        move |results| Message::DupHashBatchReady(generation, results),
    )
}

pub fn preload_viewer_images(state: &mut Looky) -> Task<Message> {
    // Abort all in-flight preloads — the user navigated, old work is stale
    for (idx, handle) in state.viewer_preload_handles.drain(..) {
        log::debug!("viewer: [{}] aborted", idx);
        handle.abort();
    }

    let Some(idx) = state.viewer.current_index else {
        return Task::none();
    };

    if state.viewer_cache.contains_key(&idx) {
        log::debug!("viewer: [{}] already cached, loading neighbors", idx);
        return preload_viewer_neighbors(state);
    }
    log::debug!("viewer: [{}] loading (current)", idx);
    spawn_viewer_load(state, idx)
}

pub fn preload_viewer_neighbors(state: &mut Looky) -> Task<Message> {
    let Some(idx) = state.viewer.current_index else {
        return Task::none();
    };
    let total = state.image_paths.len();
    let mut tasks = Vec::new();
    let start = idx.saturating_sub(PRELOAD_RADIUS);
    let end = (idx + PRELOAD_RADIUS).min(total.saturating_sub(1));
    for i in start..=end {
        if i != idx && !state.viewer_cache.contains_key(&i) {
            log::debug!("viewer: [{}] loading (neighbor)", i);
            tasks.push(spawn_viewer_load(state, i));
        }
    }
    Task::batch(tasks)
}

fn spawn_viewer_load(state: &mut Looky, index: usize) -> Task<Message> {
    let Some(path) = state.image_paths.get(index).cloned() else {
        return Task::none();
    };
    let generation = state.scan_generation;
    let (task, handle) = Task::perform(
        run_blocking(move || match open_image_oriented(&path) {
            Some(rgba) => {
                let (w, h) = rgba.dimensions();
                Message::ViewerImageLoaded(generation, index, rgba.into_raw(), w, h)
            }
            None => Message::ViewerImageFailed(index),
        }),
        |msg| msg,
    )
    .abortable();
    state.viewer_preload_handles.push((index, handle));
    task
}

fn open_image_oriented(path: &std::path::Path) -> Option<::image::RgbaImage> {
    let img = ::image::open(path).ok().or_else(|| crate::heic_decode::open_heic(path))?;
    let oriented = thumbnail::apply_orientation(img, thumbnail::read_orientation(path));
    Some(oriented.to_rgba8())
}
