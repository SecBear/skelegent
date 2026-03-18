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
    ///
    /// Not called by the default binary (ships with an empty registry),
    /// but downstream embedders use this to compile operators in.
    #[allow(dead_code)]
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
    use async_trait::async_trait;
    use layer0::{
        Content, DispatchContext, ExitReason, OperatorError, OperatorInput,
        OperatorOutput,
    };

    struct NoOp;

    #[async_trait]
    impl layer0::Operator for NoOp {
        async fn execute(
            &self,
            _input: OperatorInput,
            _ctx: &DispatchContext,
        ) -> Result<OperatorOutput, OperatorError> {
            Ok(OperatorOutput::new(
                Content::text("ok"),
                ExitReason::Complete,
            ))
        }
    }

    #[test]
    fn empty_registry_returns_none() {
        let reg = OperatorRegistry::builder().build();
        assert!(reg.get("nonexistent").is_none());
    }

    #[test]
    fn register_and_lookup() {
        let reg = OperatorRegistry::builder()
            .register(OperatorId::new("test-op"), Arc::new(NoOp))
            .build();
        assert!(reg.get("test-op").is_some());
        assert!(reg.get("other").is_none());
    }
}
