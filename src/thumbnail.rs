use std::io::{BufReader, Cursor};
use std::path::{Path, PathBuf};

use image::imageops::FilterType;
use image::{DynamicImage, GenericImageView};
use sha2::{Digest, Sha256};

/// Generate a thumbnail as RGBA bytes. Returns (rgba_bytes, width, height).
/// Checks disk cache first; on miss, generates and caches.
pub fn generate_thumbnail(path: &Path, max_size: u32) -> (Vec<u8>, u32, u32) {
    // Check disk cache (QOI format)
    let cache_key = cache_key(path, max_size);
    if let Some(key) = cache_key.as_ref() {
        // Try QOI cache first
        if let Some(cache_path) = cache_file_path(key) {
            if let Ok(data) = std::fs::read(&cache_path) {
                if let Ok((header, pixels)) = qoi::decode_to_vec(&data) {
                    touch(&cache_path);
                    return (pixels, header.width, header.height);
                }
            }
        }
        // Fallback: legacy JPEG cache — migrate to QOI and delete the old entry
        if let Some(legacy_path) = cache_file_path_legacy(key) {
            if let Ok(img) = image::open(&legacy_path) {
                let (w, h) = img.dimensions();
                let rgba = img.to_rgba8().into_raw();
                save_to_cache(key, &rgba, w, h);
                let _ = std::fs::remove_file(&legacy_path);
                return (rgba, w, h);
            }
        }
    }

    // Cache miss — generate thumbnail
    let (rgba, w, h) = generate_thumbnail_uncached(path, max_size);

    // Write to disk cache (best-effort, QOI format)
    if let Some(key) = cache_key {
        save_to_cache(&key, &rgba, w, h);
    }

    (rgba, w, h)
}

fn generate_thumbnail_uncached(path: &Path, max_size: u32) -> (Vec<u8>, u32, u32) {
    // The embedded EXIF thumbnail and DCT-scaled JPEG paths hand back raw
    // pixels, so the raw EXIF orientation applies — even for HEIC, whose
    // embedded thumbnail is stored unrotated. Only open_heic applies the
    // container transforms itself (see read_orientation).
    let (raw_orientation, exif_thumb) = read_exif_info(path);

    // Try embedded EXIF thumbnail first (fast — avoids full decode).
    // Only use it if it's large enough to avoid blurry upscaling.
    // Peek at JPEG header dimensions to skip full pixel decode for small thumbnails.
    if let Some(data) = exif_thumb {
        let large_enough = {
            let mut d = jpeg_decoder::Decoder::new(Cursor::new(&data));
            d.read_info()
                .ok()
                .and_then(|()| d.info())
                .is_some_and(|i| (i.width as u32).min(i.height as u32) >= max_size)
        };
        if large_enough {
            if let Ok(img) = image::load_from_memory(&data) {
                let thumb = img.resize(max_size, max_size, FilterType::Triangle);
                let thumb = apply_orientation(thumb, raw_orientation);
                let (w, h) = thumb.dimensions();
                return (thumb.to_rgba8().into_raw(), w, h);
            }
        }
    }

    // Try downscaled JPEG decode (avoids processing millions of unnecessary pixels)
    if let Some(img) = decode_jpeg_scaled(path, max_size) {
        let thumb = img.resize(max_size, max_size, FilterType::Triangle);
        let thumb = apply_orientation(thumb, raw_orientation);
        let (w, h) = thumb.dimensions();
        return (thumb.to_rgba8().into_raw(), w, h);
    }

    // Fallback: full decode + resize (try image crate, then HEIC decoder)
    let decoded = match image::open(path) {
        Ok(img) => Some((img, raw_orientation)),
        Err(_) => crate::heic_decode::open_heic(path).map(|img| (img, 1)),
    };
    match decoded {
        Some((img, orientation)) => {
            let thumb = img.resize(max_size, max_size, FilterType::Triangle);
            let thumb = apply_orientation(thumb, orientation);
            let (w, h) = thumb.dimensions();
            (thumb.to_rgba8().into_raw(), w, h)
        }
        None => {
            log::warn!("Failed to load image {}", path.display());
            placeholder_thumbnail(max_size)
        }
    }
}

// --- Downscaled JPEG decode ---

/// Decode a JPEG at reduced resolution using DCT scaling.
/// For a 4000x3000 image targeting 400px, decodes at ~500x375 instead of 12M pixels.
/// Returns None for non-JPEG files, small images, or on failure.
fn decode_jpeg_scaled(path: &Path, max_size: u32) -> Option<DynamicImage> {
    let ext = path.extension()?.to_str()?.to_lowercase();
    if ext != "jpg" && ext != "jpeg" {
        return None;
    }

    let file = std::fs::File::open(path).ok()?;
    let mut decoder = jpeg_decoder::Decoder::new(BufReader::new(file));

    // scale() reads the header internally and picks the optimal DCT scale factor.
    // It returns the actual output dimensions.
    let max_u16 = max_size as u16;
    let (actual_w, actual_h) = decoder.scale(max_u16, max_u16).ok()?;

    // Only beneficial if the decoder actually downscaled
    let info = decoder.info()?;
    if actual_w == info.width && actual_h == info.height {
        return None;
    }

    let pixels = decoder.decode().ok()?;

    match info.pixel_format {
        jpeg_decoder::PixelFormat::RGB24 => {
            image::RgbImage::from_raw(actual_w as u32, actual_h as u32, pixels)
                .map(DynamicImage::ImageRgb8)
        }
        jpeg_decoder::PixelFormat::L8 => {
            image::GrayImage::from_raw(actual_w as u32, actual_h as u32, pixels)
                .map(DynamicImage::ImageLuma8)
        }
        _ => None,
    }
}

// --- Disk cache ---

fn cache_dir() -> Option<PathBuf> {
    dirs_next::home_dir().map(|d| d.join(".looky").join("cache").join("thumbnails"))
}

/// Build a cache key from canonical path + file size + mtime + max_size.
fn cache_key(path: &Path, max_size: u32) -> Option<String> {
    let meta = std::fs::metadata(path).ok()?;
    let mtime = meta
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_nanos();
    let canonical = std::fs::canonicalize(path).ok()?;

    let mut hasher = Sha256::new();
    hasher.update(canonical.to_string_lossy().as_bytes());
    hasher.update(meta.len().to_le_bytes());
    hasher.update(mtime.to_le_bytes());
    hasher.update(max_size.to_le_bytes());
    let hash = hasher.finalize();
    Some(hex_encode(hash))
}

fn hex_encode(bytes: impl AsRef<[u8]>) -> String {
    bytes.as_ref().iter().map(|b| format!("{b:02x}")).collect()
}

fn cache_file_path(key: &str) -> Option<PathBuf> {
    // Use first 2 chars as subdirectory to avoid huge flat directories
    let dir = cache_dir()?.join(&key[..2]);
    Some(dir.join(format!("{}.qoi", key)))
}

/// Legacy JPEG cache path for migration fallback.
fn cache_file_path_legacy(key: &str) -> Option<PathBuf> {
    let dir = cache_dir()?.join(&key[..2]);
    Some(dir.join(format!("{}.jpg", key)))
}

/// Bump a cache file's mtime so the age-based prune treats it as in use.
fn touch(path: &Path) {
    if let Ok(file) = std::fs::File::options().append(true).open(path) {
        let _ = file.set_modified(std::time::SystemTime::now());
    }
}

/// Delete cache entries not touched within `max_age`. Cache keys include the
/// source file's mtime, so edited/re-synced photos strand their old entries
/// forever without this. Called from a background thread at startup.
pub fn prune_cache(max_age: std::time::Duration) {
    let Some(root) = cache_dir() else {
        return;
    };
    let now = std::time::SystemTime::now();
    let Ok(subdirs) = std::fs::read_dir(&root) else {
        return;
    };
    for subdir in subdirs.flatten() {
        let Ok(entries) = std::fs::read_dir(subdir.path()) else {
            continue;
        };
        for entry in entries.flatten() {
            let stale = entry
                .metadata()
                .and_then(|m| m.modified())
                .ok()
                .and_then(|mtime| now.duration_since(mtime).ok())
                .is_some_and(|age| age > max_age);
            if stale {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }
}

fn save_to_cache(key: &str, rgba: &[u8], width: u32, height: u32) {
    let Some(path) = cache_file_path(key) else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    // QOI encode is ~10x faster than JPEG and keeps RGBA directly
    if let Ok(data) = qoi::encode_to_vec(rgba, width, height) {
        let _ = std::fs::write(&path, data);
    }
}

// --- EXIF ---

/// Read just the EXIF orientation value.
/// Returns 1 (identity) for HEIC/HEIF — the heic crate applies container
/// transforms (irot/imir) during decode so EXIF orientation must not be
/// reapplied.
pub fn read_orientation(path: &Path) -> u32 {
    if crate::heic_decode::is_heic(path) {
        return 1;
    }
    read_exif_info(path).0
}

/// Single file open + EXIF parse: returns (orientation, optional embedded thumbnail JPEG bytes).
fn read_exif_info(path: &Path) -> (u32, Option<Vec<u8>>) {
    let Ok(file) = std::fs::File::open(path) else {
        return (1, None);
    };
    let mut reader = BufReader::new(file);
    let Ok(exif) = exif::Reader::new().read_from_container(&mut reader) else {
        return (1, None);
    };

    let orientation = exif
        .get_field(exif::Tag::Orientation, exif::In::PRIMARY)
        .and_then(|f| f.value.get_uint(0))
        .unwrap_or(1);

    let thumbnail = (|| {
        // JPEGInterchangeFormat offsets are relative to the TIFF header, i.e.
        // the EXIF buffer kamadak-exif already holds — not the file start.
        let offset = exif
            .get_field(exif::Tag::JPEGInterchangeFormat, exif::In::THUMBNAIL)?
            .value
            .get_uint(0)? as usize;
        let length = exif
            .get_field(exif::Tag::JPEGInterchangeFormatLength, exif::In::THUMBNAIL)?
            .value
            .get_uint(0)? as usize;
        if length == 0 || length > 1_000_000 {
            return None;
        }
        let data = exif.buf().get(offset..offset.checked_add(length)?)?;
        Some(data.to_vec())
    })();

    (orientation, thumbnail)
}

/// Apply EXIF orientation transform to an image.
pub fn apply_orientation(img: DynamicImage, orientation: u32) -> DynamicImage {
    match orientation {
        2 => img.fliph(),
        3 => img.rotate180(),
        4 => img.flipv(),
        5 => img.rotate90().fliph(),
        6 => img.rotate90(),
        7 => img.rotate270().fliph(),
        8 => img.rotate270(),
        _ => img, // 1 = normal, or unknown
    }
}


fn placeholder_thumbnail(size: u32) -> (Vec<u8>, u32, u32) {
    let pixels: Vec<u8> = [60, 60, 60, 255]
        .iter()
        .copied()
        .cycle()
        .take((size * size * 4) as usize)
        .collect();
    (pixels, size, size)
}

/// Generate a JPEG thumbnail as raw bytes, suitable for HTTP serving.
pub fn thumbnail_jpeg_bytes(path: &Path, max_size: u32, quality: u8) -> Vec<u8> {
    use image::ImageEncoder;
    let (rgba, w, h) = generate_thumbnail(path, max_size);
    // JPEG doesn't support alpha — convert RGBA to RGB
    let rgb: Vec<u8> = rgba.chunks_exact(4).flat_map(|px| &px[..3]).copied().collect();
    let mut buf = Vec::new();
    let encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buf, quality);
    let _ = encoder.write_image(&rgb, w, h, image::ExtendedColorType::Rgb8);
    buf
}

/// Fast EXIF thumbnail extraction. Returns (rgba, w, h) or None.
/// Does NOT check disk cache — that's for the full-quality path.
pub fn extract_preview(path: &Path, max_size: u32) -> Option<(Vec<u8>, u32, u32)> {
    let (orientation, exif_thumb) = read_exif_info(path);
    let data = exif_thumb?;
    let img = image::load_from_memory(&data).ok()?;
    let thumb = img.resize(max_size, max_size, FilterType::Triangle);
    let thumb = apply_orientation(thumb, orientation);
    let (w, h) = thumb.dimensions();
    Some((thumb.to_rgba8().into_raw(), w, h))
}

/// Extract EXIF previews for multiple paths in parallel.
pub fn extract_previews_parallel(
    paths: &[PathBuf],
    max_size: u32,
) -> Vec<(PathBuf, Option<(Vec<u8>, u32, u32)>)> {
    use rayon::prelude::*;
    paths
        .par_iter()
        .map(|p| (p.clone(), extract_preview(p, max_size)))
        .collect()
}

/// Generate thumbnails for multiple paths in parallel using rayon.
pub fn generate_thumbnails_parallel(
    paths: &[std::path::PathBuf],
    max_size: u32,
) -> Vec<(std::path::PathBuf, Vec<u8>, u32, u32)> {
    use rayon::prelude::*;

    paths
        .par_iter()
        .map(|p| {
            let (rgba, w, h) = generate_thumbnail(p, max_size);
            (p.clone(), rgba, w, h)
        })
        .collect()
}

