//! In-process semantic index.
//!
//! Each Spark has several normalised vectors (its retrieval-profile view and
//! body passages, see [`super::profile`]). A query's similarity to a Spark is a
//! soft maximum over those views.
//!
//! Raw cosine similarity is a poor ranking signal on its own: some Sparks are
//! written in generic "AI assistant" language and sit close to *every* goal
//! (hubness), and every embedding model has its own cosine range. Both are
//! corrected against a fixed pool of generic, unrelated goals embedded with the
//! same model:
//!
//! - a Spark's *hub level* is its mean similarity to the generic goals it is
//!   closest to; it is subtracted from the query similarity;
//! - the *scale* is derived from how much similarities vary across the pool
//!   for this model, so the same thresholds work for different models.
//!
//! A linear scan over a few thousand Sparks takes well under a millisecond per
//! thousand views, so no vector database is needed.

use std::collections::HashMap;

use rusqlite::params;

use crate::error::AppResult;
use crate::storage::Library;

use super::{Calibration, CALIBRATION};

/// Below this many indexed Sparks the corrections are too noisy to trust.
pub const MIN_INDEXED: usize = 5;

#[derive(Debug, Default)]
pub struct VectorIndex {
    /// Embedding model the vectors belong to.
    pub model: Option<String>,
    pub dims: usize,
    views: HashMap<i64, Vec<Vec<f32>>>,
    /// Generic-goal query vectors for hubness and scale.
    pool: Vec<Vec<f32>>,
    /// Per Spark: (hub level, spread of its similarities to the pool).
    corrections: HashMap<i64, (f32, f32)>,
    spread_sum: f32,
    cal: Calibration,
}

/// One Spark's semantic evidence for a query.
#[derive(Debug, Clone, Copy)]
pub struct Evidence {
    pub id: i64,
    /// Hub-corrected similarity divided by the model scale, clamped to 0..=1.
    pub score: f64,
}

impl VectorIndex {
    pub fn empty(model: Option<String>) -> Self {
        Self::with_calibration(model, CALIBRATION)
    }

    /// An index using other calibration values (tests and tuning).
    pub fn with_calibration(model: Option<String>, cal: Calibration) -> Self {
        Self {
            model,
            cal,
            ..Default::default()
        }
    }

    pub fn calibration(&self) -> Calibration {
        self.cal
    }

    /// Model-specific scale that maps corrected similarity onto 0..=1.
    pub fn scale(&self) -> f32 {
        if self.corrections.is_empty() {
            return 0.30;
        }
        let typical = self.spread_sum / self.corrections.len() as f32;
        (typical * self.cal.scale_spreads).clamp(self.cal.scale_min, self.cal.scale_max)
    }

    pub fn len(&self) -> usize {
        self.views.len()
    }

    pub fn is_empty(&self) -> bool {
        self.views.is_empty()
    }

    pub fn contains(&self, id: i64) -> bool {
        self.views.contains_key(&id)
    }

    pub fn has_pool(&self) -> bool {
        !self.pool.is_empty()
    }

    /// Enough material for trustworthy semantic evidence.
    pub fn ready(&self) -> bool {
        self.has_pool() && self.len() >= MIN_INDEXED
    }

    pub fn ids(&self) -> impl Iterator<Item = i64> + '_ {
        self.views.keys().copied()
    }

    /// Adds or replaces a Spark's views. Views with the wrong dimensions are
    /// dropped; a Spark without usable views is not indexed.
    pub fn insert(&mut self, id: i64, views: Vec<Vec<f32>>) {
        let views: Vec<Vec<f32>> = views
            .into_iter()
            .filter(|v| !v.is_empty())
            .map(normalized)
            .collect();
        let Some(first) = views.first() else {
            return;
        };
        if self.dims == 0 {
            self.dims = first.len();
        }
        if views.iter().any(|v| v.len() != self.dims) {
            return;
        }
        self.remove(id);
        if !self.pool.is_empty() {
            let c = self.correction(&views);
            self.spread_sum += c.1;
            self.corrections.insert(id, c);
        }
        self.views.insert(id, views);
    }

    pub fn remove(&mut self, id: i64) {
        self.views.remove(&id);
        if let Some((_, spread)) = self.corrections.remove(&id) {
            self.spread_sum -= spread;
        }
    }

    /// Installs the generic-goal vectors and recomputes every correction.
    pub fn set_pool(&mut self, pool: Vec<Vec<f32>>) {
        let pool: Vec<Vec<f32>> = pool.into_iter().map(normalized).collect();
        if self.dims == 0 {
            self.dims = pool.first().map(Vec::len).unwrap_or(0);
        }
        if pool.is_empty() || pool.iter().any(|v| v.len() != self.dims) {
            return;
        }
        self.pool = pool;
        self.corrections = self
            .views
            .iter()
            .map(|(id, views)| (*id, self.correction(views)))
            .collect();
        self.spread_sum = self.corrections.values().map(|c| c.1).sum();
    }

    fn view_similarity(&self, query: &[f32], views: &[Vec<f32>]) -> f32 {
        let mut sims: Vec<f32> = views
            .iter()
            .enumerate()
            .map(|(i, v)| {
                let s = dot(query, v);
                if i == 0 {
                    s * self.cal.profile_weight
                } else {
                    s
                }
            })
            .collect();
        sims.sort_by(|a, b| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal));
        match sims.as_slice() {
            [] => 0.0,
            [only] => *only,
            [best, second, ..] => self.cal.best_view * best + (1.0 - self.cal.best_view) * second,
        }
    }

    /// (hub level, spread) of a Spark against the generic pool.
    fn correction(&self, views: &[Vec<f32>]) -> (f32, f32) {
        let mut sims: Vec<f32> = self
            .pool
            .iter()
            .map(|p| self.view_similarity(p, views))
            .collect();
        let n = sims.len().max(1) as f32;
        let mean = sims.iter().sum::<f32>() / n;
        let spread = (sims.iter().map(|s| (s - mean).powi(2)).sum::<f32>() / n).sqrt();
        sims.sort_by(|a, b| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal));
        let k = self.cal.hub_neighbours.min(sims.len()).max(1);
        let hub = sims.iter().take(k).sum::<f32>() / k as f32;
        (hub, spread)
    }

    /// Semantic evidence for every indexed Spark. Empty when the query has the
    /// wrong dimensions or the index isn't ready.
    pub fn evidence(&self, query: &[f32]) -> Vec<Evidence> {
        if !self.ready() || query.len() != self.dims {
            return Vec::new();
        }
        let q = normalized(query.to_vec());
        let scale = self.scale();
        self.views
            .iter()
            .map(|(id, views)| {
                let hub = self.corrections.get(id).map(|c| c.0).unwrap_or(0.0);
                let raw = self.view_similarity(&q, views) - hub;
                Evidence {
                    id: *id,
                    score: (raw / scale).clamp(0.0, 1.0) as f64,
                }
            })
            .collect()
    }

    /// Cosine similarity of two Sparks' profile views (how alike their purposes are).
    pub fn purpose_similarity(&self, a: i64, b: i64) -> Option<f32> {
        let va = self.views.get(&a)?.first()?;
        let vb = self.views.get(&b)?.first()?;
        Some(dot(va, vb))
    }

    /// Loads stored vectors for `model` and the current profile recipe whose
    /// content hash still matches the Spark's views (`expected`); vectors from
    /// other models, recipes or edited Sparks are never mixed in.
    pub fn load(
        lib: &Library,
        model: &str,
        expected: &HashMap<i64, String>,
    ) -> AppResult<VectorIndex> {
        let mut index = VectorIndex::empty(Some(model.to_string()));
        let mut stmt = lib.conn.prepare(
            "SELECT spark_id, dimensions, vector, content_hash FROM embeddings
             WHERE model = ?1 AND profile_version = ?2",
        )?;
        let rows = stmt.query_map(params![model, super::profile::PROFILE_VERSION], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, Vec<u8>>(2)?,
                r.get::<_, String>(3)?,
            ))
        })?;
        for row in rows {
            let (id, dims, blob, hash) = row?;
            if expected.get(&id) != Some(&hash) {
                continue;
            }
            if let Some(views) = decode_views(&blob, dims as usize) {
                index.insert(id, views);
            }
        }
        Ok(index)
    }
}

fn dot(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

pub fn normalized(mut v: Vec<f32>) -> Vec<f32> {
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > f32::EPSILON {
        for x in &mut v {
            *x /= norm;
        }
    }
    v
}

/// Little-endian f32s of every view, back to back.
pub fn encode_views(views: &[Vec<f32>]) -> Vec<u8> {
    views
        .iter()
        .flat_map(|v| v.iter().flat_map(|x| x.to_le_bytes()))
        .collect()
}

pub fn decode_views(blob: &[u8], dims: usize) -> Option<Vec<Vec<f32>>> {
    if dims == 0 || blob.is_empty() || blob.len() % (dims * 4) != 0 {
        return None;
    }
    let floats: Vec<f32> = blob
        .chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect();
    Some(floats.chunks(dims).map(<[f32]>::to_vec).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unit(v: &[f32]) -> Vec<f32> {
        normalized(v.to_vec())
    }

    #[test]
    fn views_round_trip() {
        let views = vec![vec![0.25f32, -1.5, 3.0], vec![1.0, 0.0, 0.0]];
        let blob = encode_views(&views);
        assert_eq!(decode_views(&blob, 3).unwrap(), views);
        assert!(decode_views(&[1, 2, 3], 3).is_none());
        assert!(decode_views(&blob, 0).is_none());
        assert!(decode_views(&blob, 4).is_none());
    }

    #[test]
    fn soft_maximum_prefers_the_best_view() {
        let q = unit(&[1.0, 0.0]);
        let views = vec![unit(&[0.0, 1.0]), unit(&[1.0, 0.0])];
        let index = VectorIndex::empty(None);
        let s = index.view_similarity(&q, &views);
        let expected = CALIBRATION.best_view;
        assert!(
            (s - expected).abs() < 1e-6,
            "best_view * best + (1 - best_view) * second, got {s}"
        );
    }

    fn index_with(pool: Vec<Vec<f32>>, sparks: &[(i64, Vec<f32>)]) -> VectorIndex {
        let mut idx = VectorIndex::empty(Some("m".into()));
        for (id, v) in sparks {
            idx.insert(*id, vec![v.clone()]);
        }
        idx.set_pool(pool);
        idx
    }

    #[test]
    fn hub_sparks_are_corrected() {
        // Spark 1 sits near every generic goal (a hub); Spark 2 is specific.
        let pool: Vec<Vec<f32>> = (0..12)
            .map(|i| unit(&[1.0, 0.2 * (i % 3) as f32, 0.0]))
            .collect();
        let mut sparks = vec![(1, unit(&[1.0, 0.1, 0.3])), (2, unit(&[0.3, 0.1, 1.0]))];
        for id in 3..=6 {
            sparks.push((id, unit(&[0.2, 1.0, 0.2 * id as f32])));
        }
        let idx = index_with(pool, &sparks);
        // A query equally close to both by raw cosine...
        let q = unit(&[0.7, 0.1, 0.7]);
        let ev: HashMap<i64, f64> = idx
            .evidence(&q)
            .into_iter()
            .map(|e| (e.id, e.score))
            .collect();
        // ...favours the specific Spark once the hub is corrected.
        assert!(ev[&2] > ev[&1], "{ev:?}");
    }

    #[test]
    fn not_ready_without_pool_or_enough_sparks() {
        let mut idx = VectorIndex::empty(Some("m".into()));
        for id in 1..=6 {
            idx.insert(id, vec![unit(&[1.0, id as f32])]);
        }
        assert!(!idx.ready(), "no generic pool yet");
        idx.set_pool(vec![unit(&[1.0, 0.0]), unit(&[0.0, 1.0])]);
        assert!(idx.ready());
        assert!(!idx.evidence(&[1.0, 0.0]).is_empty());
        assert!(
            idx.evidence(&[1.0, 0.0, 0.0]).is_empty(),
            "wrong dimensions"
        );
        for id in 1..=2 {
            idx.remove(id);
        }
        assert!(!idx.ready(), "too few Sparks");
    }

    #[test]
    fn mismatched_dimensions_are_ignored() {
        let mut idx = VectorIndex::empty(Some("m".into()));
        idx.insert(1, vec![vec![1.0, 0.0]]);
        idx.insert(2, vec![vec![1.0, 0.0, 0.0]]);
        idx.insert(3, vec![vec![1.0, 0.0], vec![1.0, 0.0, 0.0]]);
        assert_eq!(idx.len(), 1);
    }

    #[test]
    fn purpose_similarity_uses_profile_views() {
        let mut idx = VectorIndex::empty(Some("m".into()));
        idx.insert(1, vec![unit(&[1.0, 0.0]), unit(&[0.0, 1.0])]);
        idx.insert(2, vec![unit(&[1.0, 0.0]), unit(&[1.0, 0.0])]);
        assert!((idx.purpose_similarity(1, 2).unwrap() - 1.0).abs() < 1e-6);
        assert!(idx.purpose_similarity(1, 9).is_none());
    }
}
