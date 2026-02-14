use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use iced::widget::{button, column, container, image, row, rule, scrollable, text, Space};
use iced::{Color, Element, Length, Subscription, Task, Theme};

use crate::catalog::{self, Catalog};
use crate::duplicates::{self, DuplicateGroup, ImageHashes, MatchKind};
use crate::key_listener::KeyListener;
use crate::metadata::{self, PhotoMetadata};
use crate::server;
use crate::thumbnail;
use crate::viewer::ViewerState;

const THUMBNAIL_BATCH_SIZE: usize = 32;
const PREVIEW_BATCH_SIZE: usize = 16;
const MAX_UPGRADE_BATCHES_IN_FLIGHT: usize = 3;
const DUP_HASH_BATCH_SIZE: usize = 32;
const VISUAL_DUP_THRESHOLD: u32 = 10;
const THUMB_FADE_MS: f32 = 300.0;

fn boot() -> (Looky, Task<Message>) {
    let mut state = Looky::default();

    // Open the catalog database
    if let Some(dir) = config_dir() {
        let db_path = dir.join("catalog.db");
        match Catalog::open(&db_path) {
            Ok(cat) => state.catalog = Some(cat),
            Err(e) => log::warn!("Failed to open catalog DB: {}", e),
        }
    }

    if let Some(folder) = load_last_folder() {
        state.folder = Some(folder.clone());
        state.loading = true;
        let task = Task::perform(scan_folder(folder), Message::ImagesFound);
        return (state, task);
    }
    (state, Task::none())
}

pub fn run() -> iced::Result {
    iced::application(boot, update, view)
        .title("Looky")
        .theme(theme)
        .subscription(subscription)
        .centered()
        .run()
}

struct Looky {
    folder: Option<PathBuf>,
    image_paths: Vec<PathBuf>,
    thumbnails: Vec<(PathBuf, image::Handle, Instant)>,
    pending_thumbnails: Vec<PathBuf>,
    // Two-pass loading: path → index in thumbnails vec for O(1) upgrade
    thumbnail_index: HashMap<PathBuf, usize>,
    pending_upgrades: Vec<PathBuf>,
    upgrade_batches_in_flight: usize,
    viewer: ViewerState,
    loading: bool,
    cached_metadata: Option<(usize, PhotoMetadata)>,
    catalog: Option<Catalog>,
    // Duplicate detection state
    dup_hashes: Vec<(usize, ImageHashes)>,
    dup_pending: Vec<(usize, PathBuf)>,
    dup_scanning: bool,
    dup_total: usize,
    dup_groups: Vec<DuplicateGroup>,
    dup_badge_set: HashSet<usize>,
    dup_view_active: bool,
    dup_compare: Option<usize>,
    dup_summaries: HashMap<usize, metadata::FileSummary>,
    grid_scroll_y: f32,
    dup_scroll_y: f32,
    grid_columns: usize,
    viewport_width: f32,
    viewport_height: f32,
    selected_thumb: Option<usize>,
    viewer_cache: HashMap<usize, image::Handle>,
    viewer_dimensions: HashMap<usize, (u32, u32)>,
    viewer_preload_handles: Vec<(usize, iced::task::Handle)>,
    fullscreen: bool,
    // Screensaver mode
    screensaver_active: bool,
    screensaver_order: Vec<usize>,
    screensaver_position: usize,
    was_fullscreen: bool,
    // Sharing server
    server_handle: Option<server::ServerHandle>,
    server_url: Option<String>,
    qr_handle: Option<image::Handle>,
}

impl Default for Looky {
    fn default() -> Self {
        Self {
            folder: None,
            image_paths: Vec::new(),
            thumbnails: Vec::new(),
            pending_thumbnails: Vec::new(),
            thumbnail_index: HashMap::new(),
            pending_upgrades: Vec::new(),
            upgrade_batches_in_flight: 0,
            viewer: ViewerState::default(),
            loading: false,
            cached_metadata: None,
            catalog: None,
            dup_hashes: Vec::new(),
            dup_pending: Vec::new(),
            dup_scanning: false,
            dup_total: 0,
            dup_groups: Vec::new(),
            dup_badge_set: HashSet::new(),
            dup_view_active: false,
            dup_compare: None,
            dup_summaries: HashMap::new(),
            grid_scroll_y: 0.0,
            dup_scroll_y: 0.0,
            grid_columns: 4,
            viewport_width: 800.0,
            viewport_height: 600.0,
            selected_thumb: None,
            viewer_cache: HashMap::new(),
            viewer_dimensions: HashMap::new(),
            viewer_preload_handles: Vec::new(),
            fullscreen: false,
            screensaver_active: false,
            screensaver_order: Vec::new(),
            screensaver_position: 0,
            was_fullscreen: false,
            server_handle: None,
            server_url: None,
            qr_handle: None,
        }
    }
}

#[derive(Debug, Clone)]
pub enum Message {
    OpenFolder,
    FolderSelected(Option<PathBuf>),
    ImagesFound(Vec<PathBuf>),
    ThumbnailBatchReady(Vec<(PathBuf, Vec<u8>, u32, u32)>),
    PreviewBatchReady(Vec<(PathBuf, Option<(Vec<u8>, u32, u32)>)>),
    ThumbnailUpgradeReady(Vec<(PathBuf, Vec<u8>, u32, u32)>),
    ViewImage(usize),
    NextImage,
    PrevImage,
    BackToGrid,
    ToggleInfo,
    ViewerImageLoaded(usize, Vec<u8>, u32, u32),
    Tick,
    // Duplicate detection messages
    FindDuplicates,
    CancelDupScan,
    DupHashBatchReady(Vec<(usize, Option<ImageHashes>)>),
    DupAnalysisReady(Vec<DuplicateGroup>, HashMap<usize, metadata::FileSummary>),
    CachedDupAnalysisReady(Vec<DuplicateGroup>, HashMap<usize, metadata::FileSummary>),
    ShowDuplicatesView,
    BackFromDuplicates,
    CompareDuplicates(usize),
    BackFromCompare,
    // Zoom
    ToggleZoom,
    CenterZoomScroll,
    ZoomAdjust(f32, f32, f32),
    ZoomScrolled(f32, f32),
    ViewerDrag(f32, f32),
    DragScroll(f32, f32),
    DupListScrolled(f32),
    ViewerClickZoom(f32, f32),
    ViewerClickUnzoom(f32, f32),
    PinchZoom(f32, f32, f32),
    // Screensaver
    ToggleScreensaver,
    ScreensaverAdvance,
    // Sharing
    ToggleSharing,
    // Navigation
    GridScrolled(f32),
    WindowResized(f32, f32),
    KeyEscape,
    KeyLeft,
    KeyRight,
    KeyUp,
    KeyDown,
    KeyEnter,
    ToggleFullscreen,
}

fn subscription(state: &Looky) -> Subscription<Message> {
    // Window resize still goes through subscription (not latency-sensitive).
    // Keyboard events are handled by KeyListener widget for instant response.
    let events = iced::event::listen_with(|event, _status, _window| match event {
        iced::Event::Window(iced::window::Event::Resized(size)) => {
            Some(Message::WindowResized(size.width, size.height))
        }
        _ => None,
    });

    let needs_tick = state.viewer.is_transitioning()
        || state.viewer.is_zoom_animating()
        || thumbnails_fading(state);

    let mut subs = vec![events];
    if needs_tick {
        subs.push(iced::time::every(Duration::from_millis(16)).map(|_| Message::Tick));
    }
    if state.screensaver_active {
        subs.push(
            iced::time::every(Duration::from_secs(10)).map(|_| Message::ScreensaverAdvance),
        );
    }
    Subscription::batch(subs)
}

fn thumbnails_fading(state: &Looky) -> bool {
    state
        .thumbnails
        .last()
        .is_some_and(|(_, _, added)| added.elapsed().as_secs_f32() * 1000.0 < THUMB_FADE_MS)
}

fn update(state: &mut Looky, message: Message) -> Task<Message> {
    match message {
        Message::OpenFolder => {
            return Task::perform(pick_folder(), Message::FolderSelected);
        }
        Message::FolderSelected(Some(path)) => {
            save_last_folder(&path);
            // Stop sharing server on folder change
            if let Some(handle) = state.server_handle.take() {
                std::thread::spawn(move || handle.stop());
            }
            state.server_url = None;
            state.qr_handle = None;
            state.folder = Some(path.clone());
            state.thumbnails.clear();
            state.image_paths.clear();
            state.pending_thumbnails.clear();
            state.thumbnail_index.clear();
            state.pending_upgrades.clear();
            state.upgrade_batches_in_flight = 0;
            state.viewer = ViewerState::default();
            state.loading = true;
            // Reset dup state on folder change
            state.dup_hashes.clear();
            state.dup_pending.clear();
            state.dup_scanning = false;
            state.dup_groups.clear();
            state.dup_badge_set.clear();
            state.dup_view_active = false;
            state.dup_compare = None;
            state.dup_summaries.clear();
            return Task::perform(scan_folder(path), Message::ImagesFound);
        }
        Message::FolderSelected(None) => {}
        Message::ImagesFound(paths) => {
            if let Some(cat) = state.catalog.as_ref() {
                cat.prune_missing();
            }
            state.image_paths = paths.clone();
            state.pending_thumbnails = paths;

            // Auto-load cached duplicate groups from catalog
            if let Some(cat) = state.catalog.as_ref() {
                let mut cached_hashes = Vec::new();
                for (i, path) in state.image_paths.iter().enumerate() {
                    if let Some((ch, ph)) = cat.get_hashes(path) {
                        cached_hashes.push((
                            i,
                            ImageHashes {
                                content_hash: ch,
                                perceptual_hash: ph,
                            },
                        ));
                    }
                }
                if cached_hashes.len() >= 2 {
                    let image_paths = state.image_paths.clone();
                    let mut cached_summaries: HashMap<usize, metadata::FileSummary> =
                        HashMap::new();
                    for (i, path) in image_paths.iter().enumerate() {
                        if let Some(s) = cat.get_file_summary(path) {
                            cached_summaries.insert(i, s);
                        }
                    }
                    state.dup_hashes = cached_hashes.clone();
                    let task = Task::perform(
                        async move {
                            let groups = duplicates::find_duplicates(
                                &cached_hashes,
                                VISUAL_DUP_THRESHOLD,
                            );
                            let dup_indices = duplicates::duplicate_indices(&groups);
                            let summaries: HashMap<usize, metadata::FileSummary> = dup_indices
                                .iter()
                                .filter_map(|&idx| {
                                    if let Some(cached) = cached_summaries.get(&idx) {
                                        return Some((idx, cached.clone()));
                                    }
                                    let path = image_paths.get(idx)?;
                                    Some((idx, metadata::read_file_summary(path)))
                                })
                                .collect();
                            (groups, summaries)
                        },
                        |(g, s)| Message::CachedDupAnalysisReady(g, s),
                    );
                    return Task::batch([load_next_preview_batch(state), task]);
                }
            }
            return load_next_preview_batch(state);
        }
        Message::ThumbnailBatchReady(results) => {
            let now = Instant::now();
            for (path, rgba, width, height) in results {
                let handle = image::Handle::from_rgba(width, height, rgba);
                state.thumbnails.push((path, handle, now));
            }
            return load_next_batch(state);
        }
        Message::PreviewBatchReady(results) => {
            let now = Instant::now();
            for (path, maybe_preview) in results {
                let idx = state.thumbnails.len();
                state.thumbnail_index.insert(path.clone(), idx);
                let handle = if let Some((rgba, w, h)) = maybe_preview {
                    image::Handle::from_rgba(w, h, rgba)
                } else {
                    // Placeholder — will be replaced by upgrade batch
                    image::Handle::from_rgba(1, 1, vec![60, 60, 60, 255])
                };
                state.thumbnails.push((path.clone(), handle, now));
                state.pending_upgrades.push(path);
            }
            // Continue loading previews AND fire upgrade batches
            let preview_task = load_next_preview_batch(state);
            let upgrade_task = load_upgrade_batches(state);
            return Task::batch([preview_task, upgrade_task]);
        }
        Message::ThumbnailUpgradeReady(results) => {
            state.upgrade_batches_in_flight =
                state.upgrade_batches_in_flight.saturating_sub(1);
            let now = Instant::now();
            for (path, rgba, width, height) in results {
                let handle = image::Handle::from_rgba(width, height, rgba);
                if let Some(&idx) = state.thumbnail_index.get(&path) {
                    if idx < state.thumbnails.len() {
                        state.thumbnails[idx] = (path, handle, now);
                    }
                }
            }
            if state.pending_upgrades.is_empty()
                && state.upgrade_batches_in_flight == 0
                && state.pending_thumbnails.is_empty()
            {
                state.loading = false;
            }
            return load_upgrade_batches(state);
        }
        Message::ViewImage(index) => {
            state.selected_thumb = Some(index);
            state.viewer.open_index(index);
            refresh_metadata(state);
            return preload_viewer_images(state);
        }
        Message::NextImage => {
            state.viewer.next(state.image_paths.len());
            state.selected_thumb = state.viewer.current_index;
            refresh_metadata(state);
            return preload_viewer_images(state);
        }
        Message::PrevImage => {
            state.viewer.prev();
            state.selected_thumb = state.viewer.current_index;
            refresh_metadata(state);
            return preload_viewer_images(state);
        }
        Message::BackToGrid => {
            state.viewer.close();
            state.cached_metadata = None;
            state.viewer_cache.clear();
            state.viewer_dimensions.clear();
            return restore_grid_scroll(state);
        }
        Message::ToggleInfo => {
            state.viewer.toggle_info();
        }
        Message::ViewerImageLoaded(index, rgba, width, height) => {
            log::debug!("viewer: [{}] loaded ({}x{})", index, width, height);
            let handle = image::Handle::from_rgba(width, height, rgba);
            state.viewer_cache.insert(index, handle);
            state.viewer_dimensions.insert(index, (width, height));
            // Evict distant entries to limit memory (keep ±3 of current)
            if let Some(current) = state.viewer.current_index {
                let keep_min = current.saturating_sub(3);
                let keep_max = current + 3;
                // During screensaver, also keep the next image (random order, not a neighbor)
                let ss_next = if state.screensaver_active {
                    state.screensaver_order.get(state.screensaver_position + 1).copied()
                } else {
                    None
                };
                state
                    .viewer_cache
                    .retain(|&k, _| (k >= keep_min && k <= keep_max) || ss_next == Some(k));
                state
                    .viewer_dimensions
                    .retain(|&k, _| (k >= keep_min && k <= keep_max) || ss_next == Some(k));
                // Current image just arrived — now preload neighbors
                if index == current {
                    return preload_viewer_neighbors(state);
                }
            }
        }
        Message::Tick => {
            state.viewer.tick();
            let old_zoom = state.viewer.zoom_level;
            let crossed_threshold = state.viewer.tick_zoom();
            let new_zoom = state.viewer.zoom_level;
            if crossed_threshold {
                return Task::done(Message::CenterZoomScroll);
            } else if state.viewer.is_zoomed() && (new_zoom - old_zoom).abs() > 0.001 {
                return anchor_zoom_scroll(state, old_zoom, new_zoom);
            }
        }
        // Duplicate detection
        Message::FindDuplicates => {
            state.dup_hashes.clear();
            state.dup_groups.clear();
            state.dup_badge_set.clear();
            state.dup_summaries.clear();
            state.dup_scanning = true;
            state.dup_compare = None;
            state.dup_view_active = false;
            state.dup_total = state.image_paths.len();

            // Check catalog for cached hashes; only queue uncached/stale files
            let mut pending = Vec::new();
            for (i, path) in state.image_paths.iter().enumerate() {
                if let Some((content_hash, perceptual_hash)) =
                    state.catalog.as_ref().and_then(|c| c.get_hashes(path))
                {
                    state.dup_hashes.push((
                        i,
                        ImageHashes {
                            content_hash,
                            perceptual_hash,
                        },
                    ));
                } else {
                    pending.push((i, path.clone()));
                }
            }
            state.dup_pending = pending;
            return load_next_dup_batch(state);
        }
        Message::CancelDupScan => {
            state.dup_pending.clear();
            state.dup_scanning = false;
            state.dup_hashes.clear();
            state.dup_total = 0;
        }
        Message::DupHashBatchReady(results) => {
            if !state.dup_scanning {
                // Scan was cancelled — discard late-arriving batch
                return Task::none();
            }
            for (idx, maybe_hash) in results {
                if let Some(h) = maybe_hash {
                    // Persist to catalog
                    if let (Some(cat), Some(path)) =
                        (state.catalog.as_ref(), state.image_paths.get(idx))
                    {
                        if let Some((file_size, mtime_ns)) =
                            catalog::file_size_and_mtime_for(path)
                        {
                            cat.insert_hashes(
                                path,
                                file_size,
                                mtime_ns,
                                &h.content_hash,
                                &h.perceptual_hash,
                            );
                        }
                    }
                    state.dup_hashes.push((idx, h));
                }
            }
            if state.dup_pending.is_empty() {
                // All hashes computed — run analysis off the main thread
                let hashes = state.dup_hashes.clone();
                let image_paths = state.image_paths.clone();

                // Pre-collect cached summaries from the catalog (on main thread)
                let mut cached_summaries: HashMap<usize, metadata::FileSummary> = HashMap::new();
                if let Some(cat) = state.catalog.as_ref() {
                    // We don't know dup_indices yet, but we can pre-cache all image paths
                    // to avoid disk reads in the async block. This is fast (just DB lookups).
                    for (i, path) in image_paths.iter().enumerate() {
                        if let Some(summary) = cat.get_file_summary(path) {
                            cached_summaries.insert(i, summary);
                        }
                    }
                }

                return Task::perform(
                    async move {
                        let groups =
                            duplicates::find_duplicates(&hashes, VISUAL_DUP_THRESHOLD);
                        let dup_indices = duplicates::duplicate_indices(&groups);
                        let summaries: HashMap<usize, metadata::FileSummary> = dup_indices
                            .iter()
                            .filter_map(|&idx| {
                                if let Some(cached) = cached_summaries.get(&idx) {
                                    return Some((idx, cached.clone()));
                                }
                                let path = image_paths.get(idx)?;
                                Some((idx, metadata::read_file_summary(path)))
                            })
                            .collect();
                        (groups, summaries)
                    },
                    |(groups, summaries)| Message::DupAnalysisReady(groups, summaries),
                );
            } else {
                return load_next_dup_batch(state);
            }
        }
        Message::DupAnalysisReady(groups, summaries) => {
            state.dup_scanning = false;
            state.dup_badge_set = duplicates::duplicate_indices(&groups);
            state.dup_groups = groups;

            // Persist newly computed summaries to catalog
            if let Some(cat) = state.catalog.as_ref() {
                for (idx, summary) in &summaries {
                    if let Some(path) = state.image_paths.get(*idx) {
                        if let Some((file_size, mtime_ns)) =
                            catalog::file_size_and_mtime_for(path)
                        {
                            cat.insert_file_summary(path, file_size, mtime_ns, summary);
                        }
                    }
                }
            }
            state.dup_summaries = summaries;
        }
        Message::CachedDupAnalysisReady(groups, summaries) => {
            // Only apply if we're not currently in a full scan
            if !state.dup_scanning {
                state.dup_badge_set = duplicates::duplicate_indices(&groups);
                state.dup_groups = groups;
                if let Some(cat) = state.catalog.as_ref() {
                    for (idx, summary) in &summaries {
                        if let Some(path) = state.image_paths.get(*idx) {
                            if let Some((fs, mt)) = catalog::file_size_and_mtime_for(path) {
                                cat.insert_file_summary(path, fs, mt, summary);
                            }
                        }
                    }
                }
                state.dup_summaries = summaries;
            }
        }
        Message::ShowDuplicatesView => {
            state.dup_view_active = true;
            state.dup_compare = None;
            state.dup_scroll_y = 0.0;
        }
        Message::BackFromDuplicates => {
            state.dup_view_active = false;
        }
        Message::CompareDuplicates(group_idx) => {
            state.dup_compare = Some(group_idx);
        }
        Message::BackFromCompare => {
            state.dup_compare = None;
        }
        // Zoom
        Message::ToggleZoom => {
            if let Some(idx) = state.viewer.current_index {
                if !state.viewer_cache.contains_key(&idx) {
                    return Task::none();
                }
                state.viewer.toggle_zoom();
            } else if let Some(idx) = state.selected_thumb {
                // In grid: open selected image (current Space behavior)
                if !state.dup_view_active
                    && state.dup_compare.is_none()
                    && idx < state.thumbnails.len()
                {
                    state.viewer.open_index(idx);
                    refresh_metadata(state);
                    return preload_viewer_images(state);
                }
            }
        }
        Message::CenterZoomScroll => {
            return center_zoom_scroll(state);
        }
        Message::ZoomAdjust(delta, cursor_x, cursor_y) => {
            if let Some(idx) = state.viewer.current_index {
                // Don't zoom until the full-res image is loaded — zooming the
                // thumbnail gives wrong dimensions and stretches badly.
                if !state.viewer_cache.contains_key(&idx) {
                    return Task::none();
                }
                state.viewer.zoom_anchor = Some((cursor_x, cursor_y));
                let old_zoom = state.viewer.zoom_level;
                state.viewer.adjust_zoom(delta);
                // Snap zoom_level to target immediately — no residual
                // animation after scrolling stops.
                state.viewer.zoom_level = state.viewer.zoom_target;
                let new_zoom = state.viewer.zoom_level;
                if state.viewer.is_zoomed() && (new_zoom - old_zoom).abs() > 0.001 {
                    return anchor_zoom_scroll(state, old_zoom, new_zoom);
                }
            }
        }
        Message::ZoomScrolled(x, y) => {
            state.viewer.zoom_offset = (x, y);
        }
        Message::ViewerDrag(dx, dy) => {
            if state.viewer.is_zoomed() {
                return pan_zoom(state, -dx, -dy);
            }
        }
        Message::DragScroll(_dx, dy) => {
            let (scroll_id, scroll_y) = if state.dup_view_active {
                (dup_list_scroll_id(), &mut state.dup_scroll_y)
            } else {
                (grid_scroll_id(), &mut state.grid_scroll_y)
            };
            let new_y = (*scroll_y - dy).max(0.0);
            *scroll_y = new_y;
            use iced::widget::operation::AbsoluteOffset;
            return iced::widget::operation::scroll_to(
                scroll_id,
                AbsoluteOffset { x: None, y: Some(new_y) },
            );
        }
        Message::DupListScrolled(y) => {
            state.dup_scroll_y = y;
        }
        Message::ViewerClickZoom(cx, cy) => {
            if let Some(idx) = state.viewer.current_index {
                if state.viewer_cache.contains_key(&idx) {
                    state.viewer.zoom_anchor = Some((cx, cy));
                    let old_zoom = state.viewer.zoom_level;
                    state.viewer.adjust_zoom(4.0);
                    let _crossed = state.viewer.tick_zoom();
                    let new_zoom = state.viewer.zoom_level;
                    if state.viewer.is_zoomed() && (new_zoom - old_zoom).abs() > 0.001 {
                        return anchor_zoom_scroll(state, old_zoom, new_zoom);
                    }
                }
            }
        }
        Message::ViewerClickUnzoom(cx, cy) => {
            if let Some(idx) = state.viewer.current_index {
                if state.viewer_cache.contains_key(&idx) {
                    state.viewer.zoom_anchor = Some((cx, cy));
                    let old_zoom = state.viewer.zoom_level;
                    state.viewer.adjust_zoom(-4.0);
                    let crossed = state.viewer.tick_zoom();
                    let new_zoom = state.viewer.zoom_level;
                    if state.viewer.is_zoomed() && (new_zoom - old_zoom).abs() > 0.001 {
                        return anchor_zoom_scroll(state, old_zoom, new_zoom);
                    }
                    let _ = crossed;
                }
            }
        }
        Message::PinchZoom(scale, cx, cy) => {
            if let Some(idx) = state.viewer.current_index {
                if !state.viewer_cache.contains_key(&idx) {
                    return Task::none();
                }
                state.viewer.zoom_anchor = Some((cx, cy));
                let old_zoom = state.viewer.zoom_level;
                let new_zoom = (old_zoom * scale).clamp(1.0, 8.0);
                let new_zoom = if new_zoom < 1.02 { 1.0 } else { new_zoom };
                state.viewer.zoom_level = new_zoom;
                state.viewer.zoom_target = new_zoom;
                if new_zoom > 1.0 && (new_zoom - old_zoom).abs() > 0.001 {
                    return anchor_zoom_scroll(state, old_zoom, new_zoom);
                }
                if new_zoom <= 1.0 && old_zoom > 1.0 {
                    state.viewer.zoom_offset = (0.0, 0.0);
                }
            }
        }
        // Screensaver
        Message::ToggleScreensaver => {
            // If zoomed, treat as pan-down instead
            if state.viewer.current_index.is_some() && state.viewer.is_zoomed() {
                return pan_zoom(state, 0.0, 30.0);
            }
            if state.screensaver_active {
                // Stop screensaver
                state.screensaver_active = false;
                state.viewer.close();
                state.cached_metadata = None;
                if !state.was_fullscreen {
                    state.fullscreen = false;
                    return iced::window::latest()
                        .and_then(|id| iced::window::set_mode(id, iced::window::Mode::Windowed));
                }
                return Task::none();
            } else if !state.image_paths.is_empty() {
                // Start screensaver
                state.was_fullscreen = state.fullscreen;
                state.screensaver_active = true;
                // Build shuffled order
                let mut order: Vec<usize> = (0..state.image_paths.len()).collect();
                use rand::seq::SliceRandom;
                order.shuffle(&mut rand::rng());
                state.screensaver_order = order;
                state.screensaver_position = 0;
                // Open first image
                let idx = state.screensaver_order[0];
                state.viewer.open_index(idx);
                refresh_metadata(state);
                let preload = preload_viewer_images(state);
                let preload_next = preload_next_screensaver_image(state);
                // Go fullscreen
                if !state.fullscreen {
                    state.fullscreen = true;
                    let fs = iced::window::latest()
                        .and_then(|id| iced::window::set_mode(id, iced::window::Mode::Fullscreen));
                    return Task::batch([preload, preload_next, fs]);
                }
                return Task::batch([preload, preload_next]);
            }
        }
        Message::ScreensaverAdvance => {
            if !state.screensaver_active {
                return Task::none();
            }
            state.screensaver_position += 1;
            if state.screensaver_position >= state.screensaver_order.len() {
                // Reshuffle and restart
                use rand::seq::SliceRandom;
                state.screensaver_order.shuffle(&mut rand::rng());
                state.screensaver_position = 0;
            }
            let idx = state.screensaver_order[state.screensaver_position];
            state.viewer.open_index(idx);
            state.viewer.reset_zoom();
            refresh_metadata(state);
            let preload = preload_viewer_images(state);
            let preload_next = preload_next_screensaver_image(state);
            return Task::batch([preload, preload_next]);
        }
        // Navigation
        Message::GridScrolled(y) => {
            state.grid_scroll_y = y;
            prioritize_upgrades(state);
        }
        Message::WindowResized(width, height) => {
            let available = width - GRID_PADDING * 2.0;
            let cols = (available / THUMB_CELL).max(1.0) as usize;
            state.grid_columns = cols;
            state.viewport_width = width;
            state.viewport_height = height;
        }
        Message::KeyEscape => {
            if state.screensaver_active {
                state.screensaver_active = false;
                state.viewer.close();
                state.cached_metadata = None;
                if !state.was_fullscreen {
                    state.fullscreen = false;
                    return iced::window::latest()
                        .and_then(|id| iced::window::set_mode(id, iced::window::Mode::Windowed));
                }
                return Task::none();
            } else if state.fullscreen {
                state.fullscreen = false;
                return iced::window::latest()
                    .and_then(|id| iced::window::set_mode(id, iced::window::Mode::Windowed));
            } else if state.viewer.current_index.is_some() && state.viewer.is_zoomed() {
                state.viewer.reset_zoom();
            } else if state.viewer.current_index.is_some() {
                state.viewer.close();
                state.cached_metadata = None;
                return restore_grid_scroll(state);
            } else if state.dup_compare.is_some() {
                state.dup_compare = None;
            } else if state.dup_view_active {
                state.dup_view_active = false;
            } else {
                state.selected_thumb = None;
            }
        }
        Message::KeyLeft => {
            if state.viewer.current_index.is_some() && state.viewer.is_zoomed() {
                return pan_zoom(state, -30.0, 0.0);
            } else if state.viewer.current_index.is_some() {
                state.viewer.prev();
                state.selected_thumb = state.viewer.current_index;
                refresh_metadata(state);
                return preload_viewer_images(state);
            } else if !state.dup_view_active && state.dup_compare.is_none() {
                return move_grid_selection(state, -1);
            }
        }
        Message::KeyRight => {
            if state.viewer.current_index.is_some() && state.viewer.is_zoomed() {
                return pan_zoom(state, 30.0, 0.0);
            } else if state.viewer.current_index.is_some() {
                state.viewer.next(state.image_paths.len());
                state.selected_thumb = state.viewer.current_index;
                refresh_metadata(state);
                return preload_viewer_images(state);
            } else if !state.dup_view_active && state.dup_compare.is_none() {
                return move_grid_selection(state, 1);
            }
        }
        Message::KeyUp => {
            if state.viewer.current_index.is_some() && state.viewer.is_zoomed() {
                return pan_zoom(state, 0.0, -30.0);
            } else if !state.dup_view_active
                && state.dup_compare.is_none()
                && state.viewer.current_index.is_none()
            {
                let cols = state.grid_columns.max(1) as i32;
                return move_grid_selection(state, -cols);
            }
        }
        Message::KeyDown => {
            if state.viewer.current_index.is_some() && state.viewer.is_zoomed() {
                return pan_zoom(state, 0.0, 30.0);
            } else if !state.dup_view_active
                && state.dup_compare.is_none()
                && state.viewer.current_index.is_none()
            {
                let cols = state.grid_columns.max(1) as i32;
                return move_grid_selection(state, cols);
            }
        }
        Message::KeyEnter => {
            if let Some(idx) = state.selected_thumb {
                if state.viewer.current_index.is_none()
                    && !state.dup_view_active
                    && state.dup_compare.is_none()
                    && idx < state.thumbnails.len()
                {
                    state.selected_thumb = Some(idx);
                    state.viewer.open_index(idx);
                    refresh_metadata(state);
                    return preload_viewer_images(state);
                }
            }
        }
        Message::ToggleFullscreen => {
            state.fullscreen = !state.fullscreen;
            let mode = if state.fullscreen {
                iced::window::Mode::Fullscreen
            } else {
                iced::window::Mode::Windowed
            };
            return iced::window::latest()
                .and_then(move |id| iced::window::set_mode(id, mode));
        }
        Message::ToggleSharing => {
            if state.server_handle.is_some() {
                // Stop
                if let Some(handle) = state.server_handle.take() {
                    std::thread::spawn(move || handle.stop());
                }
                state.server_url = None;
                state.qr_handle = None;
            } else if !state.image_paths.is_empty() {
                // Start
                let folder_name = state
                    .folder
                    .as_ref()
                    .and_then(|p| p.file_name())
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| "Photos".to_string());
                if let Some((handle, url)) = server::start_server(
                    state.image_paths.clone(),
                    folder_name,
                ) {
                    state.qr_handle = Some(render_qr(&url));
                    state.server_url = Some(url);
                    state.server_handle = Some(handle);
                }
            }
        }
    }
    Task::none()
}

fn grid_scroll_id() -> iced::widget::Id {
    iced::widget::Id::new("grid")
}

fn dup_list_scroll_id() -> iced::widget::Id {
    iced::widget::Id::new("dup-list")
}

fn move_grid_selection(state: &mut Looky, delta: i32) -> Task<Message> {
    let count = state.thumbnails.len();
    if count == 0 {
        return Task::none();
    }
    let current = state.selected_thumb.unwrap_or(0) as i32;
    let next = (current + delta).clamp(0, count as i32 - 1) as usize;
    state.selected_thumb = Some(next);
    scroll_to_thumb(state, next)
}

fn scroll_to_thumb(state: &Looky, index: usize) -> Task<Message> {
    let cols = state.grid_columns.max(1);
    let row = index / cols;
    let row_top = GRID_PADDING + row as f32 * THUMB_CELL;
    let row_bottom = row_top + THUMB_CELL;

    // Toolbar height is roughly 50px; visible area starts after that.
    // We just ensure the row is within the scroll viewport.
    // If the row is above the current scroll, scroll up to it.
    // If it's below, scroll down so it's visible.
    // We don't know the viewport height exactly, so use a conservative estimate.
    let viewport = state.viewport_height - 50.0; // subtract toolbar height
    let target = if row_top < state.grid_scroll_y {
        row_top
    } else if row_bottom > state.grid_scroll_y + viewport {
        row_bottom - viewport
    } else {
        return Task::none();
    };

    use iced::widget::operation::AbsoluteOffset;
    iced::widget::operation::scroll_to(
        grid_scroll_id(),
        AbsoluteOffset {
            x: None,
            y: Some(target.max(0.0)),
        },
    )
}

fn restore_grid_scroll(state: &Looky) -> Task<Message> {
    use iced::widget::operation::AbsoluteOffset;
    let offset = AbsoluteOffset {
        x: None,
        y: Some(state.grid_scroll_y),
    };
    iced::widget::operation::scroll_to(grid_scroll_id(), offset)
}

fn visible_index_range(state: &Looky) -> std::ops::Range<usize> {
    let cols = state.grid_columns.max(1);
    let toolbar_height = 50.0;
    let first_row = (state.grid_scroll_y / THUMB_CELL).floor().max(0.0) as usize;
    let visible_rows = ((state.viewport_height - toolbar_height) / THUMB_CELL).ceil() as usize + 1;
    let first_idx = first_row * cols;
    let last_idx = ((first_row + visible_rows) * cols).min(state.thumbnails.len());
    first_idx..last_idx
}

fn prioritize_upgrades(state: &mut Looky) {
    if state.pending_upgrades.is_empty() {
        return;
    }
    let visible = visible_index_range(state);
    let visible_paths: HashSet<&PathBuf> = state.thumbnails[visible]
        .iter()
        .map(|(p, _, _)| p)
        .collect();
    // Partition: visible first, then rest
    state
        .pending_upgrades
        .sort_by_key(|p| if visible_paths.contains(p) { 0 } else { 1 });
}

fn load_next_batch(state: &mut Looky) -> Task<Message> {
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

fn load_next_preview_batch(state: &mut Looky) -> Task<Message> {
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

fn load_upgrade_batches(state: &mut Looky) -> Task<Message> {
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

fn load_next_dup_batch(state: &mut Looky) -> Task<Message> {
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

fn preload_viewer_images(state: &mut Looky) -> Task<Message> {
    // Abort all in-flight preloads — the user navigated, old work is stale
    for (idx, handle) in state.viewer_preload_handles.drain(..) {
        log::debug!("viewer: [{}] aborted", idx);
        handle.abort();
    }

    let Some(idx) = state.viewer.current_index else {
        return Task::none();
    };

    // Prioritize the current image — load it first, neighbors come after
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

fn preload_viewer_neighbors(state: &mut Looky) -> Task<Message> {
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

fn preload_next_screensaver_image(state: &mut Looky) -> Task<Message> {
    if !state.screensaver_active {
        return Task::none();
    }
    let next_pos = state.screensaver_position + 1;
    // If we're at the end, we'll reshuffle on advance — can't predict the order
    if next_pos >= state.screensaver_order.len() {
        return Task::none();
    }
    let next_idx = state.screensaver_order[next_pos];
    if state.viewer_cache.contains_key(&next_idx) {
        return Task::none();
    }
    let path = state.image_paths[next_idx].clone();
    let (task, handle) = Task::perform(
        async move {
            match open_image_oriented(&path) {
                Some(rgba) => {
                    let (w, h) = rgba.dimensions();
                    Message::ViewerImageLoaded(next_idx, rgba.into_raw(), w, h)
                }
                None => Message::Tick,
            }
        },
        |msg| msg,
    )
    .abortable();
    state.viewer_preload_handles.push((next_idx, handle));
    task
}

fn open_image_oriented(path: &std::path::Path) -> Option<::image::RgbaImage> {
    let img = ::image::open(path).ok()?;
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

fn refresh_metadata(state: &mut Looky) {
    if let Some(index) = state.viewer.current_index {
        if state.cached_metadata.as_ref().is_some_and(|(i, _)| *i == index) {
            return;
        }
        if let Some(path) = state.image_paths.get(index) {
            let meta = metadata::read_metadata(path);
            state.cached_metadata = Some((index, meta));
        }
    }
}

fn view(state: &Looky) -> Element<'_, Message> {
    let content = view_inner(state);
    let in_viewer = state.viewer.current_index.is_some();
    let screensaver = state.screensaver_active;
    KeyListener::new(content, move |key, repeat| {
        use iced::keyboard::key::Named;
        use iced::keyboard::Key;
        // During screensaver, only allow 's' (toggle off) and Escape
        if screensaver {
            return match &key {
                Key::Character(c) if c.as_str() == "s" && !repeat => {
                    Some(Message::ToggleScreensaver)
                }
                Key::Named(Named::Escape) if !repeat => Some(Message::KeyEscape),
                _ => None,
            };
        }
        match &key {
            // Arrow/WASD keys allow repeats for smooth panning
            Key::Named(Named::ArrowLeft) => Some(Message::KeyLeft),
            Key::Named(Named::ArrowRight) => Some(Message::KeyRight),
            Key::Named(Named::ArrowUp) => Some(Message::KeyUp),
            Key::Named(Named::ArrowDown) => Some(Message::KeyDown),
            Key::Character(c) if c.as_str() == "s" => {
                if repeat {
                    Some(Message::KeyDown)
                } else {
                    Some(Message::ToggleScreensaver)
                }
            }
            Key::Character(c) if matches!(c.as_str(), "a" | "w" | "d") => {
                match c.as_str() {
                    "a" => Some(Message::KeyLeft),
                    "d" => Some(Message::KeyRight),
                    "w" => Some(Message::KeyUp),
                    _ => None,
                }
            }
            _ if repeat => None,
            Key::Named(Named::Space) => Some(Message::ToggleZoom),
            Key::Named(Named::Enter) => Some(Message::KeyEnter),
            Key::Named(Named::Escape) => Some(Message::KeyEscape),
            Key::Character(c) if c.as_str() == "i" => {
                if repeat { return None; }
                Some(Message::ToggleInfo)
            }
            Key::Character(c) if c.as_str() == "f" => {
                if repeat { return None; }
                Some(Message::ToggleFullscreen)
            }
            _ => None,
        }
    })
    .on_scroll(move |delta, cx, cy| {
        if screensaver { return None; }
        if in_viewer {
            Some(Message::ZoomAdjust(delta, cx, cy))
        } else {
            None
        }
    })
    .on_drag(move |dx, dy| {
        if screensaver { return None; }
        if in_viewer {
            Some(Message::ViewerDrag(dx, dy))
        } else {
            Some(Message::DragScroll(dx, dy))
        }
    })
    .on_click(move |cx, cy| {
        if screensaver { return None; }
        if in_viewer {
            Some(Message::ViewerClickZoom(cx, cy))
        } else {
            None
        }
    })
    .on_right_click(move |cx, cy| {
        if screensaver { return None; }
        if in_viewer {
            Some(Message::ViewerClickUnzoom(cx, cy))
        } else {
            None
        }
    })
    .on_pinch(move |scale, cx, cy| {
        if screensaver { return None; }
        if in_viewer {
            Some(Message::PinchZoom(scale, cx, cy))
        } else {
            None
        }
    })
    .into()
}

fn view_inner(state: &Looky) -> Element<'_, Message> {
    // 1. Single-image viewer
    if let Some(index) = state.viewer.current_index {
        if let Some(path) = state.image_paths.get(index) {
            let has_prev = index > 0;
            let has_next = index + 1 < state.image_paths.len();

            let full_handle = state.viewer_cache.get(&index);
            let thumb_handle = state.thumbnails.get(index).map(|(_, h, _)| h);

            let meta = state.cached_metadata.as_ref().map(|(_, m)| m);
            let show_info = state.viewer.show_info;

            let zoom_level = state.viewer.zoom_level;
            let image_dims = state.viewer_dimensions.get(&index).copied();

            return viewer_view(
                path,
                thumb_handle,
                full_handle,
                index,
                state.image_paths.len(),
                has_prev,
                has_next,
                meta,
                show_info,
                state.fullscreen,
                zoom_level,
                image_dims,
                state.viewport_width,
                state.viewport_height,
                state.screensaver_active,
            );
        }
    }

    // 2. Side-by-side comparison view
    if let Some(group_idx) = state.dup_compare {
        if let Some(group) = state.dup_groups.get(group_idx) {
            return duplicates_compare_view(state, group);
        }
    }

    // 3. Duplicates list view
    if state.dup_view_active {
        return duplicates_list_view(state);
    }

    // 4. Grid view with toolbar
    let mut toolbar_items: Vec<Element<'_, Message>> = vec![
        button("Open Folder").on_press(Message::OpenFolder).into(),
    ];

    // "Find Duplicates" / "Scanning..." button
    if !state.image_paths.is_empty() {
        if state.dup_scanning {
            let scanned = state.dup_total - state.dup_pending.len();
            toolbar_items.push(
                text(format!("Scanning {} / {}...", scanned, state.dup_total))
                    .size(13)
                    .color(LABEL_COLOR)
                    .into(),
            );
            toolbar_items.push(
                button("Cancel")
                    .on_press(Message::CancelDupScan)
                    .into(),
            );
        } else {
            let scan_label = if state.dup_groups.is_empty() {
                "Find Duplicates"
            } else {
                "Scan for new"
            };
            toolbar_items.push(button(scan_label).on_press(Message::FindDuplicates).into());
        }
    }

    // "Duplicates (N)" button when groups found
    if !state.dup_groups.is_empty() {
        toolbar_items.push(
            button(text(format!("Duplicates ({})", state.dup_groups.len())))
                .on_press(Message::ShowDuplicatesView)
                .into(),
        );
    }

    // Share button
    if !state.image_paths.is_empty() {
        let share_label = if state.server_handle.is_some() {
            "Stop Sharing"
        } else {
            "Share"
        };
        toolbar_items.push(button(share_label).on_press(Message::ToggleSharing).into());
    }

    // Photo count
    if !state.image_paths.is_empty() {
        let count_text = if state.loading {
            format!(
                "{} / {} photos",
                state.thumbnails.len(),
                state.image_paths.len()
            )
        } else {
            format!("{} photos", state.image_paths.len())
        };
        toolbar_items.push(text(count_text).size(13).color(LABEL_COLOR).into());
    }

    toolbar_items.push(Space::new().width(Length::Fill).into());

    // Right side: URL + QR when sharing, otherwise folder path
    if let (Some(url), Some(qr)) = (&state.server_url, &state.qr_handle) {
        toolbar_items.push(text(url.as_str()).size(13).color(LABEL_COLOR).into());
        toolbar_items.push(
            image(qr.clone())
                .width(28)
                .height(28)
                .into(),
        );
    } else {
        toolbar_items.push(
            text(match &state.folder {
                Some(p) => p.display().to_string(),
                None => "No folder selected".into(),
            })
            .size(14)
            .into(),
        );
    }

    let toolbar = row(toolbar_items).spacing(10).padding(10);

    let content = if state.loading && state.thumbnails.is_empty() {
        column![toolbar, container(text("Loading...")).center(Length::Fill),]
    } else if !state.loading && state.thumbnails.is_empty() {
        column![
            toolbar,
            container(text("Open a folder to browse photos")).center(Length::Fill),
        ]
    } else {
        let grid = thumbnail_grid(state);
        column![
            toolbar,
            scrollable(grid)
                .id(grid_scroll_id())
                .on_scroll(|vp| Message::GridScrolled(vp.absolute_offset().y))
                .height(Length::Fill),
        ]
    };

    container(content).into()
}

const THUMB_SIZE: f32 = 200.0;
const THUMB_CELL: f32 = THUMB_SIZE;
const GRID_PADDING: f32 = 0.0;

fn thumbnail_grid(state: &Looky) -> Element<'_, Message> {
    let thumbnails = &state.thumbnails;
    let badge_set = &state.dup_badge_set;
    let selected = state.selected_thumb;
    let scroll_y = state.grid_scroll_y;
    let viewport_h = state.viewport_height;

    iced::widget::responsive(move |size| {
        let available = size.width - GRID_PADDING * 2.0;
        let thumbs_per_row = (available / THUMB_CELL).max(1.0) as usize;
        let total_rows = (thumbnails.len() + thumbs_per_row - 1) / thumbs_per_row;

        // Determine visible row range (with 1-row buffer above and below)
        let first_visible_row = (scroll_y / THUMB_CELL).floor().max(0.0) as usize;
        let visible_row_count = (viewport_h / THUMB_CELL).ceil() as usize + 2;
        let first_row = first_visible_row.saturating_sub(1);
        let last_row = (first_row + visible_row_count + 1).min(total_rows);

        let mut items: Vec<Element<Message>> = Vec::new();

        // Top spacer for rows above visible range
        if first_row > 0 {
            let spacer_height = first_row as f32 * THUMB_CELL;
            items.push(
                Space::new()
                    .width(Length::Fill)
                    .height(spacer_height)
                    .into(),
            );
        }

        // Render only visible rows
        for row_idx in first_row..last_row {
            let start = row_idx * thumbs_per_row;
            let end = (start + thumbs_per_row).min(thumbnails.len());
            if start >= thumbnails.len() {
                break;
            }

            let row_items: Vec<Element<Message>> = (start..end)
                .map(|index| {
                    let (_path, handle, added) = &thumbnails[index];
                    let age_ms = added.elapsed().as_secs_f32() * 1000.0;
                    let opacity = (age_ms / THUMB_FADE_MS).min(1.0);
                    let img = image(handle.clone())
                        .width(THUMB_SIZE)
                        .height(THUMB_SIZE)
                        .content_fit(iced::ContentFit::Cover)
                        .opacity(opacity);

                    let thumb_content: Element<'_, Message> =
                        if badge_set.contains(&index) {
                            iced::widget::stack![
                                img,
                                container(
                                    container(
                                        text("DUP").size(11).color(Color::WHITE),
                                    )
                                    .padding([2, 6])
                                    .style(dup_badge_style),
                                )
                                .align_right(THUMB_SIZE)
                                .padding(4),
                            ]
                            .into()
                        } else {
                            img.into()
                        };

                    let is_selected = selected == Some(index);
                    let thumb_content: Element<'_, Message> = if is_selected {
                        iced::widget::stack![
                            thumb_content,
                            container(Space::new())
                                .width(THUMB_SIZE)
                                .height(THUMB_SIZE)
                                .style(selection_overlay_style),
                        ]
                        .into()
                    } else {
                        thumb_content
                    };
                    button(thumb_content)
                        .on_press(Message::ViewImage(index))
                        .padding(0)
                        .style(thumb_button_normal)
                        .into()
                })
                .collect();
            items.push(row(row_items).spacing(0).into());
        }

        // Bottom spacer for rows below visible range
        if last_row < total_rows {
            let spacer_height = (total_rows - last_row) as f32 * THUMB_CELL;
            items.push(
                Space::new()
                    .width(Length::Fill)
                    .height(spacer_height)
                    .into(),
            );
        }

        column(items).spacing(0).padding(GRID_PADDING).into()
    })
    .into()
}

fn thumb_button_normal(_theme: &Theme, _status: button::Status) -> button::Style {
    button::Style {
        background: None,
        border: iced::Border::default(),
        ..button::Style::default()
    }
}

fn selection_overlay_style(_theme: &Theme) -> container::Style {
    container::Style {
        border: iced::Border {
            color: Color::WHITE,
            width: 3.0,
            ..Default::default()
        },
        ..Default::default()
    }
}

fn dup_badge_style(theme: &Theme) -> container::Style {
    let palette = theme.palette();
    container::Style {
        background: Some(iced::Background::Color(palette.danger)),
        border: iced::Border {
            radius: 4.0.into(),
            ..Default::default()
        },
        ..Default::default()
    }
}

fn duplicates_list_view(state: &Looky) -> Element<'_, Message> {
    let toolbar = row![
        button("Back").on_press(Message::BackFromDuplicates),
        Space::new().width(Length::Fill),
        text(format!("{} duplicate groups found", state.dup_groups.len())).size(14),
    ]
    .spacing(10)
    .padding(10);

    let cards: Vec<Element<'_, Message>> = state
        .dup_groups
        .iter()
        .enumerate()
        .map(|(group_idx, group)| {
            let (label, label_color) = match &group.match_kind {
                MatchKind::Exact => ("Exact match", Color::from_rgb(0.9, 0.2, 0.2)),
                MatchKind::Visual { distance } => {
                    let _ = distance; // used in display below
                    ("Visual match", Color::from_rgb(0.9, 0.7, 0.1))
                }
            };

            let match_detail = match &group.match_kind {
                MatchKind::Exact => format!("{} identical files", group.indices.len()),
                MatchKind::Visual { distance } => {
                    format!("{} similar files (distance: {})", group.indices.len(), distance)
                }
            };

            // Thumbnail row for this group
            let thumb_row: Vec<Element<'_, Message>> = group
                .indices
                .iter()
                .filter_map(|&idx| {
                    let (_, handle, _) = state.thumbnails.get(idx)?;
                    let summary = state.dup_summaries.get(&idx);
                    let filename = summary
                        .map(|s| s.filename.as_str())
                        .or_else(|| {
                            state.image_paths.get(idx)?
                                .file_name()
                                .and_then(|n| n.to_str())
                        })
                        .unwrap_or_default()
                        .to_string();
                    let subtitle = summary
                        .and_then(|s| s.dimensions)
                        .map(|(w, h)| format!("{} x {}", w, h))
                        .unwrap_or_default();
                    Some(
                        column![
                            image(handle.clone())
                                .width(120)
                                .height(120)
                                .content_fit(iced::ContentFit::Cover),
                            text(filename).size(10),
                            text(subtitle).size(9).color(LABEL_COLOR),
                        ]
                        .spacing(2)
                        .width(130)
                        .into(),
                    )
                })
                .collect();

            let card_content = column![
                row![
                    text(label).size(13).color(label_color),
                    Space::new().width(Length::Fill),
                    text(match_detail).size(12).color(LABEL_COLOR),
                ]
                .spacing(8),
                scrollable(row(thumb_row).spacing(8))
                    .direction(scrollable::Direction::Horizontal(
                        scrollable::Scrollbar::default(),
                    )),
                button("Compare").on_press(Message::CompareDuplicates(group_idx)),
            ]
            .spacing(8)
            .padding(12);

            container(card_content)
                .width(Length::Fill)
                .style(container::bordered_box)
                .into()
        })
        .collect();

    let list = scrollable(column(cards).spacing(12).padding(16))
        .id(dup_list_scroll_id())
        .on_scroll(|vp| Message::DupListScrolled(vp.absolute_offset().y))
        .height(Length::Fill);

    container(column![toolbar, list]).into()
}

fn duplicates_compare_view<'a>(state: &'a Looky, group: &'a DuplicateGroup) -> Element<'a, Message> {
    let (label, label_color) = match &group.match_kind {
        MatchKind::Exact => ("Exact match", Color::from_rgb(0.9, 0.2, 0.2)),
        MatchKind::Visual { distance } => {
            let _ = distance;
            ("Visual match", Color::from_rgb(0.9, 0.7, 0.1))
        }
    };

    let toolbar = row![
        button("Back").on_press(Message::BackFromCompare),
        Space::new().width(Length::Fill),
        text(label).size(14).color(label_color),
    ]
    .spacing(10)
    .padding(10);

    let images: Vec<Element<'_, Message>> = group
        .indices
        .iter()
        .filter_map(|&idx| {
            let path = state.image_paths.get(idx)?;
            let info = state.dup_summaries.get(&idx);

            let filename = info
                .map(|s| s.filename.clone())
                .or_else(|| {
                    path.file_name()
                        .map(|n| n.to_string_lossy().to_string())
                })
                .unwrap_or_default();
            let dims_text = info
                .and_then(|s| s.dimensions)
                .map(|(w, h)| format!("{} x {} px", w, h))
                .unwrap_or_default();
            let size_text = info
                .map(|s| metadata::format_file_size(s.file_size))
                .unwrap_or_default();

            let mut details: Vec<Element<'_, Message>> = vec![
                text(filename).size(13).into(),
                text(format!("{}  {}", dims_text, size_text))
                    .size(11)
                    .color(LABEL_COLOR)
                    .into(),
            ];
            if let Some(date) = info.and_then(|s| s.date_taken.as_deref()) {
                details.push(
                    text(format!("Taken: {}", date))
                        .size(11)
                        .color(LABEL_COLOR)
                        .into(),
                );
            }
            if let Some(date) = info.and_then(|s| s.date_modified.as_deref()) {
                details.push(
                    text(format!("Modified: {}", date))
                        .size(11)
                        .color(LABEL_COLOR)
                        .into(),
                );
            }

            Some(
                column![
                    image(path.to_string_lossy().to_string())
                        .content_fit(iced::ContentFit::Contain)
                        .width(Length::Fill)
                        .height(Length::Fill),
                    column(details).spacing(2),
                ]
                .spacing(4)
                .align_x(iced::Alignment::Center)
                .width(Length::Fill)
                .height(Length::Fill)
                .into(),
            )
        })
        .collect();

    let compare_row = row(images)
        .spacing(16)
        .padding(16)
        .height(Length::Fill)
        .width(Length::Fill);

    container(column![toolbar, compare_row]).into()
}

fn viewer_scroll_id() -> iced::widget::Id {
    iced::widget::Id::new("viewer-zoom")
}

/// Compute the fit-to-screen base size for an image in the viewport.
/// Returns (fit_w, fit_h) — the size the image would be at zoom 1.0.
fn fit_size(img_w: u32, img_h: u32, vp_w: f32, vp_h: f32) -> (f32, f32) {
    let scale = (vp_w / img_w as f32).min(vp_h / img_h as f32);
    (img_w as f32 * scale, img_h as f32 * scale)
}

/// Compute the centering padding for the zoomed image inside the scrollable.
/// The container is max(render_size, viewport_size), so when the image is smaller
/// than the viewport, padding centers it.
fn zoom_padding(render: f32, viewport: f32) -> f32 {
    ((viewport - render) / 2.0).max(0.0)
}

fn center_zoom_scroll(state: &Looky) -> Task<Message> {
    let Some(idx) = state.viewer.current_index else {
        return Task::none();
    };
    let Some(&(img_w, img_h)) = state.viewer_dimensions.get(&idx) else {
        return Task::none();
    };

    let vp_w = state.viewport_width;
    let vp_h = state.viewport_height - 50.0;
    let (fit_w, fit_h) = fit_size(img_w, img_h, vp_w, vp_h);
    let render_w = fit_w * state.viewer.zoom_level;
    let render_h = fit_h * state.viewer.zoom_level;
    let pad_x = zoom_padding(render_w, vp_w);
    let pad_y = zoom_padding(render_h, vp_h);

    if let Some((anchor_x, anchor_y)) = state.viewer.zoom_anchor {
        // Zoom toward cursor. Cursor is in window coords — convert to
        // viewport-relative coords (subtract toolbar).
        let rel_x = anchor_x;
        let rel_y = anchor_y - 50.0;
        // At zoom=1.0, the image was centered (same as non-zoomed view).
        // The cursor fraction within the image:
        let img_x = rel_x - pad_x;
        let img_y = rel_y - pad_y;
        // Desired scroll: place that image point under the cursor
        let scroll_x = (img_x + pad_x - rel_x).max(0.0);
        let scroll_y = (img_y + pad_y - rel_y).max(0.0);
        // (simplifies to 0 when image < viewport, which is correct)

        use iced::widget::operation::AbsoluteOffset;
        iced::widget::operation::scroll_to(
            viewer_scroll_id(),
            AbsoluteOffset {
                x: Some(scroll_x),
                y: Some(scroll_y),
            },
        )
    } else {
        // No anchor — center the view
        let center_x = ((render_w - vp_w) / 2.0).max(0.0);
        let center_y = ((render_h - vp_h) / 2.0).max(0.0);

        use iced::widget::operation::AbsoluteOffset;
        iced::widget::operation::scroll_to(
            viewer_scroll_id(),
            AbsoluteOffset {
                x: Some(center_x),
                y: Some(center_y),
            },
        )
    }
}

/// Adjust scroll offset during zoom animation to keep the anchor point (or
/// center) fixed as zoom_level changes from `old_zoom` to `new_zoom`.
fn anchor_zoom_scroll(state: &mut Looky, old_zoom: f32, new_zoom: f32) -> Task<Message> {
    let Some(idx) = state.viewer.current_index else {
        return Task::none();
    };
    let Some(&(img_w, img_h)) = state.viewer_dimensions.get(&idx) else {
        return Task::none();
    };

    let vp_w = state.viewport_width;
    let vp_h = state.viewport_height - 50.0;
    let (fit_w, fit_h) = fit_size(img_w, img_h, vp_w, vp_h);

    let old_render_w = fit_w * old_zoom;
    let old_render_h = fit_h * old_zoom;
    let new_render_w = fit_w * new_zoom;
    let new_render_h = fit_h * new_zoom;

    let old_pad_x = zoom_padding(old_render_w, vp_w);
    let old_pad_y = zoom_padding(old_render_h, vp_h);
    let new_pad_x = zoom_padding(new_render_w, vp_w);
    let new_pad_y = zoom_padding(new_render_h, vp_h);

    let (scroll_x, scroll_y) = state.viewer.zoom_offset;

    if let Some((anchor_x, anchor_y)) = state.viewer.zoom_anchor {
        // Cursor position relative to the scrollable viewport
        let rel_x = anchor_x;
        let rel_y = anchor_y - 50.0;

        // Content position under cursor in the old layout (includes padding)
        let content_x = scroll_x + rel_x;
        let content_y = scroll_y + rel_y;

        // Position within the actual image (subtract old padding)
        let img_x = content_x - old_pad_x;
        let img_y = content_y - old_pad_y;

        // Scale to new zoom
        let ratio = new_zoom / old_zoom;
        let new_img_x = img_x * ratio;
        let new_img_y = img_y * ratio;

        // Convert back to content coords (add new padding)
        let new_content_x = new_img_x + new_pad_x;
        let new_content_y = new_img_y + new_pad_y;

        // Scroll to keep cursor over the same image point
        let new_scroll_x = (new_content_x - rel_x).max(0.0);
        let new_scroll_y = (new_content_y - rel_y).max(0.0);

        // Clamp to max scroll (content_size - viewport_size)
        let content_w = new_render_w.max(vp_w);
        let content_h = new_render_h.max(vp_h);
        let max_x = (content_w - vp_w).max(0.0);
        let max_y = (content_h - vp_h).max(0.0);
        let new_scroll_x = new_scroll_x.min(max_x);
        let new_scroll_y = new_scroll_y.min(max_y);
        state.viewer.zoom_offset = (new_scroll_x, new_scroll_y);

        use iced::widget::operation::AbsoluteOffset;
        iced::widget::operation::scroll_to(
            viewer_scroll_id(),
            AbsoluteOffset {
                x: Some(new_scroll_x),
                y: Some(new_scroll_y),
            },
        )
    } else {
        // No anchor — keep centered
        let center_x = ((new_render_w - vp_w) / 2.0).max(0.0);
        let center_y = ((new_render_h - vp_h) / 2.0).max(0.0);
        state.viewer.zoom_offset = (center_x, center_y);

        use iced::widget::operation::AbsoluteOffset;
        iced::widget::operation::scroll_to(
            viewer_scroll_id(),
            AbsoluteOffset {
                x: Some(center_x),
                y: Some(center_y),
            },
        )
    }
}

fn pan_zoom(state: &mut Looky, dx: f32, dy: f32) -> Task<Message> {
    let (ox, oy) = state.viewer.zoom_offset;
    let new_x = (ox + dx).max(0.0);
    let new_y = (oy + dy).max(0.0);
    state.viewer.zoom_offset = (new_x, new_y);

    use iced::widget::operation::AbsoluteOffset;
    iced::widget::operation::scroll_to(
        viewer_scroll_id(),
        AbsoluteOffset {
            x: Some(new_x),
            y: Some(new_y),
        },
    )
}

fn viewer_view<'a>(
    path: &'a PathBuf,
    thumb_handle: Option<&'a image::Handle>,
    full_handle: Option<&'a image::Handle>,
    index: usize,
    total: usize,
    has_prev: bool,
    has_next: bool,
    meta: Option<&'a PhotoMetadata>,
    show_info: bool,
    fullscreen: bool,
    zoom_level: f32,
    image_dims: Option<(u32, u32)>,
    viewport_width: f32,
    viewport_height: f32,
    screensaver: bool,
) -> Element<'a, Message> {
    // Screensaver mode: just the image on a black background, no UI chrome, hidden cursor
    if screensaver {
        // Prefer full-res only to avoid low→high-res flicker
        let handle = full_handle.or(thumb_handle);
        let image_layer: Element<'a, Message> = if let Some(h) = handle {
            let img = image(h.clone())
                .content_fit(iced::ContentFit::Contain)
                .width(Length::Fill)
                .height(Length::Fill);
            container(img).center(Length::Fill).into()
        } else {
            container(Space::new()).center(Length::Fill).into()
        };
        let view = container(image_layer)
            .width(Length::Fill)
            .height(Length::Fill)
            .style(screensaver_bg_style);
        return iced::widget::MouseArea::new(view)
            .interaction(iced::mouse::Interaction::Hidden)
            .into();
    }

    let filename = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();

    let zoom_label = if zoom_level > 1.0 {
        format!(" [{}%]", (zoom_level * 100.0) as u32)
    } else {
        String::new()
    };
    let info_label = if show_info { "Info \u{2190}" } else { "Info \u{2192}" };
    let fs_label = if fullscreen { "Window" } else { "Fullscreen" };
    let toolbar = row![
        button("Back").on_press(Message::BackToGrid),
        button(info_label).on_press(Message::ToggleInfo),
        button(fs_label).on_press(Message::ToggleFullscreen),
        Space::new().width(Length::Fill),
        text(format!("{} ({}/{}){}", filename, index + 1, total, zoom_label)).size(14),
    ]
    .spacing(10)
    .padding(10);

    if zoom_level > 1.0 {
        // Zoomed view: render at zoom_level × fit-to-screen size
        let handle = full_handle.or(thumb_handle);
        let image_layer: Element<'a, Message> = if let Some(h) = handle {
            let (img_w, img_h) = image_dims.unwrap_or((800, 600));
            let avail_w = viewport_width;
            let avail_h = viewport_height - 50.0;
            let (fit_w, fit_h) = fit_size(img_w, img_h, avail_w, avail_h);
            let render_w = fit_w * zoom_level;
            let render_h = fit_h * zoom_level;
            let img = image(h.clone())
                .content_fit(iced::ContentFit::Fill)
                .width(render_w)
                .height(render_h);
            container(img)
                .center_x(render_w.max(avail_w))
                .center_y(render_h.max(avail_h))
                .into()
        } else {
            container(Space::new()).center(Length::Fill).into()
        };

        let zoom_scroll = scrollable(image_layer)
            .id(viewer_scroll_id())
            .width(Length::Fill)
            .height(Length::Fill)
            .direction(scrollable::Direction::Both {
                vertical: scrollable::Scrollbar::default(),
                horizontal: scrollable::Scrollbar::default(),
            })
            .on_scroll(|vp| {
                let offset = vp.absolute_offset();
                Message::ZoomScrolled(offset.x, offset.y)
            });

        let mut layers: Vec<Element<'_, Message>> = vec![zoom_scroll.into()];
        if show_info {
            if let Some(m) = meta {
                layers.push(info_panel(m));
            }
        }
        let body = iced::widget::Stack::with_children(layers)
            .width(Length::Fill)
            .height(Length::Fill);

        return column![toolbar, body].into();
    }

    // Normal (fit-to-screen) view
    let image_layer: Element<'a, Message> = match (full_handle, thumb_handle) {
        (Some(full), Some(thumb)) => {
            let thumb_img = image(thumb.clone())
                .content_fit(iced::ContentFit::Contain)
                .width(Length::Fill)
                .height(Length::Fill);
            let full_img = image(full.clone())
                .content_fit(iced::ContentFit::Contain)
                .width(Length::Fill)
                .height(Length::Fill);
            iced::widget::stack![
                container(thumb_img).center(Length::Fill),
                container(full_img).center(Length::Fill),
            ]
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
        }
        (Some(full), None) => {
            let full_img = image(full.clone())
                .content_fit(iced::ContentFit::Contain)
                .width(Length::Fill)
                .height(Length::Fill);
            container(full_img).center(Length::Fill).into()
        }
        (None, Some(thumb)) => {
            let thumb_img = image(thumb.clone())
                .content_fit(iced::ContentFit::Contain)
                .width(Length::Fill)
                .height(Length::Fill);
            container(thumb_img).center(Length::Fill).into()
        }
        (None, None) => {
            container(Space::new())
                .center(Length::Fill)
                .into()
        }
    };

    // Left nav zone
    let left_zone: Element<'_, Message> = if has_prev {
        button(
            container(text("\u{2039}").size(48))
                .center_y(Length::Fill)
                .padding([0, 16]),
        )
        .on_press(Message::PrevImage)
        .style(button::text)
        .height(Length::Fill)
        .width(Length::FillPortion(3))
        .into()
    } else {
        Space::new()
            .width(Length::FillPortion(3))
            .height(Length::Fill)
            .into()
    };

    // Right nav zone
    let right_zone: Element<'_, Message> = if has_next {
        button(
            container(text("\u{203A}").size(48))
                .center_y(Length::Fill)
                .align_right(Length::Fill)
                .padding([0, 16]),
        )
        .on_press(Message::NextImage)
        .style(button::text)
        .height(Length::Fill)
        .width(Length::FillPortion(3))
        .into()
    } else {
        Space::new()
            .width(Length::FillPortion(3))
            .height(Length::Fill)
            .into()
    };

    let nav_overlay = row![
        left_zone,
        Space::new()
            .width(Length::FillPortion(14))
            .height(Length::Fill),
        right_zone,
    ]
    .height(Length::Fill)
    .width(Length::Fill);

    let image_with_nav = iced::widget::stack![image_layer, nav_overlay,]
        .width(Length::Fill)
        .height(Length::Fill);

    let mut layers: Vec<Element<'_, Message>> = vec![image_with_nav.into()];
    if show_info {
        if let Some(m) = meta {
            layers.push(info_panel(m));
        }
    }
    let body = iced::widget::Stack::with_children(layers)
        .width(Length::Fill)
        .height(Length::Fill);

    column![toolbar, body].into()
}

const LABEL_COLOR: Color = Color::from_rgb(0.5, 0.5, 0.55);

fn info_panel(meta: &PhotoMetadata) -> Element<'_, Message> {
    let mut items: Vec<Element<'_, Message>> = Vec::new();

    // File header
    items.push(
        text(&meta.filename)
            .size(15)
            .wrapping(text::Wrapping::WordOrGlyph)
            .into(),
    );
    items.push(
        text(metadata::format_file_size(meta.file_size))
            .size(12)
            .color(LABEL_COLOR)
            .into(),
    );
    if let Some((w, h)) = meta.dimensions {
        items.push(
            text(format!("{} x {} px", w, h))
                .size(12)
                .color(LABEL_COLOR)
                .into(),
        );
    }

    // Date
    let has_dates = meta.date_taken.is_some() || meta.date_modified.is_some();
    if has_dates {
        items.push(section_divider());
        if let Some(ref date) = meta.date_taken {
            items.push(info_field("Date Taken", date.clone()));
        }
        if let Some(ref date) = meta.date_modified {
            items.push(info_field("Modified", date.clone()));
        }
    }

    // Camera section
    let has_camera = meta.camera_make.is_some()
        || meta.camera_model.is_some()
        || meta.lens_model.is_some()
        || meta.software.is_some();
    if has_camera {
        items.push(section_divider());
        items.push(section_header("Camera"));
        if let Some(ref make) = meta.camera_make {
            items.push(info_field("Make", make.clone()));
        }
        if let Some(ref model) = meta.camera_model {
            items.push(info_field("Model", model.clone()));
        }
        if let Some(ref lens) = meta.lens_model {
            items.push(info_field("Lens", lens.clone()));
        }
        if let Some(ref sw) = meta.software {
            items.push(info_field("Software", sw.clone()));
        }
    }

    // Exposure section
    let has_exposure = meta.exposure_time.is_some()
        || meta.f_number.is_some()
        || meta.iso.is_some()
        || meta.focal_length.is_some();
    if has_exposure {
        items.push(section_divider());
        items.push(section_header("Exposure"));

        // Compact exposure summary line: 1/250s  f/2.8  ISO 400
        let mut summary_parts: Vec<String> = Vec::new();
        if let Some(ref exp) = meta.exposure_time {
            summary_parts.push(format!("{}s", exp));
        }
        if let Some(ref f) = meta.f_number {
            summary_parts.push(format!("f/{}", f));
        }
        if let Some(ref iso) = meta.iso {
            summary_parts.push(format!("ISO {}", iso));
        }
        if !summary_parts.is_empty() {
            items.push(text(summary_parts.join("  ")).size(13).into());
        }

        if let Some(ref fl) = meta.focal_length {
            let value = match &meta.focal_length_35mm {
                Some(eq) => format!("{} ({}mm eq.)", fl, eq),
                None => fl.clone(),
            };
            items.push(info_field("Focal length", value));
        }
        if let Some(ref bias) = meta.exposure_bias {
            items.push(info_field("Exp. bias", format!("{} EV", bias)));
        }
        if let Some(ref prog) = meta.exposure_program {
            items.push(info_field("Program", prog.clone()));
        }
        if let Some(ref meter) = meta.metering_mode {
            items.push(info_field("Metering", meter.clone()));
        }
    }

    // Light & color section
    let has_light =
        meta.flash.is_some() || meta.white_balance.is_some() || meta.color_space.is_some();
    if has_light {
        items.push(section_divider());
        items.push(section_header("Light & Color"));
        if let Some(ref flash) = meta.flash {
            items.push(info_field("Flash", flash.clone()));
        }
        if let Some(ref wb) = meta.white_balance {
            items.push(info_field("White balance", wb.clone()));
        }
        if let Some(ref cs) = meta.color_space {
            items.push(info_field("Color space", cs.clone()));
        }
    }

    // GPS section
    let has_gps = meta.gps_latitude.is_some() || meta.gps_altitude.is_some();
    if has_gps {
        items.push(section_divider());
        items.push(section_header("Location"));
        if let (Some(lat), Some(lon)) = (meta.gps_latitude, meta.gps_longitude) {
            items.push(info_field("Coordinates", format!("{:.6}, {:.6}", lat, lon)));
        }
        if let Some(ref alt) = meta.gps_altitude {
            items.push(info_field("Altitude", alt.clone()));
        }
    }

    // Credits section
    let has_credits = meta.artist.is_some() || meta.copyright.is_some() || meta.description.is_some();
    if has_credits {
        items.push(section_divider());
        if let Some(ref desc) = meta.description {
            items.push(info_field("Description", desc.clone()));
        }
        if let Some(ref artist) = meta.artist {
            items.push(info_field("Artist", artist.clone()));
        }
        if let Some(ref cr) = meta.copyright {
            items.push(info_field("Copyright", cr.clone()));
        }
    }

    let panel_content = column(items).spacing(6).padding(16).width(280);

    container(
        container(panel_content)
            .width(280)
            .clip(true)
            .style(info_panel_style),
    )
    .padding(12)
    .into()
}

fn screensaver_bg_style(_theme: &Theme) -> container::Style {
    container::Style {
        background: Some(iced::Background::Color(Color::BLACK)),
        ..Default::default()
    }
}

fn info_panel_style(_theme: &Theme) -> container::Style {
    container::Style {
        background: Some(iced::Background::Color(Color::from_rgba(0.1, 0.1, 0.1, 0.85))),
        border: iced::Border {
            radius: 8.0.into(),
            ..Default::default()
        },
        ..Default::default()
    }
}

fn section_header(label: &str) -> Element<'_, Message> {
    text(label.to_string())
        .size(11)
        .color(LABEL_COLOR)
        .into()
}

fn section_divider<'a>() -> Element<'a, Message> {
    container(rule::horizontal(1))
        .padding([4, 0])
        .into()
}

fn info_field(label: &str, value: String) -> Element<'_, Message> {
    row![
        text(label.to_string()).size(12).color(LABEL_COLOR).width(90),
        text(value).size(12),
    ]
    .spacing(8)
    .into()
}

fn render_qr(url: &str) -> image::Handle {
    use qrcode::QrCode;
    let code = QrCode::new(url.as_bytes()).unwrap();
    let modules = code.to_colors();
    let size = code.width();
    let scale = 4u32;
    let quiet = 2u32;
    let img_size = (size as u32) * scale + quiet * 2 * scale;
    let mut pixels = vec![255u8; (img_size * img_size * 4) as usize];
    for y in 0..size {
        for x in 0..size {
            let dark = modules[y * size + x] == qrcode::Color::Dark;
            if dark {
                let px = x as u32 * scale + quiet * scale;
                let py = y as u32 * scale + quiet * scale;
                for dy in 0..scale {
                    for dx in 0..scale {
                        let offset = ((py + dy) * img_size + (px + dx)) as usize * 4;
                        pixels[offset] = 0;
                        pixels[offset + 1] = 0;
                        pixels[offset + 2] = 0;
                        pixels[offset + 3] = 255;
                    }
                }
            }
        }
    }
    image::Handle::from_rgba(img_size, img_size, pixels)
}

fn theme(_state: &Looky) -> Theme {
    Theme::Dark
}

async fn pick_folder() -> Option<PathBuf> {
    rfd::AsyncFileDialog::new()
        .set_title("Select a photo folder")
        .pick_folder()
        .await
        .map(|handle| handle.path().to_path_buf())
}

async fn scan_folder(folder: PathBuf) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    let mut stack = vec![folder];
    while let Some(dir) = stack.pop() {
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                } else if is_image_file(&path) {
                    paths.push(path);
                }
            }
        }
    }
    paths.sort();
    paths
}

fn is_image_file(path: &std::path::Path) -> bool {
    match path.extension().and_then(|e| e.to_str()) {
        Some(ext) => matches!(
            ext.to_lowercase().as_str(),
            "jpg" | "jpeg" | "png" | "gif" | "bmp" | "webp" | "tiff" | "tif"
        ),
        None => false,
    }
}

fn config_dir() -> Option<PathBuf> {
    dirs_next::home_dir().map(|d| d.join(".looky"))
}

fn save_last_folder(path: &std::path::Path) {
    if let Some(dir) = config_dir() {
        let _ = std::fs::create_dir_all(&dir);
        let _ = std::fs::write(dir.join("last_folder"), path.to_string_lossy().as_bytes());
    }
}

fn load_last_folder() -> Option<PathBuf> {
    let dir = config_dir()?;
    let data = std::fs::read_to_string(dir.join("last_folder")).ok()?;
    let path = PathBuf::from(data.trim());
    if path.is_dir() {
        Some(path)
    } else {
        None
    }
}
