use std::path::{Path, PathBuf};

pub async fn pick_folder() -> Option<PathBuf> {
    rfd::AsyncFileDialog::new()
        .set_title("Select a photo folder")
        .pick_folder()
        .await
        .map(|handle| handle.path().to_path_buf())
}

pub async fn scan_folder(folder: PathBuf) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    let mut stack = vec![folder];
    while let Some(dir) = stack.pop() {
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                } else if is_image_file(&path) {
                    paths.push(path);
                }
            }
        }
    }
    paths.sort();
    paths
}

fn is_image_file(path: &Path) -> bool {
    match path.extension().and_then(|e| e.to_str()) {
        Some(ext) => matches!(
            ext.to_lowercase().as_str(),
            "jpg" | "jpeg" | "png" | "gif" | "bmp" | "webp" | "tiff" | "tif" | "heic" | "heif"
        ),
        None => false,
    }
}

pub fn config_dir() -> Option<PathBuf> {
    dirs_next::home_dir().map(|d| d.join(".looky"))
}

pub fn save_last_folder(path: &Path) {
    if let Some(dir) = config_dir() {
        let _ = std::fs::create_dir_all(&dir);
        let _ = std::fs::write(dir.join("last_folder"), path.to_string_lossy().as_bytes());
    }
}

pub fn load_last_folder() -> Option<PathBuf> {
    let dir = config_dir()?;
    let data = std::fs::read_to_string(dir.join("last_folder")).ok()?;
    let path = PathBuf::from(data.trim());
    if path.is_dir() { Some(path) } else { None }
}
