use iced::widget::{container, scrollable, text, Space};
use iced::{Element, Length};

use crate::app::{Looky, Message};
use crate::key_listener::KeyListener;
use crate::ui::{duplicates_view, grid, menu, viewer_view};

pub fn view(state: &Looky) -> Element<'_, Message> {
    let content = view_inner(state);
    let in_viewer = state.viewer.current_index.is_some();
    let screensaver = state.screensaver_active;
    let menu_open = state.menu_open;
    KeyListener::new(content, move |key, repeat| {
        use iced::keyboard::key::Named;
        use iced::keyboard::Key;
        if screensaver {
            return match &key {
                Key::Named(Named::Escape) if !repeat => Some(Message::KeyEscape),
                _ => None,
            };
        }
        match &key {
            Key::Named(Named::ArrowLeft) => Some(Message::KeyLeft),
            Key::Named(Named::ArrowRight) => Some(Message::KeyRight),
            Key::Named(Named::ArrowUp) => Some(Message::KeyUp),
            Key::Named(Named::ArrowDown) => Some(Message::KeyDown),
            Key::Character(c) if matches!(c.as_str(), "a" | "w" | "d" | "s") => {
                match c.as_str() {
                    "a" => Some(Message::KeyLeft),
                    "d" => Some(Message::KeyRight),
                    "w" => Some(Message::KeyUp),
                    "s" => Some(Message::KeyDown),
                    _ => None,
                }
            }
            _ if repeat => None,
            Key::Named(Named::Space) => Some(Message::ToggleZoom),
            Key::Named(Named::Enter) => Some(Message::KeyEnter),
            Key::Named(Named::Escape) => {
                if menu_open { Some(Message::ToggleMenu) } else { Some(Message::KeyEscape) }
            }
            Key::Character(c) if c.as_str() == "i" => {
                if repeat { return None; }
                Some(Message::ToggleInfo)
            }
            Key::Character(c) if c.as_str() == "f" => {
                if repeat { return None; }
                Some(Message::ToggleFullscreen)
            }
            Key::Character(c) if c.as_str() == "c" => {
                if repeat { return None; }
                Some(Message::CastImage)
            }
            _ => None,
        }
    })
    .on_scroll(move |delta, cx, cy| {
        if screensaver { return None; }
        if in_viewer { Some(Message::ZoomAdjust(delta, cx, cy)) } else { None }
    })
    .on_drag(move |dx, dy| {
        if screensaver { return None; }
        if in_viewer { Some(Message::ViewerDrag(dx, dy)) } else { Some(Message::DragScroll(dx, dy)) }
    })
    .on_click(move |cx, cy| {
        if screensaver { return None; }
        if in_viewer { Some(Message::ViewerClickZoom(cx, cy)) } else { None }
    })
    .on_right_click(move |cx, cy| {
        if screensaver { return None; }
        if in_viewer { Some(Message::ViewerClickUnzoom(cx, cy)) } else { None }
    })
    .on_pinch(move |scale, cx, cy| {
        if screensaver { return None; }
        if in_viewer { Some(Message::PinchZoom(scale, cx, cy)) } else { None }
    })
    .into()
}

fn build_viewer(state: &Looky, index: usize, screensaver: bool) -> Element<'_, Message> {
    let full_handle = state.viewer_cache.get(&index);
    let thumb_handle = state.thumbnails.get(index).map(|(_, h, _)| h);
    viewer_view::viewer_view(
        thumb_handle,
        full_handle,
        index > 0,
        index + 1 < state.image_paths.len(),
        state.cached_metadata.as_ref().map(|(_, m)| m),
        state.viewer.show_info,
        state.viewer.zoom_level,
        state.viewer_dimensions.get(&index).copied(),
        state.viewport_width,
        state.viewport_height,
        screensaver,
    )
}

fn view_inner(state: &Looky) -> Element<'_, Message> {
    if state.screensaver_active {
        if let Some(index) = state.viewer.current_index {
            if state.image_paths.get(index).is_some() {
                return build_viewer(state, index, true);
            }
        }
    }

    let content: Element<'_, Message> = if let Some(index) = state.viewer.current_index {
        if state.image_paths.get(index).is_some() {
            build_viewer(state, index, false)
        } else {
            container(Space::new()).into()
        }
    } else if let Some(group_idx) = state.dup_compare {
        if let Some(group) = state.dup_groups.get(group_idx) {
            duplicates_view::duplicates_compare_view(state, group)
        } else {
            container(Space::new()).into()
        }
    } else if state.dup_view_active {
        duplicates_view::duplicates_list_view(state)
    } else if state.loading && state.thumbnails.is_empty() {
        container(text("Loading...")).center(Length::Fill).into()
    } else if !state.loading && state.thumbnails.is_empty() {
        container(text("Open a folder to browse photos"))
            .center(Length::Fill)
            .into()
    } else {
        scrollable(grid::thumbnail_grid(state))
            .id(grid::grid_scroll_id())
            .on_scroll(|vp| Message::GridScrolled(vp.absolute_offset().y))
            .height(Length::Fill)
            .into()
    };

    let layers: Vec<Element<'_, Message>> = vec![content, menu::menu_overlay(state)];
    iced::widget::Stack::with_children(layers)
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}
