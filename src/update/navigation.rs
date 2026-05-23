use std::path::PathBuf;

use iced::Task;

use crate::app::{Looky, Message};
use crate::fs_scan;
use crate::metadata;
use crate::tasks;
use crate::ui::grid;
use crate::ui::viewer_view;
use crate::update::dup_update;
use crate::viewer::ViewerState;

pub fn folder_selected(state: &mut Looky, path: PathBuf) -> Task<Message> {
    fs_scan::save_last_folder(&path);
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
    state.image_paths.clear();
    state.pending_thumbnails.clear();
    state.thumbnail_index.clear();
    state.pending_upgrades.clear();
    state.upgrade_batches_in_flight = 0;
    state.viewer = ViewerState::default();
    state.loading = true;
    state.dup_hashes.clear();
    state.dup_pending.clear();
    state.dup_scanning = false;
    state.dup_groups.clear();
    state.dup_badge_set.clear();
    state.dup_view_active = false;
    state.dup_compare = None;
    state.dup_summaries.clear();
    Task::perform(fs_scan::scan_folder(path), Message::ImagesFound)
}

pub fn images_found(state: &mut Looky, paths: Vec<PathBuf>) -> Task<Message> {
    if let Some(cat) = state.catalog.as_ref() {
        cat.prune_missing();
    }
    state.image_paths = paths.clone();
    state.pending_thumbnails = paths;
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
        let keep_min = current.saturating_sub(3);
        let keep_max = current + 3;
        let ss_next = if state.screensaver_active {
            state.screensaver_order.get(state.screensaver_position + 1).copied()
        } else {
            None
        };
        state.viewer_cache.retain(|&k, _| (k >= keep_min && k <= keep_max) || ss_next == Some(k));
        state.viewer_dimensions.retain(|&k, _| (k >= keep_min && k <= keep_max) || ss_next == Some(k));
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
        refresh_metadata(state);
        return tasks::preload_viewer_images(state);
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
    let idx = state.screensaver_order[0];
    state.viewer.open_index(idx);
    refresh_metadata(state);
    let preload = tasks::preload_viewer_images(state);
    let preload_next = tasks::preload_next_screensaver_image(state);
    if !state.fullscreen {
        state.fullscreen = true;
        let fs = iced::window::latest()
            .and_then(|id| iced::window::set_mode(id, iced::window::Mode::Fullscreen));
        return Task::batch([preload, preload_next, fs]);
    }
    Task::batch([preload, preload_next])
}

pub fn screensaver_advance(state: &mut Looky) -> Task<Message> {
    if !state.screensaver_active {
        return Task::none();
    }
    state.screensaver_position += 1;
    if state.screensaver_position >= state.screensaver_order.len() {
        use rand::seq::SliceRandom;
        state.screensaver_order.shuffle(&mut rand::rng());
        state.screensaver_position = 0;
    }
    let idx = state.screensaver_order[state.screensaver_position];
    state.viewer.open_index(idx);
    state.viewer.reset_zoom();
    refresh_metadata(state);
    let preload = tasks::preload_viewer_images(state);
    let preload_next = tasks::preload_next_screensaver_image(state);
    Task::batch([preload, preload_next])
}

pub fn refresh_metadata(state: &mut Looky) {
    if let Some(index) = state.viewer.current_index {
        if state.cached_metadata.as_ref().is_some_and(|(i, _)| *i == index) {
            return;
        }
        if let Some(path) = state.image_paths.get(index) {
            state.cached_metadata = Some((index, metadata::read_metadata(path)));
        }
    }
}
