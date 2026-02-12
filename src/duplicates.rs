use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use image_hasher::{HashAlg, HasherConfig};
use rayon::prelude::*;
use sha2::{Digest, Sha256};

#[derive(Debug, Clone)]
pub struct ImageHashes {
    pub content_hash: [u8; 32],
    pub perceptual_hash: Vec<u8>,
}

#[derive(Debug, Clone)]
pub enum MatchKind {
    Exact,
    Visual { distance: u32 },
}

#[derive(Debug, Clone)]
pub struct DuplicateGroup {
    pub match_kind: MatchKind,
    pub indices: Vec<usize>,
}

/// Compute SHA-256 and perceptual hash for a single image.
pub fn compute_hashes(path: &Path) -> Option<ImageHashes> {
    let file_bytes = std::fs::read(path).ok()?;

    // SHA-256 content hash
    let content_hash: [u8; 32] = Sha256::digest(&file_bytes).into();

    // Perceptual hash (dHash 8x8 gradient)
    let img = image::load_from_memory(&file_bytes).ok()?;
    let hasher = HasherConfig::new()
        .hash_alg(HashAlg::Gradient)
        .hash_size(8, 8)
        .to_hasher();
    let phash = hasher.hash_image(&img);
    let perceptual_hash = phash.as_bytes().to_vec();

    Some(ImageHashes {
        content_hash,
        perceptual_hash,
    })
}

/// Compute hashes for a batch of (index, path) pairs in parallel.
pub fn compute_hashes_batch(items: &[(usize, PathBuf)]) -> Vec<(usize, Option<ImageHashes>)> {
    items
        .par_iter()
        .map(|(idx, path)| (*idx, compute_hashes(path)))
        .collect()
}

/// Find duplicate groups from a set of hashes.
/// `threshold` is the max hamming distance for visual matches.
pub fn find_duplicates(hashes: &[(usize, ImageHashes)], threshold: u32) -> Vec<DuplicateGroup> {
    let mut groups = Vec::new();

    // Phase 1: Exact matches by SHA-256
    let mut sha_groups: HashMap<[u8; 32], Vec<usize>> = HashMap::new();
    for (idx, h) in hashes {
        sha_groups.entry(h.content_hash).or_default().push(*idx);
    }

    let mut exact_matched: HashSet<usize> = HashSet::new();
    for indices in sha_groups.values() {
        if indices.len() > 1 {
            for &idx in indices {
                exact_matched.insert(idx);
            }
            groups.push(DuplicateGroup {
                match_kind: MatchKind::Exact,
                indices: indices.clone(),
            });
        }
    }

    // Phase 2: Visual matches via perceptual hash hamming distance
    // Collect non-exact hashes for pairwise comparison
    let non_exact: Vec<(usize, &[u8])> = hashes
        .iter()
        .filter(|(idx, _)| !exact_matched.contains(idx))
        .map(|(idx, h)| (*idx, h.perceptual_hash.as_slice()))
        .collect();

    let n = non_exact.len();

    // Parallel pairwise distance computation (the expensive part)
    let non_exact_ref = &non_exact;
    let matching_pairs: Vec<(usize, usize, u32)> = (0..n)
        .into_par_iter()
        .flat_map_iter(|i| {
            (i + 1..n).filter_map(move |j| {
                let dist = hamming_distance(non_exact_ref[i].1, non_exact_ref[j].1);
                if dist <= threshold {
                    Some((i, j, dist))
                } else {
                    None
                }
            })
        })
        .collect();

    // Sequential union-find clustering on the matching pairs
    let mut parent: Vec<usize> = (0..n).collect();

    fn find(parent: &mut [usize], mut x: usize) -> usize {
        while parent[x] != x {
            parent[x] = parent[parent[x]];
            x = parent[x];
        }
        x
    }

    fn union(parent: &mut [usize], a: usize, b: usize) {
        let ra = find(parent, a);
        let rb = find(parent, b);
        if ra != rb {
            parent[rb] = ra;
        }
    }

    let mut min_distance: HashMap<usize, u32> = HashMap::new();

    for (i, j, dist) in &matching_pairs {
        let ri = find(&mut parent, *i);
        let rj = find(&mut parent, *j);
        union(&mut parent, *i, *j);
        let root = find(&mut parent, *i);
        let existing = min_distance.get(&root).copied().unwrap_or(u32::MAX);
        let di = min_distance.get(&ri).copied().unwrap_or(u32::MAX);
        let dj = min_distance.get(&rj).copied().unwrap_or(u32::MAX);
        let best = dist.min(&existing).min(&di).min(&dj);
        min_distance.insert(root, *best);
    }

    // Collect clusters
    let mut clusters: HashMap<usize, Vec<usize>> = HashMap::new();
    for i in 0..n {
        let root = find(&mut parent, i);
        clusters.entry(root).or_default().push(non_exact[i].0);
    }

    for (root, indices) in clusters {
        if indices.len() > 1 {
            let distance = min_distance.get(&root).copied().unwrap_or(0);
            groups.push(DuplicateGroup {
                match_kind: MatchKind::Visual { distance },
                indices,
            });
        }
    }

    groups
}

/// Get the set of all indices that appear in any duplicate group, for O(1) badge lookup.
pub fn duplicate_indices(groups: &[DuplicateGroup]) -> HashSet<usize> {
    let mut set = HashSet::new();
    for g in groups {
        for &idx in &g.indices {
            set.insert(idx);
        }
    }
    set
}

fn hamming_distance(a: &[u8], b: &[u8]) -> u32 {
    a.iter()
        .zip(b.iter())
        .map(|(x, y)| (x ^ y).count_ones())
        .sum()
}
