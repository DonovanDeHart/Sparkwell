//! In-process vector index. For a personal library of thousands of Sparks a
//! linear cosine scan over normalised vectors takes well under a millisecond
//! per thousand vectors, so no separate vector database is needed.

use std::collections::HashMap;

use rusqlite::params;

use crate::error::AppResult;
use crate::storage::Library;

#[derive(Debug, Default)]
pub struct VectorIndex {
    /// Embedding model the vectors belong to.
    pub model: Option<String>,
    pub dims: usize,
    vectors: HashMap<i64, Vec<f32>>,
}

impl VectorIndex {
    pub fn empty(model: Option<String>) -> Self {
        Self { model, dims: 0, vectors: HashMap::new() }
    }

    pub fn len(&self) -> usize {
        self.vectors.len()
    }

    pub fn is_empty(&self) -> bool {
        self.vectors.is_empty()
    }

    pub fn contains(&self, id: i64) -> bool {
        self.vectors.contains_key(&id)
    }

    pub fn insert(&mut self, id: i64, vector: Vec<f32>) {
        if self.dims == 0 {
            self.dims = vector.len();
        }
        if vector.len() == self.dims {
            self.vectors.insert(id, vector);
        }
    }

    pub fn remove(&mut self, id: i64) {
        self.vectors.remove(&id);
    }

    /// Cosine similarity of `query` (normalised here) against every vector.
    /// Returns nothing when dimensions don't match the index.
    pub fn similarities(&self, query: &[f32]) -> Vec<(i64, f32)> {
        if query.len() != self.dims || self.dims == 0 {
            return Vec::new();
        }
        let q = normalized(query.to_vec());
        self.vectors
            .iter()
            .map(|(id, v)| (*id, dot(&q, v)))
            .collect()
    }

    /// Loads all stored vectors for `model` from the library.
    pub fn load(lib: &Library, model: &str) -> AppResult<VectorIndex> {
        let mut index = VectorIndex::empty(Some(model.to_string()));
        let mut stmt = lib
            .conn
            .prepare("SELECT spark_id, dimensions, vector FROM embeddings WHERE model = ?1")?;
        let rows = stmt.query_map(params![model], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?, r.get::<_, Vec<u8>>(2)?))
        })?;
        for row in rows {
            let (id, dims, blob) = row?;
            if let Some(v) = decode(&blob) {
                if v.len() as i64 == dims {
                    index.insert(id, v);
                }
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

pub fn encode(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|x| x.to_le_bytes()).collect()
}

pub fn decode(blob: &[u8]) -> Option<Vec<f32>> {
    if blob.len() % 4 != 0 || blob.is_empty() {
        return None;
    }
    Some(
        blob.chunks_exact(4)
            .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_decode_round_trip() {
        let v = vec![0.25f32, -1.5, 3.0];
        assert_eq!(decode(&encode(&v)).unwrap(), v);
        assert!(decode(&[1, 2, 3]).is_none());
        assert!(decode(&[]).is_none());
    }

    #[test]
    fn cosine_prefers_aligned_vectors() {
        let mut idx = VectorIndex::empty(Some("m".into()));
        idx.insert(1, normalized(vec![1.0, 0.0]));
        idx.insert(2, normalized(vec![0.0, 1.0]));
        idx.insert(3, normalized(vec![1.0, 1.0]));
        let mut sims = idx.similarities(&[2.0, 0.1]);
        sims.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
        assert_eq!(sims[0].0, 1);
        assert_eq!(sims[2].0, 2);
    }

    #[test]
    fn mismatched_dimensions_are_ignored() {
        let mut idx = VectorIndex::empty(Some("m".into()));
        idx.insert(1, vec![1.0, 0.0]);
        idx.insert(2, vec![1.0, 0.0, 0.0]); // wrong dims, dropped
        assert_eq!(idx.len(), 1);
        assert!(idx.similarities(&[1.0, 0.0, 0.0]).is_empty());
    }
}
