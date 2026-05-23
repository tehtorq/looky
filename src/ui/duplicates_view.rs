use iced::widget::{button, column, container, image, row, scrollable, text, Space};
use iced::{Color, Element, Length, Theme};

use crate::app::{Looky, Message};
use crate::duplicates::{DuplicateGroup, MatchKind};
use crate::metadata;
use crate::ui::info_panel::LABEL_COLOR;

pub fn dup_list_scroll_id() -> iced::widget::Id {
    iced::widget::Id::new("dup-list")
}

pub fn dup_badge_style(theme: &Theme) -> container::Style {
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

pub fn duplicates_list_view(state: &Looky) -> Element<'_, Message> {
    let cards: Vec<Element<'_, Message>> = state
        .dup_groups
        .iter()
        .enumerate()
        .map(|(group_idx, group)| {
            let (label, label_color) = match &group.match_kind {
                MatchKind::Exact => ("Exact match", Color::from_rgb(0.9, 0.2, 0.2)),
                MatchKind::Visual { .. } => ("Visual match", Color::from_rgb(0.9, 0.7, 0.1)),
            };

            let match_detail = match &group.match_kind {
                MatchKind::Exact => format!("{} identical files", group.indices.len()),
                MatchKind::Visual { distance } => {
                    format!("{} similar files (distance: {})", group.indices.len(), distance)
                }
            };

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
                scrollable(row(thumb_row).spacing(8)).direction(
                    scrollable::Direction::Horizontal(scrollable::Scrollbar::default()),
                ),
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

    container(list).into()
}

pub fn duplicates_compare_view<'a>(
    state: &'a Looky,
    group: &'a DuplicateGroup,
) -> Element<'a, Message> {
    let images: Vec<Element<'_, Message>> = group
        .indices
        .iter()
        .filter_map(|&idx| {
            let path = state.image_paths.get(idx)?;
            let info = state.dup_summaries.get(&idx);

            let filename = info
                .map(|s| s.filename.clone())
                .or_else(|| path.file_name().map(|n| n.to_string_lossy().to_string()))
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

    container(compare_row).into()
}
