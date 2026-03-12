//! Typed ID wrappers for operator, session, workflow, and scope identifiers.

use serde::{Deserialize, Serialize};
use std::fmt;

/// Typed ID wrappers prevent mixing up operator IDs, session IDs, etc.
/// These are just strings underneath — no UUID enforcement, no format
/// requirement. The protocol doesn't care what your IDs look like.
macro_rules! typed_id {
    ($name:ident, $doc:expr) => {
        #[doc = $doc]
        #[derive(Debug, Clone, Hash, Eq, PartialEq, Serialize, Deserialize)]
        pub struct $name(pub String);

        impl $name {
            /// Create a new typed ID from anything that converts to String.
            pub fn new(id: impl Into<String>) -> Self {
                Self(id.into())
            }

            /// Borrow the inner string.
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}", self.0)
            }
        }

        impl From<&str> for $name {
            fn from(s: &str) -> Self {
                Self(s.to_owned())
            }
        }

        impl From<String> for $name {
            fn from(s: String) -> Self {
                Self(s)
            }
        }
    };
}

typed_id!(OperatorId, "Unique identifier for an operator.");
typed_id!(SessionId, "Unique identifier for a conversation session.");
typed_id!(WorkflowId, "Unique identifier for a workflow execution.");
