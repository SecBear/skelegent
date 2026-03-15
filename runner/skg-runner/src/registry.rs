//! Operator registry — maps `OperatorId` to `Arc<dyn Operator>`.

use layer0::{Operator, OperatorId};
use std::collections::HashMap;
use std::sync::Arc;

/// In-memory operator registry. Operators are compiled into the binary
/// and registered at startup. The runner dispatches by operator id.
pub struct OperatorRegistry {
    operators: HashMap<OperatorId, Arc<dyn Operator>>,
}

impl OperatorRegistry {
    /// Start building a registry with a builder pattern.
    pub fn builder() -> OperatorRegistryBuilder {
        OperatorRegistryBuilder {
            operators: HashMap::new(),
        }
    }

    /// Look up an operator by string id.
    pub fn get(&self, id: &str) -> Option<&Arc<dyn Operator>> {
        self.operators.get(&OperatorId::new(id))
    }
}

/// Ergonomic builder for constructing an `OperatorRegistry` at startup.
pub struct OperatorRegistryBuilder {
    operators: HashMap<OperatorId, Arc<dyn Operator>>,
}

impl OperatorRegistryBuilder {
    /// Register an operator into the builder.
    pub fn register(mut self, id: OperatorId, op: Arc<dyn Operator>) -> Self {
        self.operators.insert(id, op);
        self
    }

    /// Finalize the registry.
    pub fn build(self) -> OperatorRegistry {
        OperatorRegistry {
            operators: self.operators,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_registry_returns_none() {
        let reg = OperatorRegistry::builder().build();
        assert!(reg.get("nonexistent").is_none());
    }

    #[test]
    fn builder_produces_empty_registry() {
        let reg = OperatorRegistry::builder().build();
        assert!(reg.get("anything").is_none());
    }
}
