use std::collections::HashMap;
use std::path::PathBuf;

use iced::Task;

use crate::app::{Looky, Message};
use crate::duplicates::{self, DuplicateGroup, HashResult, ImageHashes};
use crate::metadata;
use crate::tasks;

const VISUAL_DUP_THRESHOLD: u32 = 6;

pub fn find_duplicates(state: &mut Looky) -> Task<Message> {
    state.dup_generation += 1;
    state.dup_hashes.clear();
    state.dup_pending.clear();
    state.dup_groups.clear();
    state.dup_badge_set.clear();
    state.dup_summaries.clear();
    state.dup_scanning = true;
    state.dup_compare = None;
    state.dup_view_active = false;
    state.dup_total = state.image_paths.len();

    // Cache lookups stat + query per image — run them off the UI thread on a
    // second catalog connection.
    let generation = state.dup_generation;
    let image_paths = state.image_paths.clone();
    let failed = state.dup_failed.clone();
    let catalog = state.catalog.as_ref().and_then(|c| c.try_clone());
    Task::perform(
        tasks::run_blocking(move || {
            let mut cached = Vec::new();
            let mut pending = Vec::new();
            for (i, path) in image_paths.iter().enumerate() {
                if failed.contains(path) {
                    continue;
                }
                if let Some((content_hash, perceptual_hash)) =
                    catalog.as_ref().and_then(|c| c.get_hashes(path))
                {
                    cached.push((
                        i,
                        ImageHashes {
                            content_hash,
                            perceptual_hash,
                        },
                    ));
                } else {
                    pending.push((i, path.clone()));
                }
            }
            (cached, pending)
        }),
        move |(cached, pending)| Message::DupScanPrepared(generation, cached, pending),
    )
}

pub fn dup_scan_prepared(
    state: &mut Looky,
    generation: u64,
    hashes: Vec<(usize, ImageHashes)>,
    pending: Vec<(usize, PathBuf)>,
) -> Task<Message> {
    if generation != state.dup_generation || !state.dup_scanning {
        return Task::none();
    }
    state.dup_hashes = hashes;
    state.dup_pending = pending;
    if state.dup_pending.is_empty() {
        return finish_dup_scan(state);
    }
    tasks::load_next_dup_batch(state)
}

pub fn cancel_dup_scan(state: &mut Looky) {
    state.dup_generation += 1;
    state.dup_pending.clear();
    state.dup_scanning = false;
    state.dup_hashes.clear();
    state.dup_total = 0;
}

pub fn dup_hash_batch_ready(
    state: &mut Looky,
    generation: u64,
    results: Vec<(usize, Option<HashResult>)>,
) -> Task<Message> {
    if generation != state.dup_generation || !state.dup_scanning {
        return Task::none();
    }
    let mut to_persist: Vec<(PathBuf, HashResult)> = Vec::new();
    for (idx, maybe_hash) in results {
        match maybe_hash {
            Some(r) => {
                if let Some(path) = state.image_paths.get(idx) {
                    to_persist.push((path.clone(), r.clone()));
                }
                state.dup_hashes.push((idx, r.hashes));
            }
            None => {
                if let Some(path) = state.image_paths.get(idx) {
                    state.dup_failed.insert(path.clone());
                }
            }
        }
    }
    if let Some(cat) = state.catalog.as_ref() {
        cat.insert_hashes_batch(&to_persist);
    }
    if state.dup_pending.is_empty() {
        finish_dup_scan(state)
    } else {
        tasks::load_next_dup_batch(state)
    }
}

/// All hashes collected — group them and gather summaries off-thread.
fn finish_dup_scan(state: &mut Looky) -> Task<Message> {
    let generation = state.dup_generation;
    let hashes = state.dup_hashes.clone();
    let image_paths = state.image_paths.clone();
    let catalog = state.catalog.as_ref().and_then(|c| c.try_clone());
    Task::perform(
        tasks::run_blocking(move || {
            let groups = duplicates::find_duplicates(&hashes, VISUAL_DUP_THRESHOLD);
            let dup_indices = duplicates::duplicate_indices(&groups);
            let summaries: HashMap<usize, metadata::FileSummary> = dup_indices
                .iter()
                .filter_map(|&idx| {
                    let path = image_paths.get(idx)?;
                    if let Some(cached) = catalog.as_ref().and_then(|c| c.get_file_summary(path)) {
                        return Some((idx, cached));
                    }
                    Some((idx, metadata::read_file_summary(path)))
                })
                .collect();
            (groups, summaries)
        }),
        move |(groups, summaries)| Message::DupAnalysisReady(generation, groups, summaries),
    )
}

pub fn dup_analysis_ready(
    state: &mut Looky,
    generation: u64,
    groups: Vec<DuplicateGroup>,
    summaries: HashMap<usize, metadata::FileSummary>,
) {
    if generation != state.dup_generation {
        return;
    }
    state.dup_scanning = false;
    state.dup_badge_set = duplicates::duplicate_indices(&groups);
    state.dup_groups = groups;
    persist_summaries(state, &summaries);
    state.dup_summaries = summaries;
}

pub fn cached_dup_analysis_ready(
    state: &mut Looky,
    generation: u64,
    groups: Vec<DuplicateGroup>,
    summaries: HashMap<usize, metadata::FileSummary>,
) {
    if generation != state.scan_generation || state.dup_scanning {
        return;
    }
    state.dup_badge_set = duplicates::duplicate_indices(&groups);
    state.dup_groups = groups;
    persist_summaries(state, &summaries);
    state.dup_summaries = summaries;
}

fn persist_summaries(state: &Looky, summaries: &HashMap<usize, metadata::FileSummary>) {
    let Some(cat) = state.catalog.as_ref() else {
        return;
    };
    let items: Vec<(PathBuf, u64, i64, metadata::FileSummary)> = summaries
        .iter()
        .filter_map(|(idx, summary)| {
            let path = state.image_paths.get(*idx)?;
            let (file_size, mtime_ns) = crate::catalog::file_size_and_mtime_for(path)?;
            Some((path.clone(), file_size, mtime_ns, summary.clone()))
        })
        .collect();
    cat.insert_summaries_batch(&items);
}

/// On folder open, run duplicate analysis from cached hashes alone (no
/// decoding) so badges appear without an explicit scan.
pub fn images_found_dup_bootstrap(state: &mut Looky) -> Option<Task<Message>> {
    let catalog = state.catalog.as_ref()?.try_clone()?;
    let image_paths = state.image_paths.clone();
    let generation = state.scan_generation;
    Some(Task::perform(
        tasks::run_blocking(move || {
            let mut cached_hashes = Vec::new();
            for (i, path) in image_paths.iter().enumerate() {
                if let Some((content_hash, perceptual_hash)) = catalog.get_hashes(path) {
                    cached_hashes.push((
                        i,
                        ImageHashes {
                            content_hash,
                            perceptual_hash,
                        },
                    ));
                }
            }
            if cached_hashes.len() < 2 {
                return (Vec::new(), HashMap::new());
            }
            let groups = duplicates::find_duplicates(&cached_hashes, VISUAL_DUP_THRESHOLD);
            let dup_indices = duplicates::duplicate_indices(&groups);
            let summaries: HashMap<usize, metadata::FileSummary> = dup_indices
                .iter()
                .filter_map(|&idx| {
                    let path = image_paths.get(idx)?;
                    if let Some(cached) = catalog.get_file_summary(path) {
                        return Some((idx, cached));
                    }
                    Some((idx, metadata::read_file_summary(path)))
                })
                .collect();
            (groups, summaries)
        }),
        move |(groups, summaries)| Message::CachedDupAnalysisReady(generation, groups, summaries),
    ))
}
