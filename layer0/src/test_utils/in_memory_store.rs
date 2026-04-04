//! InMemoryStore — HashMap-backed StateStore for testing.

use crate::intent::Scope;
use crate::error::StateError;
use crate::state::{MemoryLink, SearchResult, StateStore};
use async_trait::async_trait;
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::RwLock;

/// In-memory state store backed by a `HashMap` behind a `RwLock`.
/// Scopes are serialized to strings as map keys for simplicity.
pub struct InMemoryStore {
    data: RwLock<HashMap<(String, String), serde_json::Value>>,
    links: RwLock<Vec<(String, MemoryLink)>>,
}

impl InMemoryStore {
    /// Create a new empty store.
    pub fn new() -> Self {
        Self {
            data: RwLock::new(HashMap::new()),
            links: RwLock::new(Vec::new()),
        }
    }
}

impl Default for InMemoryStore {
    fn default() -> Self {
        Self::new()
    }
}

fn scope_key(scope: &Scope) -> String {
    serde_json::to_string(scope).unwrap_or_default()
}

#[async_trait]
impl StateStore for InMemoryStore {
    async fn read(
        &self,
        scope: &Scope,
        key: &str,
    ) -> Result<Option<serde_json::Value>, StateError> {
        let data = self
            .data
            .read()
            .map_err(|e| StateError::Other(e.to_string().into()))?;
        Ok(data.get(&(scope_key(scope), key.to_owned())).cloned())
    }

    async fn write(
        &self,
        scope: &Scope,
        key: &str,
        value: serde_json::Value,
    ) -> Result<(), StateError> {
        let mut data = self
            .data
            .write()
            .map_err(|e| StateError::WriteFailed(e.to_string()))?;
        data.insert((scope_key(scope), key.to_owned()), value);
        Ok(())
    }

    async fn delete(&self, scope: &Scope, key: &str) -> Result<(), StateError> {
        let mut data = self
            .data
            .write()
            .map_err(|e| StateError::WriteFailed(e.to_string()))?;
        data.remove(&(scope_key(scope), key.to_owned()));
        Ok(())
    }

    async fn list(&self, scope: &Scope, prefix: &str) -> Result<Vec<String>, StateError> {
        let data = self
            .data
            .read()
            .map_err(|e| StateError::Other(e.to_string().into()))?;
        let sk = scope_key(scope);
        Ok(data
            .keys()
            .filter(|(s, k)| s == &sk && k.starts_with(prefix))
            .map(|(_, k)| k.clone())
            .collect())
    }

    async fn search(
        &self,
        _scope: &Scope,
        _query: &str,
        _limit: usize,
    ) -> Result<Vec<SearchResult>, StateError> {
        // InMemoryStore doesn't support semantic search
        Ok(vec![])
    }

    async fn link(&self, scope: &Scope, link: &MemoryLink) -> Result<(), StateError> {
        let mut links = self
            .links
            .write()
            .map_err(|e| StateError::WriteFailed(e.to_string()))?;
        links.push((scope_key(scope), link.clone()));
        Ok(())
    }

    async fn unlink(
        &self,
        scope: &Scope,
        from_key: &str,
        to_key: &str,
        relation: &str,
    ) -> Result<(), StateError> {
        let mut links = self
            .links
            .write()
            .map_err(|e| StateError::WriteFailed(e.to_string()))?;
        let sk = scope_key(scope);
        links.retain(|(s, l)| {
            !(s == &sk && l.from_key == from_key && l.to_key == to_key && l.relation == relation)
        });
        Ok(())
    }

    async fn traverse(
        &self,
        scope: &Scope,
        from_key: &str,
        relation: Option<&str>,
        max_depth: u32,
    ) -> Result<Vec<String>, StateError> {
        let links = self
            .links
            .read()
            .map_err(|e| StateError::Other(e.to_string().into()))?;
        let sk = scope_key(scope);

        let mut visited = HashSet::new();
        let mut queue = VecDeque::new();
        let mut result = Vec::new();

        queue.push_back((from_key.to_owned(), 0u32));
        visited.insert(from_key.to_owned());

        while let Some((current, depth)) = queue.pop_front() {
            if depth >= max_depth {
                continue;
            }
            for (s, l) in links.iter() {
                if s != &sk || l.from_key != current {
                    continue;
                }
                if let Some(rel) = relation
                    && l.relation != rel
                {
                    continue;
                }
                if visited.insert(l.to_key.clone()) {
                    result.push(l.to_key.clone());
                    queue.push_back((l.to_key.clone(), depth + 1));
                }
            }
        }

        Ok(result)
    }
}
