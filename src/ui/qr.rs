use iced::widget::image;

pub fn render_qr(url: &str) -> image::Handle {
    use qrcode::QrCode;
    let code = QrCode::new(url.as_bytes()).unwrap();
    let modules = code.to_colors();
    let size = code.width();
    let scale = 4u32;
    let quiet = 2u32;
    let img_size = (size as u32) * scale + quiet * 2 * scale;
    let mut pixels = vec![255u8; (img_size * img_size * 4) as usize];
    for y in 0..size {
        for x in 0..size {
            let dark = modules[y * size + x] == qrcode::Color::Dark;
            if dark {
                let px = x as u32 * scale + quiet * scale;
                let py = y as u32 * scale + quiet * scale;
                for dy in 0..scale {
                    for dx in 0..scale {
                        let offset = ((py + dy) * img_size + (px + dx)) as usize * 4;
                        pixels[offset] = 0;
                        pixels[offset + 1] = 0;
                        pixels[offset + 2] = 0;
                        pixels[offset + 3] = 255;
                    }
                }
            }
        }
    }
    image::Handle::from_rgba(img_size, img_size, pixels)
}
