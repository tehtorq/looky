use std::path::PathBuf;

use iced::Task;

use crate::app::{Looky, Message};
use crate::duplicates;
use crate::thumbnail;

const THUMBNAIL_BATCH_SIZE: usize = 32;
const PREVIEW_BATCH_SIZE: usize = 16;
const MAX_UPGRADE_BATCHES_IN_FLIGHT: usize = 3;
const DUP_HASH_BATCH_SIZE: usize = 32;

pub fn load_next_batch(state: &mut Looky) -> Task<Message> {
    if state.pending_thumbnails.is_empty() {
        state.loading = false;
        return Task::none();
    }

    let count = THUMBNAIL_BATCH_SIZE.min(state.pending_thumbnails.len());
    let batch: Vec<PathBuf> = state.pending_thumbnails.drain(..count).collect();

    Task::perform(
        async move { thumbnail::generate_thumbnails_parallel(&batch, 400) },
        Message::ThumbnailBatchReady,
    )
}

pub fn load_next_preview_batch(state: &mut Looky) -> Task<Message> {
    if state.pending_thumbnails.is_empty() {
        return Task::none();
    }

    let count = PREVIEW_BATCH_SIZE.min(state.pending_thumbnails.len());
    let batch: Vec<PathBuf> = state.pending_thumbnails.drain(..count).collect();

    Task::perform(
        async move { thumbnail::extract_previews_parallel(&batch, 400) },
        Message::PreviewBatchReady,
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
        tasks.push(Task::perform(
            async move { thumbnail::generate_thumbnails_parallel(&batch, 400) },
            Message::ThumbnailUpgradeReady,
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

    Task::perform(
        async move { duplicates::compute_hashes_batch(&batch) },
        Message::DupHashBatchReady,
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
    let path = state.image_paths[idx].clone();
    let (task, handle) = Task::perform(
        async move {
            match open_image_oriented(&path) {
                Some(rgba) => {
                    let (w, h) = rgba.dimensions();
                    Message::ViewerImageLoaded(idx, rgba.into_raw(), w, h)
                }
                None => Message::Tick,
            }
        },
        |msg| msg,
    )
    .abortable();
    state.viewer_preload_handles.push((idx, handle));
    task
}

pub fn preload_viewer_neighbors(state: &mut Looky) -> Task<Message> {
    let Some(idx) = state.viewer.current_index else {
        return Task::none();
    };
    let total = state.image_paths.len();
    let mut tasks = Vec::new();
    let start = idx.saturating_sub(3);
    let end = (idx + 3).min(total.saturating_sub(1));
    for i in start..=end {
        if i != idx && !state.viewer_cache.contains_key(&i) {
            let path = state.image_paths[i].clone();
            let index = i;
            log::debug!("viewer: [{}] loading (neighbor)", i);
            let (task, handle) = Task::perform(
                async move {
                    match open_image_oriented(&path) {
                        Some(rgba) => {
                            let (w, h) = rgba.dimensions();
                            Message::ViewerImageLoaded(index, rgba.into_raw(), w, h)
                        }
                        None => Message::Tick,
                    }
                },
                |msg| msg,
            )
            .abortable();
            state.viewer_preload_handles.push((i, handle));
            tasks.push(task);
        }
    }
    Task::batch(tasks)
}


fn open_image_oriented(path: &std::path::Path) -> Option<::image::RgbaImage> {
    let img = ::image::open(path).ok().or_else(|| crate::heic_decode::open_heic(path))?;
    let orientation = thumbnail::read_orientation(path);
    let oriented = match orientation {
        2 => img.fliph(),
        3 => img.rotate180(),
        4 => img.flipv(),
        5 => img.rotate90().fliph(),
        6 => img.rotate90(),
        7 => img.rotate270().fliph(),
        8 => img.rotate270(),
        _ => img,
    };
    Some(oriented.to_rgba8())
}

