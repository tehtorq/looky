use std::path::Path;

use rusqlite::{Connection, Result, params};

use crate::metadata::FileSummary;

pub struct Catalog {
    conn: Connection,
}

impl Catalog {
    pub fn open(db_path: &Path) -> Result<Self> {
        if let Some(parent) = db_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let conn = Connection::open(db_path)?;
        let catalog = Catalog { conn };
        catalog.init_schema()?;
        Ok(catalog)
    }

    fn init_schema(&self) -> Result<()> {
        self.conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS images (
                id INTEGER PRIMARY KEY,
                path TEXT NOT NULL UNIQUE,
                file_size INTEGER NOT NULL,
                mtime_ns INTEGER NOT NULL,
                width INTEGER,
                height INTEGER,
                date_taken TEXT,
                date_modified TEXT,
                content_hash BLOB,
                perceptual_hash BLOB
            );

            CREATE INDEX IF NOT EXISTS idx_images_content_hash ON images(content_hash);",
        )
    }

    /// Returns cached hashes if the path exists in DB and file_size + mtime still match.
    pub fn get_hashes(&self, path: &Path) -> Option<([u8; 32], Vec<u8>)> {
        let path_str = path.to_string_lossy();
        let (disk_size, disk_mtime) = file_size_and_mtime(path)?;

        let mut stmt = self
            .conn
            .prepare_cached(
                "SELECT file_size, mtime_ns, content_hash, perceptual_hash
                 FROM images WHERE path = ?1",
            )
            .ok()?;

        stmt.query_row(params![path_str.as_ref()], |row| {
            let db_size: i64 = row.get(0)?;
            let db_mtime: i64 = row.get(1)?;
            let content_hash: Option<Vec<u8>> = row.get(2)?;
            let perceptual_hash: Option<Vec<u8>> = row.get(3)?;
            Ok((db_size, db_mtime, content_hash, perceptual_hash))
        })
        .ok()
        .and_then(|(db_size, db_mtime, content_hash, perceptual_hash)| {
            if db_size != disk_size as i64 || db_mtime != disk_mtime {
                return None;
            }
            let ch = content_hash?;
            let ph = perceptual_hash?;
            if ch.len() != 32 {
                return None;
            }
            let mut arr = [0u8; 32];
            arr.copy_from_slice(&ch);
            Some((arr, ph))
        })
    }

    /// Insert or replace hashes for a path.
    pub fn insert_hashes(
        &self,
        path: &Path,
        file_size: u64,
        mtime_ns: i64,
        content_hash: &[u8; 32],
        perceptual_hash: &[u8],
    ) {
        let path_str = path.to_string_lossy();
        let _ = self.conn.execute(
            "INSERT INTO images (path, file_size, mtime_ns, content_hash, perceptual_hash)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(path) DO UPDATE SET
                file_size = excluded.file_size,
                mtime_ns = excluded.mtime_ns,
                content_hash = excluded.content_hash,
                perceptual_hash = excluded.perceptual_hash",
            params![
                path_str.as_ref(),
                file_size as i64,
                mtime_ns,
                &content_hash[..],
                perceptual_hash,
            ],
        );
    }

    /// Returns a cached FileSummary if the path exists and size+mtime match.
    pub fn get_file_summary(&self, path: &Path) -> Option<FileSummary> {
        let path_str = path.to_string_lossy();
        let (disk_size, disk_mtime) = file_size_and_mtime(path)?;

        let mut stmt = self
            .conn
            .prepare_cached(
                "SELECT file_size, mtime_ns, width, height, date_taken, date_modified
                 FROM images WHERE path = ?1",
            )
            .ok()?;

        stmt.query_row(params![path_str.as_ref()], |row| {
            let db_size: i64 = row.get(0)?;
            let db_mtime: i64 = row.get(1)?;
            let width: Option<u32> = row.get(2)?;
            let height: Option<u32> = row.get(3)?;
            let date_taken: Option<String> = row.get(4)?;
            let date_modified: Option<String> = row.get(5)?;
            Ok((db_size, db_mtime, width, height, date_taken, date_modified))
        })
        .ok()
        .and_then(|(db_size, db_mtime, width, height, date_taken, date_modified)| {
            if db_size != disk_size as i64 || db_mtime != disk_mtime {
                return None;
            }
            // Only return if we actually have the summary fields populated
            if width.is_none() && date_taken.is_none() && date_modified.is_none() {
                return None;
            }
            let filename = path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            let dimensions = width.zip(height);
            Some(FileSummary {
                filename,
                file_size: disk_size,
                dimensions,
                date_taken,
                date_modified,
            })
        })
    }

    /// Insert or update the file summary metadata for a path.
    pub fn insert_file_summary(
        &self,
        path: &Path,
        file_size: u64,
        mtime_ns: i64,
        summary: &FileSummary,
    ) {
        let path_str = path.to_string_lossy();
        let (width, height) = match summary.dimensions {
            Some((w, h)) => (Some(w), Some(h)),
            None => (None, None),
        };
        let _ = self.conn.execute(
            "INSERT INTO images (path, file_size, mtime_ns, width, height, date_taken, date_modified)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(path) DO UPDATE SET
                file_size = excluded.file_size,
                mtime_ns = excluded.mtime_ns,
                width = excluded.width,
                height = excluded.height,
                date_taken = excluded.date_taken,
                date_modified = excluded.date_modified",
            params![
                path_str.as_ref(),
                file_size as i64,
                mtime_ns,
                width,
                height,
                summary.date_taken.as_deref(),
                summary.date_modified.as_deref(),
            ],
        );
    }

    /// Remove rows whose paths no longer exist on disk.
    pub fn prune_missing(&self) {
        let paths: Vec<String> = {
            let mut stmt = match self.conn.prepare("SELECT path FROM images") {
                Ok(s) => s,
                Err(_) => return,
            };
            stmt.query_map([], |row| row.get(0))
                .ok()
                .map(|rows| rows.filter_map(|r| r.ok()).collect())
                .unwrap_or_default()
        };

        for path_str in &paths {
            if !Path::new(path_str).exists() {
                let _ = self
                    .conn
                    .execute("DELETE FROM images WHERE path = ?1", params![path_str]);
            }
        }
    }
}

/// Get file size and mtime (as nanoseconds since epoch) from disk.
fn file_size_and_mtime(path: &Path) -> Option<(u64, i64)> {
    let meta = std::fs::metadata(path).ok()?;
    let size = meta.len();
    let mtime = meta
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?;
    let mtime_ns = mtime.as_nanos() as i64;
    Some((size, mtime_ns))
}

/// Helper to get file_size and mtime_ns for use by callers (e.g. app.rs).
pub fn file_size_and_mtime_for(path: &Path) -> Option<(u64, i64)> {
    file_size_and_mtime(path)
}
