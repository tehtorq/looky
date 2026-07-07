use std::path::PathBuf;

use iced::Task;
use rand::Rng;

use crate::app::{Looky, Message, ScatteredCard};
use crate::fs_scan;
use crate::metadata;
use crate::tasks;
use crate::ui::grid;
use crate::ui::viewer_view;
use crate::update::dup_update;
use crate::viewer::ViewerState;

const SCREENSAVER_MAX_CARDS: usize = 20;

pub fn folder_selected(state: &mut Looky, path: PathBuf) -> Task<Message> {
    fs_scan::save_last_folder(&path);
    // Invalidate every in-flight async result from the previous folder —
    // indices and paths are about to mean something else entirely.
    state.scan_generation += 1;
    state.dup_generation += 1;
    if let Some(session) = state.cast_session.take() {
        session.stop();
    }
    state.cast_target_name = None;
    state.cast_devices.clear();
    state.cast_error = None;
    if let Some(handle) = state.server_handle.take() {
        std::thread::spawn(move || handle.stop());
    }
    state.server_url = None;
    state.qr_handle = None;
    state.folder = Some(path.clone());
    state.thumbnails.clear();
    state.last_thumb_added = None;
    state.image_paths.clear();
    state.pending_thumbnails.clear();
    state.thumbnail_index.clear();
    state.pending_upgrades.clear();
    state.upgrade_batches_in_flight = 0;
    state.viewer = ViewerState::default();
    state.loading = true;
    for (_, handle) in state.viewer_preload_handles.drain(..) {
        handle.abort();
    }
    state.viewer_cache.clear();
    state.viewer_dimensions.clear();
    state.cached_metadata = None;
    state.selected_thumb = None;
    state.grid_scroll_y = 0.0;
    state.dup_scroll_y = 0.0;
    state.dup_hashes.clear();
    state.dup_pending.clear();
    state.dup_scanning = false;
    state.dup_total = 0;
    state.dup_groups.clear();
    state.dup_badge_set.clear();
    state.dup_view_active = false;
    state.dup_compare = None;
    state.dup_summaries.clear();
    let generation = state.scan_generation;
    Task::perform(
        tasks::run_blocking(move || fs_scan::scan_folder(path)),
        move |paths| Message::ImagesFound(generation, paths),
    )
}

pub fn images_found(state: &mut Looky, paths: Vec<PathBuf>) -> Task<Message> {
    if let (Some(cat), Some(folder)) = (state.catalog.as_mut(), state.folder.clone()) {
        let present: std::collections::HashSet<PathBuf> = paths.iter().cloned().collect();
        cat.prune_stale(&folder, &present);
    }
    state.image_paths = paths.clone();
    state.pending_thumbnails = paths;
    if state.image_paths.is_empty() {
        state.loading = false;
        return Task::none();
    }
    if let Some(dup_task) = dup_update::images_found_dup_bootstrap(state) {
        return Task::batch([tasks::load_next_preview_batch(state), dup_task]);
    }
    tasks::load_next_preview_batch(state)
}

pub fn viewer_image_loaded(
    state: &mut Looky,
    index: usize,
    rgba: Vec<u8>,
    width: u32,
    height: u32,
) -> Task<Message> {
    log::debug!("viewer: [{}] loaded ({}x{})", index, width, height);
    let handle = iced::widget::image::Handle::from_rgba(width, height, rgba);
    state.viewer_cache.insert(index, handle);
    state.viewer_dimensions.insert(index, (width, height));
    if let Some(current) = state.viewer.current_index {
        let keep_min = current.saturating_sub(tasks::PRELOAD_RADIUS);
        let keep_max = current + tasks::PRELOAD_RADIUS;
        state.viewer_cache.retain(|&k, _| k >= keep_min && k <= keep_max);
        state.viewer_dimensions.retain(|&k, _| k >= keep_min && k <= keep_max);
        if index == current {
            return tasks::preload_viewer_neighbors(state);
        }
    }
    Task::none()
}

pub fn escape(state: &mut Looky) -> Task<Message> {
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
        state.viewer_cache.clear();
        state.viewer_dimensions.clear();
        return grid::restore_grid_scroll(state);
    } else if state.dup_compare.is_some() {
        state.dup_compare = None;
    } else if state.dup_view_active {
        state.dup_view_active = false;
    } else if state.selected_thumb.is_some() {
        state.selected_thumb = None;
    } else {
        return iced::window::latest().and_then(iced::window::close);
    }
    Task::none()
}

pub fn arrow(state: &mut Looky, dx: i32, dy: i32) -> Task<Message> {
    if state.viewer.current_index.is_some() && state.viewer.is_zoomed() {
        return viewer_view::pan_zoom(state, dx as f32 * 30.0, dy as f32 * 30.0);
    }
    if dy == 0 && state.viewer.current_index.is_some() {
        if dx < 0 { state.viewer.prev(); } else { state.viewer.next(state.image_paths.len()); }
        state.selected_thumb = state.viewer.current_index;
        return Task::batch([
            refresh_metadata(state),
            tasks::preload_viewer_images(state),
        ]);
    }
    if !state.dup_view_active && state.dup_compare.is_none() && state.viewer.current_index.is_none() {
        let delta = if dy != 0 { dy * state.grid_columns.max(1) as i32 } else { dx };
        return grid::move_grid_selection(state, delta);
    }
    Task::none()
}

pub fn toggle_screensaver(state: &mut Looky) -> Task<Message> {
    if state.screensaver_active {
        state.screensaver_active = false;
        state.screensaver_cards.clear();
        state.viewer.close();
        state.cached_metadata = None;
        if !state.was_fullscreen {
            state.fullscreen = false;
            return iced::window::latest()
                .and_then(|id| iced::window::set_mode(id, iced::window::Mode::Windowed));
        }
        return Task::none();
    }
    if state.image_paths.is_empty() {
        return Task::none();
    }
    state.was_fullscreen = state.fullscreen;
    state.screensaver_active = true;
    let mut order: Vec<usize> = (0..state.image_paths.len()).collect();
    use rand::seq::SliceRandom;
    order.shuffle(&mut rand::rng());
    state.screensaver_order = order;
    state.screensaver_position = 0;
    // Seed initial cards
    state.screensaver_cards.clear();
    let initial_count = SCREENSAVER_MAX_CARDS.min(state.thumbnails.len());
    for _ in 0..initial_count {
        add_screensaver_card(state);
    }
    if !state.fullscreen {
        state.fullscreen = true;
        return iced::window::latest()
            .and_then(|id| iced::window::set_mode(id, iced::window::Mode::Fullscreen));
    }
    Task::none()
}

pub fn screensaver_advance(state: &mut Looky) -> Task<Message> {
    if !state.screensaver_active || state.thumbnails.is_empty() {
        return Task::none();
    }
    add_screensaver_card(state);
    while state.screensaver_cards.len() > SCREENSAVER_MAX_CARDS {
        state.screensaver_cards.remove(0);
    }
    Task::none()
}

fn add_screensaver_card(state: &mut Looky) {
    let loaded = state.thumbnails.len();
    if loaded == 0 || state.screensaver_order.is_empty() {
        return;
    }
    if state.screensaver_position >= state.screensaver_order.len() {
        use rand::seq::SliceRandom;
        state.screensaver_order.shuffle(&mut rand::rng());
        state.screensaver_position = 0;
    }
    let idx = state.screensaver_order[state.screensaver_position] % loaded;
    state.screensaver_position += 1;

    let vw = state.viewport_width;
    let vh = state.viewport_height;
    let (x, y, size) = pick_placement(state, vw, vh);
    state.screensaver_cards.push(ScatteredCard { idx, x, y, size });
}

/// Divide the screen into a grid of cells. Pick the cell whose most recent
/// card is the oldest (or that has no card at all), then jitter within it.
fn pick_placement(state: &Looky, vw: f32, vh: f32) -> (f32, f32, f32) {
    let mut rng = rand::rng();

    // Grid dimensions — enough cells to tile the screen at roughly card-size
    let cols = 4_usize;
    let rows = 3_usize;
    let cell_w = vw / cols as f32;
    let cell_h = vh / rows as f32;

    // For each cell, find the index of the newest card that overlaps it.
    // "Newest" = highest index in screensaver_cards (they're in insertion order).
    let mut cell_freshness = vec![0_usize; cols * rows];
    for (card_age, card) in state.screensaver_cards.iter().enumerate() {
        let cx = ((card.x + card.size * 0.5) / cell_w).clamp(0.0, (cols - 1) as f32) as usize;
        let cy = ((card.y + card.size * 0.5) / cell_h).clamp(0.0, (rows - 1) as f32) as usize;
        let cell = cy * cols + cx;
        cell_freshness[cell] = cell_freshness[cell].max(card_age + 1);
    }

    // Pick the stalest cell (lowest freshness = least recently updated)
    let stalest_cell = cell_freshness
        .iter()
        .enumerate()
        .min_by_key(|(_, f)| **f)
        .map(|(i, _)| i)
        .unwrap_or(0);

    let cell_col = (stalest_cell % cols) as f32;
    let cell_row = (stalest_cell / cols) as f32;

    // Size: larger than the cell so neighbors always overlap (no black gaps)
    let base_size = cell_w.max(cell_h);
    let size = rng.random_range((base_size * 1.0)..(base_size * 1.3));

    // Position: center in cell with jitter
    let center_x = (cell_col + 0.5) * cell_w;
    let center_y = (cell_row + 0.5) * cell_h;
    let jitter_x = rng.random_range(-(cell_w * 0.15)..(cell_w * 0.15));
    let jitter_y = rng.random_range(-(cell_h * 0.15)..(cell_h * 0.15));
    let x = center_x - size * 0.5 + jitter_x;
    let y = center_y - size * 0.5 + jitter_y;

    (x, y, size)
}

/// Load metadata for the current photo off-thread (EXIF parse + header read
/// can hitch the UI on slow volumes). Result arrives as MetadataLoaded.
pub fn refresh_metadata(state: &mut Looky) -> Task<Message> {
    let Some(index) = state.viewer.current_index else {
        return Task::none();
    };
    if state.cached_metadata.as_ref().is_some_and(|(i, _)| *i == index) {
        return Task::none();
    }
    let Some(path) = state.image_paths.get(index).cloned() else {
        return Task::none();
    };
    Task::perform(
        tasks::run_blocking(move || metadata::read_metadata(&path)),
        move |meta| Message::MetadataLoaded(index, Box::new(meta)),
    )
}
