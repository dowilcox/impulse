//! A thread-safe LRU cache from directories to their repository's root (the
//! `REPO_ROOT_CACHE` of the original Rust), so repeated lookups skip the
//! `git_repository_discover` walk up the directory tree.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

pub struct RepoCache {
    capacity: usize,
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    map: HashMap<String, String>,
    /// Least recently used first.
    order: Vec<String>,
}

/// The cache every repository lookup uses.
pub static REPO_CACHE: LazyLock<RepoCache> = LazyLock::new(|| RepoCache::new(64));

impl RepoCache {
    pub fn new(capacity: usize) -> Self {
        assert!(capacity > 0);
        RepoCache {
            capacity,
            state: Mutex::new(State::default()),
        }
    }

    /// The cached root for `dir`, marking it most recently used.
    pub fn root(&self, dir: &str) -> Option<String> {
        let mut state = self.state.lock().unwrap();
        let root = state.map.get(dir)?.clone();
        touch(&mut state, dir);
        Some(root)
    }

    /// Insert or update an entry, evicting the least recently used one when
    /// full.
    pub fn store(&self, root: &str, dir: &str) {
        let mut state = self.state.lock().unwrap();
        if !state.map.contains_key(dir)
            && state.map.len() >= self.capacity
            && !state.order.is_empty()
        {
            let oldest = state.order.remove(0);
            state.map.remove(&oldest);
        }
        state.map.insert(dir.to_string(), root.to_string());
        touch(&mut state, dir);
    }

    pub fn clear(&self) {
        let mut state = self.state.lock().unwrap();
        state.map.clear();
        state.order.clear();
    }

    pub fn len(&self) -> usize {
        self.state.lock().unwrap().map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

fn touch(state: &mut State, key: &str) {
    if let Some(index) = state.order.iter().position(|k| k == key) {
        state.order.remove(index);
    }
    state.order.push(key.to_string());
}
