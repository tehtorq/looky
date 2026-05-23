use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use iced::widget::image;
use iced::{Subscription, Task, Theme};

use crate::catalog::Catalog;
use crate::duplicates::{DuplicateGroup, ImageHashes};
use crate::fs_scan;
use crate::metadata::{self, PhotoMetadata};
use crate::server;
use crate::tasks;
use crate::ui::duplicates_view;
use crate::ui::grid;
use crate::ui::view;
use crate::ui::viewer_view;
use crate::update::{dup_update, navigation as nav, sharing, zoom};
use crate::viewer::ViewerState;

fn boot() -> (Looky, Task<Message>) {
    let mut state = Looky::new();

    if let Some(dir) = fs_scan::config_dir() {
        let db_path = dir.join("catalog.db");
        match Catalog::open(&db_path) {
            Ok(cat) => state.catalog = Some(cat),
            Err(e) => log::warn!("Failed to open catalog DB: {}", e),
        }
    }

    if let Some(folder) = fs_scan::load_last_folder() {
        state.folder = Some(folder.clone());
        state.loading = true;
        let task = Task::perform(fs_scan::scan_folder(folder), Message::ImagesFound);
        return (state, task);
    }
    (state, Task::none())
}

pub fn run() -> iced::Result {
    iced::application(boot, update, view::view)
        .title("Looky")
        .theme(theme)
        .subscription(subscription)
        .centered()
        .run()
}

#[derive(Default)]
pub(crate) struct Looky {
    pub(crate) folder: Option<PathBuf>,
    pub(crate) image_paths: Vec<PathBuf>,
    pub(crate) thumbnails: Vec<(PathBuf, image::Handle, Instant)>,
    pub(crate) pending_thumbnails: Vec<PathBuf>,
    pub(crate) thumbnail_index: HashMap<PathBuf, usize>,
    pub(crate) pending_upgrades: Vec<PathBuf>,
    pub(crate) upgrade_batches_in_flight: usize,
    pub(crate) viewer: ViewerState,
    pub(crate) loading: bool,
    pub(crate) cached_metadata: Option<(usize, PhotoMetadata)>,
    pub(crate) catalog: Option<Catalog>,
    pub(crate) dup_hashes: Vec<(usize, ImageHashes)>,
    pub(crate) dup_pending: Vec<(usize, PathBuf)>,
    pub(crate) dup_scanning: bool,
    pub(crate) dup_total: usize,
    pub(crate) dup_groups: Vec<DuplicateGroup>,
    pub(crate) dup_badge_set: HashSet<usize>,
    pub(crate) dup_view_active: bool,
    pub(crate) dup_compare: Option<usize>,
    pub(crate) dup_summaries: HashMap<usize, metadata::FileSummary>,
    pub(crate) grid_scroll_y: f32,
    pub(crate) dup_scroll_y: f32,
    pub(crate) grid_columns: usize,
    pub(crate) viewport_width: f32,
    pub(crate) viewport_height: f32,
    pub(crate) selected_thumb: Option<usize>,
    pub(crate) viewer_cache: HashMap<usize, image::Handle>,
    pub(crate) viewer_dimensions: HashMap<usize, (u32, u32)>,
    pub(crate) viewer_preload_handles: Vec<(usize, iced::task::Handle)>,
    pub(crate) fullscreen: bool,
    pub(crate) screensaver_active: bool,
    pub(crate) screensaver_order: Vec<usize>,
    pub(crate) screensaver_position: usize,
    pub(crate) was_fullscreen: bool,
    pub(crate) server_handle: Option<server::ServerHandle>,
    pub(crate) server_url: Option<String>,
    pub(crate) qr_handle: Option<image::Handle>,
    pub(crate) cast_session: Option<server::cast::CastSession>,
    pub(crate) cast_target_name: Option<String>,
    pub(crate) cast_scanning: bool,
    pub(crate) cast_devices: Vec<server::cast::CastTarget>,
    pub(crate) cast_error: Option<String>,
    pub(crate) menu_open: bool,
}

impl Looky {
    fn new() -> Self {
        Self {
            grid_columns: 4,
            viewport_width: 800.0,
            viewport_height: 600.0,
            ..Default::default()
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
    FindDuplicates,
    CancelDupScan,
    DupHashBatchReady(Vec<(usize, Option<ImageHashes>)>),
    DupAnalysisReady(Vec<DuplicateGroup>, HashMap<usize, metadata::FileSummary>),
    CachedDupAnalysisReady(Vec<DuplicateGroup>, HashMap<usize, metadata::FileSummary>),
    ShowDuplicatesView,
    BackFromDuplicates,
    CompareDuplicates(usize),
    BackFromCompare,
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
    ToggleScreensaver,
    ScreensaverAdvance,
    ToggleSharing,
    StartCastScan,
    CastDevicesFound(Vec<server::cast::CastTarget>),
    CastSelect(usize),
    CastConnected(server::cast::CastSession),
    CastImage,
    StopCast,
    GridScrolled(f32),
    WindowResized(f32, f32),
    KeyEscape,
    KeyLeft,
    KeyRight,
    KeyUp,
    KeyDown,
    KeyEnter,
    ToggleFullscreen,
    ToggleMenu,
}

fn subscription(state: &Looky) -> Subscription<Message> {
    let events = iced::event::listen_with(|event, _status, _window| match event {
        iced::Event::Window(iced::window::Event::Resized(size)) => {
            Some(Message::WindowResized(size.width, size.height))
        }
        _ => None,
    });

    let needs_tick = state.viewer.is_transitioning()
        || state.viewer.is_zoom_animating()
        || (!state.loading && thumbnails_fading(state));

    let mut subs = vec![events];
    if needs_tick {
        subs.push(iced::time::every(Duration::from_millis(16)).map(|_| Message::Tick));
    }
    if state.screensaver_active {
        subs.push(iced::time::every(Duration::from_secs(10)).map(|_| Message::ScreensaverAdvance));
    }
    Subscription::batch(subs)
}

fn thumbnails_fading(state: &Looky) -> bool {
    state.thumbnails.last()
        .is_some_and(|(_, _, added)| added.elapsed().as_secs_f32() * 1000.0 < grid::THUMB_FADE_MS)
}

fn update(state: &mut Looky, message: Message) -> Task<Message> {
    if state.menu_open {
        let close = matches!(
            message,
            Message::OpenFolder | Message::ShowDuplicatesView | Message::ToggleScreensaver
                | Message::BackToGrid | Message::ToggleInfo | Message::ToggleFullscreen
                | Message::BackFromDuplicates | Message::BackFromCompare
        );
        if close { state.menu_open = false; }
    }
    match message {
        Message::OpenFolder => return Task::perform(fs_scan::pick_folder(), Message::FolderSelected),
        Message::FolderSelected(Some(path)) => return nav::folder_selected(state, path),
        Message::FolderSelected(None) => {}
        Message::ImagesFound(paths) => return nav::images_found(state, paths),
        Message::ThumbnailBatchReady(results) => {
            let now = Instant::now();
            for (path, rgba, w, h) in results {
                state.thumbnails.push((path, image::Handle::from_rgba(w, h, rgba), now));
            }
            return tasks::load_next_batch(state);
        }
        Message::PreviewBatchReady(results) => {
            let now = Instant::now();
            for (path, preview) in results {
                let idx = state.thumbnails.len();
                state.thumbnail_index.insert(path.clone(), idx);
                let handle = match preview {
                    Some((rgba, w, h)) => image::Handle::from_rgba(w, h, rgba),
                    None => image::Handle::from_rgba(1, 1, vec![60, 60, 60, 255]),
                };
                state.thumbnails.push((path.clone(), handle, now));
                state.pending_upgrades.push(path);
            }
            return Task::batch([
                tasks::load_next_preview_batch(state),
                tasks::load_upgrade_batches(state),
            ]);
        }
        Message::ThumbnailUpgradeReady(results) => {
            state.upgrade_batches_in_flight = state.upgrade_batches_in_flight.saturating_sub(1);
            let now = Instant::now();
            for (path, rgba, w, h) in results {
                let handle = image::Handle::from_rgba(w, h, rgba);
                if let Some(&idx) = state.thumbnail_index.get(&path) {
                    if idx < state.thumbnails.len() {
                        state.thumbnails[idx] = (path, handle, now);
                    }
                }
            }
            if state.pending_upgrades.is_empty()
                && state.upgrade_batches_in_flight == 0
                && state.pending_thumbnails.is_empty()
            { state.loading = false; }
            return tasks::load_upgrade_batches(state);
        }
        Message::ViewImage(index) => {
            state.selected_thumb = Some(index);
            state.viewer.open_index(index);
            nav::refresh_metadata(state);
            return tasks::preload_viewer_images(state);
        }
        Message::NextImage => {
            state.viewer.next(state.image_paths.len());
            state.selected_thumb = state.viewer.current_index;
            nav::refresh_metadata(state);
            return tasks::preload_viewer_images(state);
        }
        Message::PrevImage => {
            state.viewer.prev();
            state.selected_thumb = state.viewer.current_index;
            nav::refresh_metadata(state);
            return tasks::preload_viewer_images(state);
        }
        Message::BackToGrid => {
            state.viewer.close();
            state.cached_metadata = None;
            state.viewer_cache.clear();
            state.viewer_dimensions.clear();
            return grid::restore_grid_scroll(state);
        }
        Message::ToggleInfo => state.viewer.toggle_info(),
        Message::ViewerImageLoaded(i, rgba, w, h) => return nav::viewer_image_loaded(state, i, rgba, w, h),
        Message::Tick => {
            state.viewer.tick();
            let old = state.viewer.zoom_level;
            let crossed = state.viewer.tick_zoom();
            let new = state.viewer.zoom_level;
            if crossed { return Task::done(Message::CenterZoomScroll); }
            if state.viewer.is_zoomed() && (new - old).abs() > 0.001 {
                return viewer_view::anchor_zoom_scroll(state, old, new);
            }
        }
        Message::FindDuplicates => return dup_update::find_duplicates(state),
        Message::CancelDupScan => dup_update::cancel_dup_scan(state),
        Message::DupHashBatchReady(r) => return dup_update::dup_hash_batch_ready(state, r),
        Message::DupAnalysisReady(g, s) => dup_update::dup_analysis_ready(state, g, s),
        Message::CachedDupAnalysisReady(g, s) => dup_update::cached_dup_analysis_ready(state, g, s),
        Message::ShowDuplicatesView => { state.dup_view_active = true; state.dup_compare = None; state.dup_scroll_y = 0.0; }
        Message::BackFromDuplicates => state.dup_view_active = false,
        Message::CompareDuplicates(i) => state.dup_compare = Some(i),
        Message::BackFromCompare => state.dup_compare = None,
        Message::ToggleZoom => return zoom::toggle_zoom(state),
        Message::CenterZoomScroll => return viewer_view::center_zoom_scroll(state),
        Message::ZoomAdjust(d, cx, cy) => return zoom::zoom_adjust(state, d, cx, cy),
        Message::ZoomScrolled(x, y) => state.viewer.zoom_offset = (x, y),
        Message::ViewerDrag(dx, dy) if state.viewer.is_zoomed() => return viewer_view::pan_zoom(state, -dx, -dy),
        Message::ViewerDrag(_, _) => {}
        Message::DragScroll(_dx, dy) => {
            let (id, scroll_y) = if state.dup_view_active {
                (duplicates_view::dup_list_scroll_id(), &mut state.dup_scroll_y)
            } else {
                (grid::grid_scroll_id(), &mut state.grid_scroll_y)
            };
            let new_y = (*scroll_y - dy).max(0.0);
            *scroll_y = new_y;
            return iced::widget::operation::scroll_to(id, iced::widget::operation::AbsoluteOffset { x: None, y: Some(new_y) });
        }
        Message::DupListScrolled(y) => state.dup_scroll_y = y,
        Message::ViewerClickZoom(cx, cy) => return zoom::click_zoom(state, cx, cy),
        Message::ViewerClickUnzoom(cx, cy) => return zoom::click_unzoom(state, cx, cy),
        Message::PinchZoom(s, cx, cy) => return zoom::pinch_zoom(state, s, cx, cy),
        Message::ToggleScreensaver => return nav::toggle_screensaver(state),
        Message::ScreensaverAdvance => return nav::screensaver_advance(state),
        Message::GridScrolled(y) => { state.grid_scroll_y = y; grid::prioritize_upgrades(state); }
        Message::WindowResized(w, h) => {
            state.grid_columns = ((w - grid::GRID_PADDING * 2.0) / grid::THUMB_CELL).max(1.0) as usize;
            state.viewport_width = w;
            state.viewport_height = h;
        }
        Message::KeyEscape => return nav::escape(state),
        Message::KeyLeft => return nav::arrow(state, -1, 0),
        Message::KeyRight => return nav::arrow(state, 1, 0),
        Message::KeyUp => return nav::arrow(state, 0, -1),
        Message::KeyDown => return nav::arrow(state, 0, 1),
        Message::KeyEnter => {
            if let Some(idx) = state.selected_thumb {
                if state.viewer.current_index.is_none()
                    && !state.dup_view_active && state.dup_compare.is_none()
                    && idx < state.thumbnails.len()
                {
                    state.viewer.open_index(idx);
                    nav::refresh_metadata(state);
                    return tasks::preload_viewer_images(state);
                }
            }
        }
        Message::ToggleFullscreen => {
            state.fullscreen = !state.fullscreen;
            let mode = if state.fullscreen { iced::window::Mode::Fullscreen } else { iced::window::Mode::Windowed };
            return iced::window::latest().and_then(move |id| iced::window::set_mode(id, mode));
        }
        Message::ToggleSharing => sharing::toggle_sharing(state),
        Message::StartCastScan => return sharing::start_cast_scan(state),
        Message::CastDevicesFound(d) => { state.cast_scanning = false; state.cast_devices = d; }
        Message::CastSelect(i) => return sharing::cast_select(state, i),
        Message::CastConnected(s) => { state.cast_target_name = Some(s.target.name.clone()); state.cast_session = Some(s); }
        Message::CastImage => sharing::cast_current_image(state),
        Message::StopCast => sharing::stop_cast(state),
        Message::ToggleMenu => state.menu_open = !state.menu_open,
    }
    Task::none()
}

fn theme(_state: &Looky) -> Theme {
    Theme::Dark
}
