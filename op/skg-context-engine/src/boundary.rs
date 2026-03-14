//! Typed governance markers for provider boundaries.
//!
//! Provider calls are not [`ContextOp`](crate::ContextOp)s, but they still need
//! first-class rule targets so pre-inference governance can happen immediately
//! around the actual network boundary.

/// Typed boundary for a non-streaming provider inference call.
#[derive(Debug, Clone, Copy, Default)]
pub struct InferBoundary;

/// Typed boundary for a streaming provider inference call.
#[derive(Debug, Clone, Copy, Default)]
pub struct StreamInferBoundary;
