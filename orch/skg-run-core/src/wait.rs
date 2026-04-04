//! Durable waiting and resume primitives.
//!
//! [`WaitReason`] and [`ResumeInput`] are re-exported from `layer0::wait`
//! so that durable orchestration and in-process callers share the same types.

pub use layer0::wait::{ResumeInput, WaitReason};
