//! A fast hasher for the maps of the pagination and layout code.
//!
//! The keys are small integers, tuples of them, and positions in the flow, which the standard
//! library's default hasher (SipHash, with a random key) hashes much more carefully than needed:
//! nothing here is exposed to input that could be crafted against the hash function, and the
//! maps are never iterated in a way that the output depends on. This is the multiply-and-rotate
//! hash of rustc (`FxHasher`).

use std::collections::{HashMap, HashSet};
use std::hash::{BuildHasherDefault, Hasher};

const SEED: u64 = 0xf135_7aea_2e62_a9c5;

#[derive(Clone, Copy, Default)]
pub struct FxHasher {
    hash: u64,
}

impl FxHasher {
    #[inline]
    fn add(&mut self, word: u64) {
        self.hash = (self.hash.rotate_left(5) ^ word).wrapping_mul(SEED);
    }
}

impl Hasher for FxHasher {
    #[inline]
    fn write(&mut self, bytes: &[u8]) {
        let mut chunks = bytes.chunks_exact(8);
        for chunk in &mut chunks {
            self.add(u64::from_le_bytes(chunk.try_into().unwrap()));
        }
        let rest = chunks.remainder();
        if !rest.is_empty() {
            let mut buf = [0u8; 8];
            buf[..rest.len()].copy_from_slice(rest);
            // the length keeps "a" and "a\0" apart
            self.add(u64::from_le_bytes(buf) ^ ((rest.len() as u64) << 56));
        }
    }
    #[inline]
    fn write_u8(&mut self, i: u8) {
        self.add(i as u64);
    }
    #[inline]
    fn write_u16(&mut self, i: u16) {
        self.add(i as u64);
    }
    #[inline]
    fn write_u32(&mut self, i: u32) {
        self.add(i as u64);
    }
    #[inline]
    fn write_u64(&mut self, i: u64) {
        self.add(i);
    }
    #[inline]
    fn write_usize(&mut self, i: usize) {
        self.add(i as u64);
    }
    #[inline]
    fn finish(&self) -> u64 {
        // hashbrown takes the index from the low bits and a tag from the high ones, and the last
        // multiplication leaves the low bits the weakest
        self.hash.rotate_left(26)
    }
}

pub type FxBuildHasher = BuildHasherDefault<FxHasher>;
/// A `HashMap` with the fast hasher: make one with `FxHashMap::default()`.
pub type FxHashMap<K, V> = HashMap<K, V, FxBuildHasher>;
pub type FxHashSet<K> = HashSet<K, FxBuildHasher>;
