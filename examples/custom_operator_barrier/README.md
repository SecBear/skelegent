# Custom Operator: Barrier Scheduling and Steering

This example demonstrates a minimal custom Operator that batches `tool_use` blocks between barriers, flushes them as a batch, and injects a steering message after each flush. It does not call a live provider. Instead, the operator reads `Content::Blocks` with `tool_use` items and produces `tool_result` items.

See `src/lib.rs` for the implementation and unit test.
