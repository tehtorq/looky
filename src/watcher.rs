use std::path::Path;
use std::sync::mpsc;

use notify::{Event, RecommendedWatcher, RecursiveMode, Watcher};

pub struct FolderWatcher {
    _watcher: RecommendedWatcher,
    pub events: mpsc::Receiver<notify::Result<Event>>,
}

impl FolderWatcher {
    pub fn new(path: &Path) -> notify::Result<Self> {
        let (tx, rx) = mpsc::channel();
        let mut watcher = notify::recommended_watcher(tx)?;
        watcher.watch(path, RecursiveMode::NonRecursive)?;
        Ok(FolderWatcher {
            _watcher: watcher,
            events: rx,
        })
    }
}
