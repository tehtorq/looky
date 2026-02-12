use std::path::Path;

pub struct PhotoMetadata {
    pub filename: String,
    pub file_size: u64,
    pub dimensions: Option<(u32, u32)>,
    pub orientation: Option<u32>,
    // Date & time
    pub date_taken: Option<String>,
    pub date_modified: Option<String>,
    // Camera
    pub camera_make: Option<String>,
    pub camera_model: Option<String>,
    pub lens_model: Option<String>,
    pub software: Option<String>,
    // Exposure
    pub exposure_time: Option<String>,
    pub f_number: Option<String>,
    pub iso: Option<String>,
    pub focal_length: Option<String>,
    pub focal_length_35mm: Option<String>,
    pub exposure_bias: Option<String>,
    pub exposure_program: Option<String>,
    pub metering_mode: Option<String>,
    // Light & color
    pub flash: Option<String>,
    pub white_balance: Option<String>,
    pub color_space: Option<String>,
    // Other
    pub artist: Option<String>,
    pub copyright: Option<String>,
    pub description: Option<String>,
    // GPS
    pub gps_latitude: Option<f64>,
    pub gps_longitude: Option<f64>,
    pub gps_altitude: Option<String>,
}

pub fn read_metadata(path: &Path) -> PhotoMetadata {
    let filename = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();

    let fs_meta = std::fs::metadata(path).ok();
    let file_size = fs_meta.as_ref().map(|m| m.len()).unwrap_or(0);
    let date_modified = fs_meta
        .as_ref()
        .and_then(|m| m.modified().ok())
        .map(format_system_time);

    let dimensions = image::image_dimensions(path).ok();

    let exif_data = read_exif(path);

    let e = exif_data.as_ref();
    PhotoMetadata {
        filename,
        file_size,
        dimensions,
        orientation: e.and_then(|d| d.orientation),
        date_taken: e.and_then(|d| d.date_taken.clone()),
        date_modified,
        camera_make: e.and_then(|d| d.camera_make.clone()),
        camera_model: e.and_then(|d| d.camera_model.clone()),
        lens_model: e.and_then(|d| d.lens_model.clone()),
        software: e.and_then(|d| d.software.clone()),
        exposure_time: e.and_then(|d| d.exposure_time.clone()),
        f_number: e.and_then(|d| d.f_number.clone()),
        iso: e.and_then(|d| d.iso.clone()),
        focal_length: e.and_then(|d| d.focal_length.clone()),
        focal_length_35mm: e.and_then(|d| d.focal_length_35mm.clone()),
        exposure_bias: e.and_then(|d| d.exposure_bias.clone()),
        exposure_program: e.and_then(|d| d.exposure_program.clone()),
        metering_mode: e.and_then(|d| d.metering_mode.clone()),
        flash: e.and_then(|d| d.flash.clone()),
        white_balance: e.and_then(|d| d.white_balance.clone()),
        color_space: e.and_then(|d| d.color_space.clone()),
        artist: e.and_then(|d| d.artist.clone()),
        copyright: e.and_then(|d| d.copyright.clone()),
        description: e.and_then(|d| d.description.clone()),
        gps_latitude: e.and_then(|d| d.gps_latitude),
        gps_longitude: e.and_then(|d| d.gps_longitude),
        gps_altitude: e.and_then(|d| d.gps_altitude.clone()),
    }
}

struct ExifData {
    orientation: Option<u32>,
    date_taken: Option<String>,
    camera_make: Option<String>,
    camera_model: Option<String>,
    lens_model: Option<String>,
    software: Option<String>,
    exposure_time: Option<String>,
    f_number: Option<String>,
    iso: Option<String>,
    focal_length: Option<String>,
    focal_length_35mm: Option<String>,
    exposure_bias: Option<String>,
    exposure_program: Option<String>,
    metering_mode: Option<String>,
    flash: Option<String>,
    white_balance: Option<String>,
    color_space: Option<String>,
    artist: Option<String>,
    copyright: Option<String>,
    description: Option<String>,
    gps_latitude: Option<f64>,
    gps_longitude: Option<f64>,
    gps_altitude: Option<String>,
}

fn read_exif(path: &Path) -> Option<ExifData> {
    let file = std::fs::File::open(path).ok()?;
    let mut reader = std::io::BufReader::new(file);
    let exif = exif::Reader::new().read_from_container(&mut reader).ok()?;

    let get_str = |tag| {
        exif.get_field(tag, exif::In::PRIMARY)
            .map(|f| f.display_value().to_string())
    };

    Some(ExifData {
        orientation: exif
            .get_field(exif::Tag::Orientation, exif::In::PRIMARY)
            .and_then(|f| f.value.get_uint(0)),
        date_taken: get_str(exif::Tag::DateTimeOriginal),
        camera_make: get_str(exif::Tag::Make),
        camera_model: get_str(exif::Tag::Model),
        lens_model: get_str(exif::Tag::LensModel),
        software: get_str(exif::Tag::Software),
        exposure_time: get_str(exif::Tag::ExposureTime),
        f_number: get_str(exif::Tag::FNumber),
        iso: get_str(exif::Tag::PhotographicSensitivity),
        focal_length: get_str(exif::Tag::FocalLength),
        focal_length_35mm: get_str(exif::Tag::FocalLengthIn35mmFilm),
        exposure_bias: get_str(exif::Tag::ExposureBiasValue),
        exposure_program: get_str(exif::Tag::ExposureProgram),
        metering_mode: get_str(exif::Tag::MeteringMode),
        flash: get_str(exif::Tag::Flash),
        white_balance: get_str(exif::Tag::WhiteBalance),
        color_space: get_str(exif::Tag::ColorSpace),
        artist: exif
            .get_field(exif::Tag::Artist, exif::In::PRIMARY)
            .map(|f| f.display_value().to_string()),
        copyright: exif
            .get_field(exif::Tag::Copyright, exif::In::PRIMARY)
            .map(|f| f.display_value().to_string()),
        description: exif
            .get_field(exif::Tag::ImageDescription, exif::In::PRIMARY)
            .map(|f| f.display_value().to_string()),
        gps_latitude: parse_gps_coord(&exif, exif::Tag::GPSLatitude, exif::Tag::GPSLatitudeRef),
        gps_longitude: parse_gps_coord(&exif, exif::Tag::GPSLongitude, exif::Tag::GPSLongitudeRef),
        gps_altitude: get_str(exif::Tag::GPSAltitude),
    })
}

fn parse_gps_coord(exif: &exif::Exif, coord_tag: exif::Tag, ref_tag: exif::Tag) -> Option<f64> {
    let field = exif.get_field(coord_tag, exif::In::PRIMARY)?;
    let rationals = match &field.value {
        exif::Value::Rational(v) if v.len() >= 3 => v,
        _ => return None,
    };

    let degrees = rationals[0].to_f64();
    let minutes = rationals[1].to_f64();
    let seconds = rationals[2].to_f64();
    let mut coord = degrees + minutes / 60.0 + seconds / 3600.0;

    let ref_field = exif.get_field(ref_tag, exif::In::PRIMARY)?;
    let ref_str = ref_field.display_value().to_string();
    if ref_str == "S" || ref_str == "W" {
        coord = -coord;
    }

    Some(coord)
}

/// Lightweight summary for duplicate comparison — avoids full EXIF parse.
#[derive(Debug, Clone)]
pub struct FileSummary {
    pub filename: String,
    pub file_size: u64,
    pub dimensions: Option<(u32, u32)>,
    pub date_taken: Option<String>,
    pub date_modified: Option<String>,
}

pub fn read_file_summary(path: &Path) -> FileSummary {
    let filename = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();

    let fs_meta = std::fs::metadata(path).ok();
    let file_size = fs_meta.as_ref().map(|m| m.len()).unwrap_or(0);
    let date_modified = fs_meta
        .as_ref()
        .and_then(|m| m.modified().ok())
        .map(format_system_time);

    let dimensions = image::image_dimensions(path).ok();

    // Quick EXIF read just for date_taken
    let date_taken = read_exif(path).and_then(|d| d.date_taken);

    FileSummary {
        filename,
        file_size,
        dimensions,
        date_taken,
        date_modified,
    }
}

fn format_system_time(time: std::time::SystemTime) -> String {
    let duration = time
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = duration.as_secs() as i64;

    // Simple UTC formatting without pulling in chrono
    let days = secs / 86400;
    let time_of_day = secs % 86400;
    let hours = time_of_day / 3600;
    let minutes = (time_of_day % 3600) / 60;
    let seconds = time_of_day % 60;

    // Days since 1970-01-01
    let (year, month, day) = days_to_date(days);
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
        year, month, day, hours, minutes, seconds
    )
}

fn days_to_date(mut days: i64) -> (i64, i64, i64) {
    // Algorithm from http://howardhinnant.github.io/date_algorithms.html
    days += 719468;
    let era = if days >= 0 { days } else { days - 146096 } / 146097;
    let doe = days - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}

pub fn format_file_size(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{} B", bytes)
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else if bytes < 1024 * 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    } else {
        format!("{:.2} GB", bytes as f64 / (1024.0 * 1024.0 * 1024.0))
    }
}
