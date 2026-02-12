use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use iced::widget::{button, column, container, image, row, rule, scrollable, text, Space};
use iced::{Color, Element, Length, Subscription, Task, Theme};

use crate::catalog::{self, Catalog};
use crate::duplicates::{self, DuplicateGroup, ImageHashes, MatchKind};
use crate::metadata::{self, PhotoMetadata};
use crate::thumbnail;
use crate::viewer::ViewerState;

const THUMBNAIL_BATCH_SIZE: usize = 32;
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
    grid_columns: usize,
    selected_thumb: Option<usize>,
    viewer_cache: HashMap<usize, image::Handle>,
}

impl Default for Looky {
    fn default() -> Self {
        Self {
            folder: None,
            image_paths: Vec::new(),
            thumbnails: Vec::new(),
            pending_thumbnails: Vec::new(),
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
            grid_columns: 4,
            selected_thumb: None,
            viewer_cache: HashMap::new(),
        }
    }
}

#[derive(Debug, Clone)]
pub enum Message {
    OpenFolder,
    FolderSelected(Option<PathBuf>),
    ImagesFound(Vec<PathBuf>),
    ThumbnailBatchReady(Vec<(PathBuf, Vec<u8>, u32, u32)>),
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
    // Navigation
    GridScrolled(f32),
    WindowResized(f32),
    KeyEscape,
    KeyLeft,
    KeyRight,
    KeyUp,
    KeyDown,
    KeyEnter,
}

fn subscription(state: &Looky) -> Subscription<Message> {
    let events = iced::event::listen_with(|event, _status, _window| match event {
        iced::Event::Keyboard(iced::keyboard::Event::KeyPressed { key, .. }) => {
            use iced::keyboard::key::Named;
            use iced::keyboard::Key;
            match key {
                Key::Named(Named::ArrowLeft) => Some(Message::KeyLeft),
                Key::Named(Named::ArrowRight) => Some(Message::KeyRight),
                Key::Named(Named::ArrowUp) => Some(Message::KeyUp),
                Key::Named(Named::ArrowDown) => Some(Message::KeyDown),
                Key::Named(Named::Enter) => Some(Message::KeyEnter),
                Key::Named(Named::Escape) => Some(Message::KeyEscape),
                _ => None,
            }
        }
        iced::Event::Window(iced::window::Event::Resized(size)) => {
            Some(Message::WindowResized(size.width))
        }
        _ => None,
    });

    let needs_tick = state.viewer.is_transitioning() || thumbnails_fading(state);
    if needs_tick {
        Subscription::batch([
            events,
            iced::time::every(Duration::from_millis(16)).map(|_| Message::Tick),
        ])
    } else {
        events
    }
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
            state.folder = Some(path.clone());
            state.thumbnails.clear();
            state.image_paths.clear();
            state.pending_thumbnails.clear();
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
                    return Task::batch([load_next_batch(state), task]);
                }
            }
            return load_next_batch(state);
        }
        Message::ThumbnailBatchReady(results) => {
            let now = Instant::now();
            for (path, rgba, width, height) in results {
                let handle = image::Handle::from_rgba(width, height, rgba);
                state.thumbnails.push((path, handle, now));
            }
            return load_next_batch(state);
        }
        Message::ViewImage(index) => {
            state.selected_thumb = Some(index);
            state.viewer.open_index(index);
            refresh_metadata(state);
            return preload_viewer_images(state);
        }
        Message::NextImage => {
            state.viewer.next(state.image_paths.len());
            refresh_metadata(state);
            return preload_viewer_images(state);
        }
        Message::PrevImage => {
            state.viewer.prev();
            refresh_metadata(state);
            return preload_viewer_images(state);
        }
        Message::BackToGrid => {
            state.viewer.close();
            state.cached_metadata = None;
            state.viewer_cache.clear();
            return restore_grid_scroll(state);
        }
        Message::ToggleInfo => {
            state.viewer.toggle_info();
        }
        Message::ViewerImageLoaded(index, rgba, width, height) => {
            let handle = image::Handle::from_rgba(width, height, rgba);
            state.viewer_cache.insert(index, handle);
            // Evict distant entries to limit memory (keep ±3 of current)
            if let Some(current) = state.viewer.current_index {
                let keep_min = current.saturating_sub(3);
                let keep_max = current + 3;
                state
                    .viewer_cache
                    .retain(|&k, _| k >= keep_min && k <= keep_max);
            }
        }
        Message::Tick => {
            state.viewer.tick();
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
        // Navigation
        Message::GridScrolled(y) => {
            state.grid_scroll_y = y;
        }
        Message::WindowResized(width) => {
            let available = width - GRID_PADDING * 2.0;
            let cols = ((available + 8.0) / THUMB_CELL).max(1.0) as usize;
            state.grid_columns = cols;
        }
        Message::KeyEscape => {
            if state.viewer.current_index.is_some() {
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
            if state.viewer.current_index.is_some() {
                state.viewer.prev();
                refresh_metadata(state);
                return preload_viewer_images(state);
            } else if !state.dup_view_active && state.dup_compare.is_none() {
                return move_grid_selection(state, -1);
            }
        }
        Message::KeyRight => {
            if state.viewer.current_index.is_some() {
                state.viewer.next(state.image_paths.len());
                refresh_metadata(state);
                return preload_viewer_images(state);
            } else if !state.dup_view_active && state.dup_compare.is_none() {
                return move_grid_selection(state, 1);
            }
        }
        Message::KeyUp => {
            if !state.dup_view_active
                && state.dup_compare.is_none()
                && state.viewer.current_index.is_none()
            {
                let cols = state.grid_columns.max(1) as i32;
                return move_grid_selection(state, -cols);
            }
        }
        Message::KeyDown => {
            if !state.dup_view_active
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
                    state.viewer.open_index(idx);
                    refresh_metadata(state);
                }
            }
        }
    }
    Task::none()
}

fn grid_scroll_id() -> iced::widget::Id {
    iced::widget::Id::new("grid")
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
    let target = if row_top < state.grid_scroll_y {
        row_top
    } else if row_bottom > state.grid_scroll_y + 600.0 {
        // Approximate: keep the row near the bottom of a ~600px viewport
        row_bottom - 600.0
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

fn preload_viewer_images(state: &Looky) -> Task<Message> {
    let Some(idx) = state.viewer.current_index else {
        return Task::none();
    };
    let total = state.image_paths.len();
    let mut tasks = Vec::new();
    for i in [idx.saturating_sub(1), idx, (idx + 1).min(total.saturating_sub(1))] {
        if i < total && !state.viewer_cache.contains_key(&i) {
            let path = state.image_paths[i].clone();
            let index = i;
            tasks.push(Task::perform(
                async move {
                    let img = open_image_oriented(&path);
                    match img {
                        Some(rgba) => {
                            let (w, h) = rgba.dimensions();
                            Message::ViewerImageLoaded(index, rgba.into_raw(), w, h)
                        }
                        None => Message::Tick,
                    }
                },
                |msg| msg,
            ));
        }
    }
    if tasks.is_empty() {
        Task::none()
    } else {
        Task::batch(tasks)
    }
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
    // 1. Single-image viewer
    if let Some(index) = state.viewer.current_index {
        if let Some(path) = state.image_paths.get(index) {
            let has_prev = index > 0;
            let has_next = index + 1 < state.image_paths.len();

            let current_handle = state.viewer_cache.get(&index);

            let fade_from = state.viewer.transition.as_ref().and_then(|t| {
                let progress = state.viewer.transition_progress().unwrap_or(1.0);
                if progress < 1.0 {
                    let from_path = state.image_paths.get(t.from_index)?;
                    let from_handle = state.viewer_cache.get(&t.from_index);
                    Some((from_path, from_handle, progress))
                } else {
                    None
                }
            });

            let meta = if state.viewer.show_info {
                state.cached_metadata.as_ref().map(|(_, m)| m)
            } else {
                None
            };

            return viewer_view(
                path,
                current_handle,
                index,
                state.image_paths.len(),
                has_prev,
                has_next,
                fade_from,
                meta,
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
    toolbar_items.push(
        text(match &state.folder {
            Some(p) => p.display().to_string(),
            None => "No folder selected".into(),
        })
        .size(14)
        .into(),
    );

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
const THUMB_CELL: f32 = THUMB_SIZE + 8.0 + 8.0; // image + button padding + spacing
const GRID_PADDING: f32 = 10.0;

fn thumbnail_grid(state: &Looky) -> Element<'_, Message> {
    let thumbnails = &state.thumbnails;
    let badge_set = &state.dup_badge_set;
    let selected = state.selected_thumb;

    iced::widget::responsive(move |size| {
        let available = size.width - GRID_PADDING * 2.0;
        let thumbs_per_row = ((available + 8.0) / THUMB_CELL).max(1.0) as usize;

        let rows: Vec<Element<Message>> = thumbnails
            .chunks(thumbs_per_row)
            .enumerate()
            .map(|(row_idx, chunk)| {
                let items: Vec<Element<Message>> = chunk
                    .iter()
                    .enumerate()
                    .map(|(col_idx, (_path, handle, added))| {
                        let index = row_idx * thumbs_per_row + col_idx;
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
                        let btn = button(thumb_content)
                            .on_press(Message::ViewImage(index))
                            .padding(4);
                        let btn: Element<'_, Message> = if is_selected {
                            btn.style(button::primary).into()
                        } else {
                            btn.into()
                        };

                        if is_selected {
                            container(btn)
                                .style(selected_thumb_style)
                                .into()
                        } else {
                            btn
                        }
                    })
                    .collect();
                row(items).spacing(8).into()
            })
            .collect();

        column(rows).spacing(8).padding(GRID_PADDING).into()
    })
    .into()
}

fn selected_thumb_style(theme: &Theme) -> container::Style {
    let palette = theme.palette();
    container::Style {
        border: iced::Border {
            color: palette.primary,
            width: 2.0,
            radius: 4.0.into(),
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

    let list = scrollable(column(cards).spacing(12).padding(16)).height(Length::Fill);

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

fn viewer_view<'a>(
    path: &'a PathBuf,
    current_handle: Option<&'a image::Handle>,
    index: usize,
    total: usize,
    has_prev: bool,
    has_next: bool,
    fade_from: Option<(&'a PathBuf, Option<&'a image::Handle>, f32)>,
    meta: Option<&'a PhotoMetadata>,
) -> Element<'a, Message> {
    let new_img = viewer_image(path, current_handle)
        .content_fit(iced::ContentFit::Contain)
        .width(Length::Fill)
        .height(Length::Fill);

    let image_layer: Element<'a, Message> =
        if let Some((from_path, from_handle, progress)) = fade_from {
            // Old image on top fading out, new image underneath at full opacity.
            // This way the fade-out starts immediately even if the new image
            // hasn't loaded yet (it just reveals the dark background).
            let old_img = viewer_image(from_path, from_handle)
                .content_fit(iced::ContentFit::Contain)
                .width(Length::Fill)
                .height(Length::Fill)
                .opacity(1.0 - progress);

            iced::widget::stack![
                container(new_img).center(Length::Fill),
                container(old_img).center(Length::Fill),
            ]
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
        } else {
            container(new_img).center(Length::Fill).into()
        };

    let filename = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();

    let info_label = if meta.is_some() { "Info \u{2190}" } else { "Info \u{2192}" };
    let toolbar = row![
        button("Back").on_press(Message::BackToGrid),
        button(info_label).on_press(Message::ToggleInfo),
        Space::new().width(Length::Fill),
        text(format!("{} ({}/{})", filename, index + 1, total)).size(14),
    ]
    .spacing(10)
    .padding(10);

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

    let body: Element<'_, Message> = if let Some(m) = meta {
        let panel = info_panel(m);
        row![panel, image_with_nav].into()
    } else {
        image_with_nav.into()
    };

    column![toolbar, body,].into()
}

fn open_image_oriented(path: &std::path::Path) -> Option<::image::RgbaImage> {
    let img = ::image::open(path).ok()?;

    let orientation = (|| -> Option<u32> {
        let file = std::fs::File::open(path).ok()?;
        let mut reader = std::io::BufReader::new(file);
        let exif = exif::Reader::new().read_from_container(&mut reader).ok()?;
        exif.get_field(exif::Tag::Orientation, exif::In::PRIMARY)?
            .value
            .get_uint(0)
    })()
    .unwrap_or(1);

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

fn viewer_image(
    path: &PathBuf,
    handle: Option<&image::Handle>,
) -> iced::widget::Image<image::Handle> {
    match handle {
        Some(h) => image(h.clone()),
        None => image(path.to_string_lossy().to_string()),
    }
}

const LABEL_COLOR: Color = Color::from_rgb(0.5, 0.5, 0.55);

fn info_panel(meta: &PhotoMetadata) -> Element<'_, Message> {
    let mut items: Vec<Element<'_, Message>> = Vec::new();

    // File header
    items.push(text(&meta.filename).size(15).into());
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

    let panel_content = scrollable(column(items).spacing(6).padding(16)).height(Length::Fill);

    row![
        container(panel_content)
            .width(280)
            .height(Length::Fill)
            .style(container::dark),
        rule::vertical(1),
    ]
    .into()
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
