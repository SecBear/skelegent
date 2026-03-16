//! Durable run and wait-point identifiers.

use serde::{Deserialize, Serialize};
use std::fmt;

macro_rules! typed_id {
    ($name:ident, $doc:expr) => {
        #[doc = $doc]
        #[derive(Debug, Clone, Hash, Eq, PartialEq, Serialize, Deserialize)]
        pub struct $name(pub String);

        impl $name {
            /// Create a new typed identifier from an existing backend-provided string.
            ///
            /// This constructor preserves type safety at the API boundary; it does
            /// not enforce global uniqueness.
            pub fn new(id: impl Into<String>) -> Self {
                Self(id.into())
            }

            /// Borrow the inner identifier string.
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
            fn from(value: &str) -> Self {
                Self(value.to_owned())
            }
        }

        impl From<String> for $name {
            fn from(value: String) -> Self {
                Self(value)
            }
        }
    };
}

typed_id!(RunId, "Typed identifier for a durable run.");
typed_id!(WaitPointId, "Typed identifier for a durable wait point.");

typed_id!(
    CheckpointId,
    "Typed identifier for a durable execution checkpoint."
);
