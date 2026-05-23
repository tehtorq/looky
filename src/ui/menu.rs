use iced::widget::{button, column, container, image, rule, text};
use iced::{Color, Element, Length, Theme};

use crate::app::{Looky, Message};
use crate::duplicates::MatchKind;
use crate::ui::info_panel::LABEL_COLOR;

pub fn menu_overlay(state: &Looky) -> Element<'_, Message> {
    let mut items: Vec<Element<'_, Message>> = Vec::new();

    // Hamburger button (always shown)
    let hamburger = button(
        container(text("☰").size(18).line_height(1.0))
            .padding(iced::Padding {
                top: 3.0,
                right: 0.0,
                bottom: 0.0,
                left: 0.0,
            })
            .center(Length::Fill),
    )
    .width(40)
    .height(40)
    .padding(0)
    .on_press(Message::ToggleMenu)
    .style(hamburger_button_style);
    items.push(hamburger.into());

    if state.menu_open {
        let menu_items = build_menu_items(state);
        let menu = container(column(menu_items).spacing(4).padding(8))
            .style(menu_container_style)
            .max_width(220);
        items.push(menu.into());
    }

    container(column(items).spacing(4)).padding(12).into()
}

fn build_menu_items(state: &Looky) -> Vec<Element<'_, Message>> {
    if state.viewer.current_index.is_some() {
        viewer_menu_items(state)
    } else if state.dup_compare.is_some() {
        compare_menu_items(state)
    } else if state.dup_view_active {
        dup_list_menu_items(state)
    } else {
        grid_menu_items(state)
    }
}

fn grid_menu_items(state: &Looky) -> Vec<Element<'_, Message>> {
    let mut items: Vec<Element<'_, Message>> = Vec::new();

    items.push(menu_item("Open Folder", Message::OpenFolder));
    items.push(rule::horizontal(1).into());

    if !state.image_paths.is_empty() {
        if state.dup_scanning {
            let scanned = state.dup_total - state.dup_pending.len();
            items.push(menu_info(format!(
                "Scanning {} / {}...",
                scanned, state.dup_total
            )));
            items.push(menu_item("Cancel", Message::CancelDupScan));
        } else {
            items.push(menu_item("Find Duplicates", Message::FindDuplicates));
        }
    }

    if !state.dup_groups.is_empty() {
        items.push(
            button(
                text(format!("Duplicates ({})", state.dup_groups.len())).width(Length::Fill),
            )
            .on_press(Message::ShowDuplicatesView)
            .style(menu_item_style)
            .width(Length::Fill)
            .into(),
        );
    }

    if !state.image_paths.is_empty() {
        let ss_label = if state.screensaver_active {
            "Stop Screensaver"
        } else {
            "Screensaver"
        };
        items.push(menu_item(ss_label, Message::ToggleScreensaver));
    }

    items.push(rule::horizontal(1).into());

    if !state.image_paths.is_empty() {
        let share_label = if state.server_handle.is_some() {
            "Stop Sharing"
        } else {
            "Share"
        };
        items.push(menu_item(share_label, Message::ToggleSharing));
    }

    if state.server_handle.is_some() {
        if let Some(name) = &state.cast_target_name {
            items.push(menu_info(format!("TV: {name}")));
            items.push(menu_item("Stop Cast", Message::StopCast));
        } else if state.cast_scanning {
            items.push(menu_info("Scanning...".to_string()));
        } else if !state.cast_devices.is_empty() {
            for (i, dev) in state.cast_devices.iter().enumerate() {
                items.push(
                    button(text(dev.name.as_str()).width(Length::Fill))
                        .on_press(Message::CastSelect(i))
                        .style(menu_item_style)
                        .width(Length::Fill)
                        .into(),
                );
            }
        } else {
            items.push(menu_item("Cast to TV", Message::StartCastScan));
        }
        if let Some(err) = &state.cast_error {
            items.push(
                text(err.as_str())
                    .size(12)
                    .color(Color::from_rgb(0.9, 0.2, 0.2))
                    .into(),
            );
        }
    }

    items.push(rule::horizontal(1).into());

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
        items.push(menu_info(count_text));
    }

    if let (Some(url), Some(qr)) = (&state.server_url, &state.qr_handle) {
        items.push(
            text(url.as_str())
                .size(13)
                .color(LABEL_COLOR)
                .wrapping(text::Wrapping::WordOrGlyph)
                .into(),
        );
        items.push(image(qr.clone()).width(80).height(80).into());
    } else {
        items.push(
            text(match &state.folder {
                Some(p) => p.display().to_string(),
                None => "No folder selected".into(),
            })
            .size(13)
            .color(LABEL_COLOR)
            .wrapping(text::Wrapping::WordOrGlyph)
            .into(),
        );
    }

    items
}

fn viewer_menu_items(state: &Looky) -> Vec<Element<'_, Message>> {
    let mut items: Vec<Element<'_, Message>> = Vec::new();

    items.push(menu_item("Back", Message::BackToGrid));

    let info_label = if state.viewer.show_info {
        "Hide Info"
    } else {
        "Info"
    };
    items.push(menu_item(info_label, Message::ToggleInfo));

    let fs_label = if state.fullscreen {
        "Window"
    } else {
        "Fullscreen"
    };
    items.push(menu_item(fs_label, Message::ToggleFullscreen));

    items.push(rule::horizontal(1).into());

    if let Some(index) = state.viewer.current_index {
        if let Some(path) = state.image_paths.get(index) {
            let filename = path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            items.push(menu_info(filename));
            items.push(menu_info(format!(
                "{} / {}",
                index + 1,
                state.image_paths.len()
            )));
            if state.viewer.zoom_level > 1.0 {
                items.push(menu_info(format!(
                    "Zoom: {}%",
                    (state.viewer.zoom_level * 100.0) as u32
                )));
            }
        }
    }

    items
}

fn dup_list_menu_items(state: &Looky) -> Vec<Element<'_, Message>> {
    let mut items: Vec<Element<'_, Message>> = Vec::new();
    items.push(menu_item("Back", Message::BackFromDuplicates));
    items.push(rule::horizontal(1).into());
    items.push(menu_info(format!(
        "{} duplicate groups found",
        state.dup_groups.len()
    )));
    items
}

fn compare_menu_items(state: &Looky) -> Vec<Element<'_, Message>> {
    let mut items: Vec<Element<'_, Message>> = Vec::new();
    items.push(menu_item("Back", Message::BackFromCompare));
    items.push(rule::horizontal(1).into());

    if let Some(group_idx) = state.dup_compare {
        if let Some(group) = state.dup_groups.get(group_idx) {
            let (label, label_color) = match &group.match_kind {
                MatchKind::Exact => ("Exact match", Color::from_rgb(0.9, 0.2, 0.2)),
                MatchKind::Visual { .. } => ("Visual match", Color::from_rgb(0.9, 0.7, 0.1)),
            };
            items.push(text(label).size(13).color(label_color).into());
        }
    }

    items
}

fn menu_item(label: &str, msg: Message) -> Element<'_, Message> {
    button(text(label).width(Length::Fill))
        .on_press(msg)
        .style(menu_item_style)
        .width(Length::Fill)
        .into()
}

fn menu_info(content: impl Into<String>) -> Element<'static, Message> {
    text(content.into()).size(13).color(LABEL_COLOR).into()
}

fn hamburger_button_style(_theme: &Theme, status: button::Status) -> button::Style {
    let bg = match status {
        button::Status::Hovered => Color::from_rgba(0.25, 0.25, 0.25, 0.85),
        _ => Color::from_rgba(0.15, 0.15, 0.15, 0.85),
    };
    button::Style {
        background: Some(iced::Background::Color(bg)),
        text_color: Color::WHITE,
        border: iced::Border {
            radius: 20.0.into(),
            ..Default::default()
        },
        ..Default::default()
    }
}

fn menu_container_style(_theme: &Theme) -> container::Style {
    container::Style {
        background: Some(iced::Background::Color(Color::from_rgba(0.1, 0.1, 0.1, 0.85))),
        border: iced::Border {
            radius: 8.0.into(),
            ..Default::default()
        },
        ..Default::default()
    }
}

fn menu_item_style(_theme: &Theme, status: button::Status) -> button::Style {
    let bg = match status {
        button::Status::Hovered => {
            Some(iced::Background::Color(Color::from_rgba(1.0, 1.0, 1.0, 0.1)))
        }
        _ => None,
    };
    button::Style {
        background: bg,
        text_color: Color::WHITE,
        border: iced::Border::default(),
        ..Default::default()
    }
}
