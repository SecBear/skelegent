//! Reducer strategies for concurrent state writes to the same key.
//!
//! When parallel branches both emit [`Effect::WriteMemory`] for the same key,
//! the orchestrator needs a deterministic merge policy. A [`StateReducer`]
//! encodes that policy; a [`ReducerRegistry`] routes keys to the right one.

use serde_json::Value;
use std::collections::HashMap;

/// Merge strategy for concurrent state writes to the same key.
///
/// Implementations decide how an incoming `update` value is merged with
/// the `current` stored value. The result is what gets written.
pub trait StateReducer: Send + Sync {
    /// Merge an incoming update with the current stored value.
    ///
    /// Both values are arbitrary JSON. Implementations must handle every
    /// JSON type combination — at a minimum, produce a defined result
    /// rather than panicking.
    fn reduce(&self, current: &Value, update: &Value) -> Value;
}

/// Last-writer-wins: the incoming update replaces the stored value entirely.
///
/// This is the default reducer and preserves the behavior that existed
/// before reducers were introduced.
pub struct Overwrite;

impl StateReducer for Overwrite {
    fn reduce(&self, _current: &Value, update: &Value) -> Value {
        update.clone()
    }
}

/// Append arrays: concatenates the update array onto the current array.
///
/// If either operand is not an array, falls back to [`Overwrite`] semantics
/// so the call never silently loses data.
pub struct AppendList;

impl StateReducer for AppendList {
    fn reduce(&self, current: &Value, update: &Value) -> Value {
        match (current, update) {
            (Value::Array(cur), Value::Array(upd)) => {
                let mut merged = cur.clone();
                merged.extend(upd.iter().cloned());
                Value::Array(merged)
            }
            // Non-array fallback: treat as overwrite so we don't silently drop data.
            _ => update.clone(),
        }
    }
}

/// Deep merge objects: recursively merges update keys into current object.
///
/// Keys present only in `current` are preserved. Keys present only in
/// `update` are added. Keys present in both recurse into this reducer.
/// If either operand is not an object, falls back to [`Overwrite`] semantics.
pub struct MergeObject;

impl MergeObject {
    fn merge_values(current: &Value, update: &Value) -> Value {
        match (current, update) {
            (Value::Object(cur), Value::Object(upd)) => {
                let mut merged = cur.clone();
                for (k, v) in upd {
                    let entry = merged.entry(k.clone()).or_insert(Value::Null);
                    *entry = Self::merge_values(entry, v);
                }
                Value::Object(merged)
            }
            // For non-object pairs, the update wins.
            _ => update.clone(),
        }
    }
}

impl StateReducer for MergeObject {
    fn reduce(&self, current: &Value, update: &Value) -> Value {
        Self::merge_values(current, update)
    }
}

/// Sum numeric values: adds the update number to the current number.
///
/// Both operands must be JSON numbers. If either is not a number,
/// falls back to [`Overwrite`] semantics.
///
/// The result is an `f64` JSON number. Integer precision is preserved
/// as long as both operands are exact `f64` values (i.e., not larger
/// than 2^53).
pub struct Sum;

impl StateReducer for Sum {
    fn reduce(&self, current: &Value, update: &Value) -> Value {
        match (current.as_f64(), update.as_f64()) {
            (Some(a), Some(b)) => {
                // serde_json represents the sum as a JSON number.
                // `json!` round-trips correctly for typical agent counters.
                serde_json::json!(a + b)
            }
            // Non-numeric fallback: overwrite.
            _ => update.clone(),
        }
    }
}

/// Routes state-key writes to the appropriate [`StateReducer`].
///
/// Keys are matched by exact string equality. Keys without a registered
/// reducer use the registry's default (initially [`Overwrite`]).
///
/// # Example
/// ```rust
/// use layer0::reducer::{ReducerRegistry, AppendList};
///
/// let registry = ReducerRegistry::new()
///     .register("events", AppendList);
///
/// let current = serde_json::json!(["a"]);
/// let update  = serde_json::json!(["b"]);
/// let result  = registry.reduce("events", &current, &update);
/// assert_eq!(result, serde_json::json!(["a", "b"]));
/// ```
pub struct ReducerRegistry {
    reducers: HashMap<String, Box<dyn StateReducer>>,
    default: Box<dyn StateReducer>,
}

impl Default for ReducerRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl ReducerRegistry {
    /// Create a registry with [`Overwrite`] as the default reducer.
    pub fn new() -> Self {
        Self {
            reducers: HashMap::new(),
            default: Box::new(Overwrite),
        }
    }

    /// Register a reducer for an exact key, replacing any previous entry.
    ///
    /// Returns `self` for builder-style chaining.
    pub fn register(
        mut self,
        key: impl Into<String>,
        reducer: impl StateReducer + 'static,
    ) -> Self {
        self.reducers.insert(key.into(), Box::new(reducer));
        self
    }

    /// Replace the default reducer used for keys without an explicit entry.
    ///
    /// Returns `self` for builder-style chaining.
    pub fn with_default(mut self, reducer: impl StateReducer + 'static) -> Self {
        self.default = Box::new(reducer);
        self
    }

    /// Reduce `update` into `current` using the reducer registered for `key`,
    /// or the default reducer if no entry exists.
    pub fn reduce(&self, key: &str, current: &Value, update: &Value) -> Value {
        self.reducers
            .get(key)
            .map(|r| r.as_ref())
            .unwrap_or(self.default.as_ref())
            .reduce(current, update)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn overwrite_replaces() {
        let r = Overwrite;
        let current = json!({"x": 1});
        let update = json!({"y": 2});
        assert_eq!(r.reduce(&current, &update), json!({"y": 2}));
    }

    #[test]
    fn append_list_merges() {
        let r = AppendList;
        let current = json!([1, 2]);
        let update = json!([3, 4]);
        assert_eq!(r.reduce(&current, &update), json!([1, 2, 3, 4]));
    }

    #[test]
    fn merge_object_deep() {
        let r = MergeObject;
        let current = json!({"a": 1, "nested": {"x": 10}});
        let update = json!({"b": 2, "nested": {"y": 20}});
        let result = r.reduce(&current, &update);
        assert_eq!(
            result,
            json!({"a": 1, "b": 2, "nested": {"x": 10, "y": 20}})
        );
    }

    #[test]
    fn sum_adds() {
        let r = Sum;
        let current = json!(5);
        let update = json!(3);
        assert_eq!(r.reduce(&current, &update), json!(8.0));
    }

    #[test]
    fn registry_routes_by_key() {
        let registry = ReducerRegistry::new().register("items", AppendList);

        // "items" key uses AppendList
        let result = registry.reduce("items", &json!([1, 2]), &json!([3, 4]));
        assert_eq!(result, json!([1, 2, 3, 4]));

        // unknown key falls back to Overwrite
        let result = registry.reduce("other", &json!({"old": true}), &json!({"new": true}));
        assert_eq!(result, json!({"new": true}));
    }
}
