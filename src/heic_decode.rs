use std::path::Path;

use image::DynamicImage;

pub fn is_heic(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|e| e.to_str()).map(|e| e.to_lowercase()).as_deref(),
        Some("heic" | "heif")
    )
}

pub fn open_heic(path: &Path) -> Option<DynamicImage> {
    let data = std::fs::read(path).ok()?;
    let output = heic::DecoderConfig::new()
        .decode(&data, heic::PixelLayout::Rgba8)
        .ok()?;
    let img = image::RgbaImage::from_raw(output.width, output.height, output.data)?;
    Some(DynamicImage::ImageRgba8(img))
}
