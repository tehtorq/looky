use std::collections::HashMap;

use iced::Task;

use crate::app::{Looky, Message};
use crate::catalog;
use crate::duplicates::{self, DuplicateGroup, ImageHashes};
use crate::metadata;
use crate::tasks;

const VISUAL_DUP_THRESHOLD: u32 = 10;

pub fn find_duplicates(state: &mut Looky) -> Task<Message> {
    state.dup_hashes.clear();
    state.dup_groups.clear();
    state.dup_badge_set.clear();
    state.dup_summaries.clear();
    state.dup_scanning = true;
    state.dup_compare = None;
    state.dup_view_active = false;
    state.dup_total = state.image_paths.len();

    let mut pending = Vec::new();
    for (i, path) in state.image_paths.iter().enumerate() {
        if let Some((content_hash, perceptual_hash)) =
            state.catalog.as_ref().and_then(|c| c.get_hashes(path))
        {
            state.dup_hashes.push((
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
    state.dup_pending = pending;
    tasks::load_next_dup_batch(state)
}

pub fn cancel_dup_scan(state: &mut Looky) {
    state.dup_pending.clear();
    state.dup_scanning = false;
    state.dup_hashes.clear();
    state.dup_total = 0;
}

pub fn dup_hash_batch_ready(
    state: &mut Looky,
    results: Vec<(usize, Option<ImageHashes>)>,
) -> Task<Message> {
    if !state.dup_scanning {
        return Task::none();
    }
    for (idx, maybe_hash) in results {
        if let Some(h) = maybe_hash {
            if let (Some(cat), Some(path)) =
                (state.catalog.as_ref(), state.image_paths.get(idx))
            {
                if let Some((file_size, mtime_ns)) = catalog::file_size_and_mtime_for(path) {
                    cat.insert_hashes(
                        path,
                        file_size,
                        mtime_ns,
                        &h.content_hash,
                        &h.perceptual_hash,
                    );
                }
            }
            state.dup_hashes.push((idx, h));
        }
    }
    if state.dup_pending.is_empty() {
        let hashes = state.dup_hashes.clone();
        let image_paths = state.image_paths.clone();

        let mut cached_summaries: HashMap<usize, metadata::FileSummary> = HashMap::new();
        if let Some(cat) = state.catalog.as_ref() {
            for (i, path) in image_paths.iter().enumerate() {
                if let Some(summary) = cat.get_file_summary(path) {
                    cached_summaries.insert(i, summary);
                }
            }
        }

        Task::perform(
            async move {
                let groups = duplicates::find_duplicates(&hashes, VISUAL_DUP_THRESHOLD);
                let dup_indices = duplicates::duplicate_indices(&groups);
                let summaries: HashMap<usize, metadata::FileSummary> = dup_indices
                    .iter()
                    .filter_map(|&idx| {
                        if let Some(cached) = cached_summaries.get(&idx) {
                            return Some((idx, cached.clone()));
                        }
                        let path = image_paths.get(idx)?;
                        Some((idx, metadata::read_file_summary(path)))
                    })
                    .collect();
                (groups, summaries)
            },
            |(groups, summaries)| Message::DupAnalysisReady(groups, summaries),
        )
    } else {
        tasks::load_next_dup_batch(state)
    }
}

pub fn dup_analysis_ready(
    state: &mut Looky,
    groups: Vec<DuplicateGroup>,
    summaries: HashMap<usize, metadata::FileSummary>,
) {
    state.dup_scanning = false;
    state.dup_badge_set = duplicates::duplicate_indices(&groups);
    state.dup_groups = groups;

    if let Some(cat) = state.catalog.as_ref() {
        for (idx, summary) in &summaries {
            if let Some(path) = state.image_paths.get(*idx) {
                if let Some((file_size, mtime_ns)) = catalog::file_size_and_mtime_for(path) {
                    cat.insert_file_summary(path, file_size, mtime_ns, summary);
                }
            }
        }
    }
    state.dup_summaries = summaries;
}

pub fn cached_dup_analysis_ready(
    state: &mut Looky,
    groups: Vec<DuplicateGroup>,
    summaries: HashMap<usize, metadata::FileSummary>,
) {
    if !state.dup_scanning {
        state.dup_badge_set = duplicates::duplicate_indices(&groups);
        state.dup_groups = groups;
        if let Some(cat) = state.catalog.as_ref() {
            for (idx, summary) in &summaries {
                if let Some(path) = state.image_paths.get(*idx) {
                    if let Some((fs, mt)) = catalog::file_size_and_mtime_for(path) {
                        cat.insert_file_summary(path, fs, mt, summary);
                    }
                }
            }
        }
        state.dup_summaries = summaries;
    }
}

pub fn images_found_dup_bootstrap(state: &mut Looky) -> Option<Task<Message>> {
    let cat = state.catalog.as_ref()?;
    let mut cached_hashes = Vec::new();
    for (i, path) in state.image_paths.iter().enumerate() {
        if let Some((ch, ph)) = cat.get_hashes(path) {
            cached_hashes.push((
                i,
                ImageHashes {
                    content_hash: ch,
                    perceptual_hash: ph,
                },
            ));
        }
    }
    if cached_hashes.len() < 2 {
        return None;
    }
    let image_paths = state.image_paths.clone();
    let mut cached_summaries: HashMap<usize, metadata::FileSummary> = HashMap::new();
    for (i, path) in image_paths.iter().enumerate() {
        if let Some(s) = cat.get_file_summary(path) {
            cached_summaries.insert(i, s);
        }
    }
    state.dup_hashes = cached_hashes.clone();
    Some(Task::perform(
        async move {
            let groups = duplicates::find_duplicates(&cached_hashes, VISUAL_DUP_THRESHOLD);
            let dup_indices = duplicates::duplicate_indices(&groups);
            let summaries: HashMap<usize, metadata::FileSummary> = dup_indices
                .iter()
                .filter_map(|&idx| {
                    if let Some(cached) = cached_summaries.get(&idx) {
                        return Some((idx, cached.clone()));
                    }
                    let path = image_paths.get(idx)?;
                    Some((idx, metadata::read_file_summary(path)))
                })
                .collect();
            (groups, summaries)
        },
        |(g, s)| Message::CachedDupAnalysisReady(g, s),
    ))
}
