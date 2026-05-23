use iced::widget::{button, container, image, row, scrollable, text, Space};
use iced::{Color, Element, Length, Task, Theme};

use crate::app::{Looky, Message};
use crate::metadata::PhotoMetadata;
use crate::ui::info_panel;

pub fn viewer_scroll_id() -> iced::widget::Id {
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

pub fn center_zoom_scroll(state: &Looky) -> Task<Message> {
    let Some(idx) = state.viewer.current_index else {
        return Task::none();
    };
    let Some(&(img_w, img_h)) = state.viewer_dimensions.get(&idx) else {
        return Task::none();
    };

    let vp_w = state.viewport_width;
    let vp_h = state.viewport_height;
    let (fit_w, fit_h) = fit_size(img_w, img_h, vp_w, vp_h);
    let render_w = fit_w * state.viewer.zoom_level;
    let render_h = fit_h * state.viewer.zoom_level;
    let pad_x = zoom_padding(render_w, vp_w);
    let pad_y = zoom_padding(render_h, vp_h);

    if let Some((anchor_x, anchor_y)) = state.viewer.zoom_anchor {
        let rel_x = anchor_x;
        let rel_y = anchor_y;
        let img_x = rel_x - pad_x;
        let img_y = rel_y - pad_y;
        let scroll_x = (img_x + pad_x - rel_x).max(0.0);
        let scroll_y = (img_y + pad_y - rel_y).max(0.0);

        use iced::widget::operation::AbsoluteOffset;
        iced::widget::operation::scroll_to(
            viewer_scroll_id(),
            AbsoluteOffset {
                x: Some(scroll_x),
                y: Some(scroll_y),
            },
        )
    } else {
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
pub fn anchor_zoom_scroll(state: &mut Looky, old_zoom: f32, new_zoom: f32) -> Task<Message> {
    let Some(idx) = state.viewer.current_index else {
        return Task::none();
    };
    let Some(&(img_w, img_h)) = state.viewer_dimensions.get(&idx) else {
        return Task::none();
    };

    let vp_w = state.viewport_width;
    let vp_h = state.viewport_height;
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
        let rel_x = anchor_x;
        let rel_y = anchor_y;

        let content_x = scroll_x + rel_x;
        let content_y = scroll_y + rel_y;

        let img_x = content_x - old_pad_x;
        let img_y = content_y - old_pad_y;

        let ratio = new_zoom / old_zoom;
        let new_img_x = img_x * ratio;
        let new_img_y = img_y * ratio;

        let new_content_x = new_img_x + new_pad_x;
        let new_content_y = new_img_y + new_pad_y;

        let new_scroll_x = (new_content_x - rel_x).max(0.0);
        let new_scroll_y = (new_content_y - rel_y).max(0.0);

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

pub fn pan_zoom(state: &mut Looky, dx: f32, dy: f32) -> Task<Message> {
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

pub fn viewer_view<'a>(
    thumb_handle: Option<&'a image::Handle>,
    full_handle: Option<&'a image::Handle>,
    has_prev: bool,
    has_next: bool,
    meta: Option<&'a PhotoMetadata>,
    show_info: bool,
    zoom_level: f32,
    image_dims: Option<(u32, u32)>,
    viewport_width: f32,
    viewport_height: f32,
    screensaver: bool,
) -> Element<'a, Message> {
    if screensaver {
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

    if zoom_level > 1.0 {
        let handle = full_handle.or(thumb_handle);
        let image_layer: Element<'a, Message> = if let Some(h) = handle {
            let (img_w, img_h) = image_dims.unwrap_or((800, 600));
            let avail_w = viewport_width;
            let avail_h = viewport_height;
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
                layers.push(info_panel::info_panel(m));
            }
        }
        return iced::widget::Stack::with_children(layers)
            .width(Length::Fill)
            .height(Length::Fill)
            .into();
    }

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
        (None, None) => container(Space::new()).center(Length::Fill).into(),
    };

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
            layers.push(info_panel::info_panel(m));
        }
    }
    iced::widget::Stack::with_children(layers)
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}

fn screensaver_bg_style(_theme: &Theme) -> container::Style {
    container::Style {
        background: Some(iced::Background::Color(Color::BLACK)),
        ..Default::default()
    }
}
