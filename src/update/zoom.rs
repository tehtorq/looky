use iced::Task;

use crate::app::{Looky, Message};
use crate::metadata;
use crate::tasks;
use crate::ui::viewer_view;

pub fn toggle_zoom(state: &mut Looky) -> Task<Message> {
    if let Some(idx) = state.viewer.current_index {
        if !state.viewer_cache.contains_key(&idx) {
            return Task::none();
        }
        state.viewer.toggle_zoom();
    } else if let Some(idx) = state.selected_thumb {
        if !state.dup_view_active
            && state.dup_compare.is_none()
            && idx < state.thumbnails.len()
        {
            state.viewer.open_index(idx);
            refresh_metadata(state);
            return tasks::preload_viewer_images(state);
        }
    }
    Task::none()
}

pub fn zoom_adjust(state: &mut Looky, delta: f32, cursor_x: f32, cursor_y: f32) -> Task<Message> {
    let Some(idx) = state.viewer.current_index else {
        return Task::none();
    };
    if !state.viewer_cache.contains_key(&idx) {
        return Task::none();
    }
    state.viewer.zoom_anchor = Some((cursor_x, cursor_y));
    let old_zoom = state.viewer.zoom_level;
    state.viewer.adjust_zoom(delta);
    state.viewer.zoom_level = state.viewer.zoom_target;
    let new_zoom = state.viewer.zoom_level;
    if state.viewer.is_zoomed() && (new_zoom - old_zoom).abs() > 0.001 {
        return viewer_view::anchor_zoom_scroll(state, old_zoom, new_zoom);
    }
    Task::none()
}

pub fn click_zoom(state: &mut Looky, cx: f32, cy: f32) -> Task<Message> {
    let Some(idx) = state.viewer.current_index else {
        return Task::none();
    };
    if !state.viewer_cache.contains_key(&idx) {
        return Task::none();
    }
    state.viewer.zoom_anchor = Some((cx, cy));
    let old_zoom = state.viewer.zoom_level;
    state.viewer.adjust_zoom(4.0);
    let _crossed = state.viewer.tick_zoom();
    let new_zoom = state.viewer.zoom_level;
    if state.viewer.is_zoomed() && (new_zoom - old_zoom).abs() > 0.001 {
        return viewer_view::anchor_zoom_scroll(state, old_zoom, new_zoom);
    }
    Task::none()
}

pub fn click_unzoom(state: &mut Looky, cx: f32, cy: f32) -> Task<Message> {
    let Some(idx) = state.viewer.current_index else {
        return Task::none();
    };
    if !state.viewer_cache.contains_key(&idx) {
        return Task::none();
    }
    state.viewer.zoom_anchor = Some((cx, cy));
    let old_zoom = state.viewer.zoom_level;
    state.viewer.adjust_zoom(-4.0);
    let _ = state.viewer.tick_zoom();
    let new_zoom = state.viewer.zoom_level;
    if state.viewer.is_zoomed() && (new_zoom - old_zoom).abs() > 0.001 {
        return viewer_view::anchor_zoom_scroll(state, old_zoom, new_zoom);
    }
    Task::none()
}

pub fn pinch_zoom(state: &mut Looky, scale: f32, cx: f32, cy: f32) -> Task<Message> {
    let Some(idx) = state.viewer.current_index else {
        return Task::none();
    };
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
        return viewer_view::anchor_zoom_scroll(state, old_zoom, new_zoom);
    }
    if new_zoom <= 1.0 && old_zoom > 1.0 {
        state.viewer.zoom_offset = (0.0, 0.0);
    }
    Task::none()
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
