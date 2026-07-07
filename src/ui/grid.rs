use std::collections::HashSet;
use std::path::PathBuf;

use iced::widget::{button, column, container, image, row, text, Space};
use iced::{Color, Element, Length, Task, Theme};

use crate::app::{Looky, Message};
use crate::ui::duplicates_view;

pub const THUMB_SIZE: f32 = 200.0;
pub const THUMB_CELL: f32 = THUMB_SIZE;
pub const GRID_PADDING: f32 = 0.0;
pub const THUMB_FADE_MS: f32 = 300.0;

pub fn grid_scroll_id() -> iced::widget::Id {
    iced::widget::Id::new("grid")
}

pub fn move_grid_selection(state: &mut Looky, delta: i32) -> Task<Message> {
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

    let viewport = state.viewport_height;
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

pub fn restore_grid_scroll(state: &Looky) -> Task<Message> {
    use iced::widget::operation::AbsoluteOffset;
    let offset = AbsoluteOffset {
        x: None,
        y: Some(state.grid_scroll_y),
    };
    iced::widget::operation::scroll_to(grid_scroll_id(), offset)
}

fn visible_index_range(state: &Looky) -> std::ops::Range<usize> {
    let cols = state.grid_columns.max(1);
    let first_row = (state.grid_scroll_y / THUMB_CELL).floor().max(0.0) as usize;
    let visible_rows = (state.viewport_height / THUMB_CELL).ceil() as usize + 1;
    // Clamp both ends: a stale scroll offset combined with a fresh column
    // count can otherwise produce first > last and panic the slice.
    let last_idx = ((first_row + visible_rows) * cols).min(state.thumbnails.len());
    let first_idx = (first_row * cols).min(last_idx);
    first_idx..last_idx
}

pub fn prioritize_upgrades(state: &mut Looky) {
    if state.pending_upgrades.is_empty() {
        return;
    }
    let visible = visible_index_range(state);
    let visible_paths: HashSet<&PathBuf> = state.thumbnails[visible]
        .iter()
        .map(|(p, _, _)| p)
        .collect();
    state
        .pending_upgrades
        .sort_by_key(|p| if visible_paths.contains(p) { 0 } else { 1 });
}

pub fn thumbnail_grid(state: &Looky) -> Element<'_, Message> {
    let thumbnails = &state.thumbnails;
    let badge_set = &state.dup_badge_set;
    let selected = state.selected_thumb;
    let scroll_y = state.grid_scroll_y;
    let viewport_h = state.viewport_height;
    let loading = state.loading;

    iced::widget::responsive(move |size| {
        let available = size.width - GRID_PADDING * 2.0;
        let thumbs_per_row = (available / THUMB_CELL).max(1.0) as usize;
        let total_rows = (thumbnails.len() + thumbs_per_row - 1) / thumbs_per_row;

        let first_visible_row = (scroll_y / THUMB_CELL).floor().max(0.0) as usize;
        let visible_row_count = (viewport_h / THUMB_CELL).ceil() as usize + 2;
        let first_row = first_visible_row.saturating_sub(1);
        let last_row = (first_row + visible_row_count + 1).min(total_rows);

        let mut items: Vec<Element<Message>> = Vec::new();

        if first_row > 0 {
            let spacer_height = first_row as f32 * THUMB_CELL;
            items.push(
                Space::new()
                    .width(Length::Fill)
                    .height(spacer_height)
                    .into(),
            );
        }

        for row_idx in first_row..last_row {
            let start = row_idx * thumbs_per_row;
            let end = (start + thumbs_per_row).min(thumbnails.len());
            if start >= thumbnails.len() {
                break;
            }

            let row_items: Vec<Element<Message>> = (start..end)
                .map(|index| {
                    let (_path, handle, added) = &thumbnails[index];
                    let opacity = if loading {
                        1.0
                    } else {
                        let age_ms = added.elapsed().as_secs_f32() * 1000.0;
                        (age_ms / THUMB_FADE_MS).min(1.0)
                    };
                    let img = image(handle.clone())
                        .width(THUMB_SIZE)
                        .height(THUMB_SIZE)
                        .content_fit(iced::ContentFit::Cover)
                        .opacity(opacity);

                    let thumb_content: Element<'_, Message> = if badge_set.contains(&index) {
                        iced::widget::stack![
                            img,
                            container(
                                container(text("DUP").size(11).color(Color::WHITE))
                                    .padding([2, 6])
                                    .style(duplicates_view::dup_badge_style),
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
