use iced::widget::{column, container, rule, row, text};
use iced::{Color, Element, Theme};

use crate::app::Message;
use crate::metadata::{self, PhotoMetadata};

pub const LABEL_COLOR: Color = Color::from_rgb(0.5, 0.5, 0.55);

pub fn info_panel(meta: &PhotoMetadata) -> Element<'_, Message> {
    let mut items: Vec<Element<'_, Message>> = Vec::new();

    // File header
    items.push(
        text(&meta.filename)
            .size(15)
            .wrapping(text::Wrapping::WordOrGlyph)
            .into(),
    );
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
    let has_credits =
        meta.artist.is_some() || meta.copyright.is_some() || meta.description.is_some();
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

    let panel_content = column(items).spacing(6).padding(16).width(280);

    container(
        container(panel_content)
            .width(280)
            .clip(true)
            .style(info_panel_style),
    )
    .padding(12)
    .into()
}

fn info_panel_style(_theme: &Theme) -> container::Style {
    container::Style {
        background: Some(iced::Background::Color(Color::from_rgba(0.1, 0.1, 0.1, 0.85))),
        border: iced::Border {
            radius: 8.0.into(),
            ..Default::default()
        },
        ..Default::default()
    }
}

pub fn section_header(label: &str) -> Element<'_, Message> {
    text(label.to_string())
        .size(11)
        .color(LABEL_COLOR)
        .into()
}

pub fn section_divider<'a>() -> Element<'a, Message> {
    container(rule::horizontal(1))
        .padding([4, 0])
        .into()
}

pub fn info_field(label: &str, value: String) -> Element<'_, Message> {
    row![
        text(label.to_string()).size(12).color(LABEL_COLOR).width(90),
        text(value).size(12),
    ]
    .spacing(8)
    .into()
}
