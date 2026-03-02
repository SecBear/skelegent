//! Barrier-scheduled custom operator example.
//!
//! This crate demonstrates a minimal Operator that batches tool calls between
//! barriers and injects a steering message between batches.
//!
//! The operator does not call a live provider — it consumes `Content::Blocks`
//! with `tool_use` items (as if produced by a model) and emits corresponding
//! `tool_result` blocks. Any `text` block with the literal "BARRIER" acts as a
//! flush point.
//!
//! Example:
//!
//! ```rust
//! use std::sync::Arc;
//! use layer0::content::{Content, ContentBlock};
//! use layer0::operator::{Operator, OperatorInput, TriggerType, ExitReason};
//! use neuron_tool::{ToolRegistry, ToolDyn, ToolError};
//! use serde_json::{json, Value};
//! use std::pin::Pin;
//! use std::future::Future;
//!
//! struct Echo;
//! impl ToolDyn for Echo {
//!     fn name(&self) -> &str { "echo" }
//!     fn description(&self) -> &str { "echoes input" }
//!     fn input_schema(&self) -> Value { json!({"type": "object"}) }
//!     fn call(&self, input: Value) -> Pin<Box<dyn Future<Output = Result<Value, ToolError>> + Send + '_>> {
//!         Box::pin(async move { Ok(json!({"echo": input})) })
//!     }
//! }
//!
//! # tokio::runtime::Runtime::new().unwrap().block_on(async {
//! let mut tools = ToolRegistry::new();
//! tools.register(Arc::new(Echo));
//!
//! let op = custom_operator_barrier::BarrierOperator::new(tools);
//! let input = OperatorInput::new(
//!     Content::Blocks(vec![
//!         ContentBlock::ToolUse { id: "1".into(), name: "echo".into(), input: json!({"a":1}) },
//!         ContentBlock::Text { text: "BARRIER".into() },
//!         ContentBlock::ToolUse { id: "2".into(), name: "echo".into(), input: json!({"b":2}) },
//!     ]),
//!     TriggerType::Task,
//! );
//!
//! let out = op.execute(input).await.unwrap();
//! assert_eq!(out.exit_reason, ExitReason::Complete);
//! if let Content::Blocks(blocks) = out.message {
//!     let results = blocks.iter().filter(|b| matches!(b, ContentBlock::ToolResult{..})).count();
//!     assert_eq!(results, 2);
//! }
//! # });
//! ```

use async_trait::async_trait;
use layer0::content::{Content, ContentBlock};
use layer0::duration::DurationMs;
use layer0::effect::Effect;
use layer0::error::OperatorError;
use layer0::operator::{ExitReason, Operator, OperatorInput, OperatorOutput, ToolCallRecord};
use neuron_tool::ToolRegistry;

/// A minimal operator that batches tool calls between barriers.
pub struct BarrierOperator {
    tools: ToolRegistry,
}

impl BarrierOperator {
    /// Create a new barrier operator with a tool registry.
    pub fn new(tools: ToolRegistry) -> Self {
        Self { tools }
    }
}

#[async_trait]
impl Operator for BarrierOperator {
    async fn execute(&self, input: OperatorInput) -> Result<OperatorOutput, OperatorError> {
        let mut out_blocks: Vec<ContentBlock> = Vec::new();
        let mut batch: Vec<(String, String, serde_json::Value)> = Vec::new();
        let mut metadata = layer0::operator::OperatorMetadata::default();

        // Helper to flush a batch of tool calls
        async fn flush(
            tools: &ToolRegistry,
            batch: &mut Vec<(String, String, serde_json::Value)>,
            out_blocks: &mut Vec<ContentBlock>,
            metadata: &mut layer0::operator::OperatorMetadata,
        ) {
            if batch.is_empty() {
                return;
            }
            // Execute sequentially for simplicity; a real operator could parallelize
            for (id, name, params) in batch.drain(..) {
                let start = std::time::Instant::now();
                if let Some(tool) = tools.get(&name) {
                    match tool.call(params).await {
                        Ok(val) => {
                            let content = val.to_string();
                            out_blocks.push(ContentBlock::ToolResult {
                                tool_use_id: id,
                                content,
                                is_error: false,
                            });
                            metadata.tools_called.push(ToolCallRecord::new(
                                name,
                                DurationMs::from_millis(start.elapsed().as_millis() as u64),
                                true,
                            ));
                        }
                        Err(e) => {
                            out_blocks.push(ContentBlock::ToolResult {
                                tool_use_id: id,
                                content: e.to_string(),
                                is_error: true,
                            });
                            metadata.tools_called.push(ToolCallRecord::new(
                                name,
                                DurationMs::from_millis(start.elapsed().as_millis() as u64),
                                false,
                            ));
                        }
                    }
                } else {
                    let msg = format!("tool not found: {}", name);
                    out_blocks.push(ContentBlock::ToolResult {
                        tool_use_id: id,
                        content: msg.clone(),
                        is_error: true,
                    });
                    metadata
                        .tools_called
                        .push(ToolCallRecord::new(name, DurationMs::ZERO, false));
                }
            }
            // Inject a steering message after each batch flush
            out_blocks.push(ContentBlock::Text {
                text: "[steer] batch flushed".into(),
            });
        }

        // Interpret the incoming content as a scripted sequence
        match input.message {
            Content::Text(t) => {
                out_blocks.push(ContentBlock::Text { text: t });
            }
            Content::Blocks(blocks) => {
                for b in blocks {
                    match b {
                        ContentBlock::ToolUse { id, name, input } => {
                            batch.push((id, name, input));
                        }
                        ContentBlock::Text { text } if text.trim() == "BARRIER" => {
                            flush(&self.tools, &mut batch, &mut out_blocks, &mut metadata).await;
                        }
                        other => {
                            // Pass-through any other content blocks
                            out_blocks.push(other);
                        }
                    }
                }
                // Final flush
                flush(&self.tools, &mut batch, &mut out_blocks, &mut metadata).await;
            }
            _ => { /* ignore unknown content kinds */ }
        }

        let mut output = OperatorOutput::new(Content::Blocks(out_blocks), ExitReason::Complete);
        output.metadata = metadata;
        // Demonstrate effect declaration boundary (no-op here)
        output.effects.push(Effect::Log {
            level: layer0::effect::LogLevel::Info,
            message: "barrier operator executed".into(),
            data: None,
        });
        Ok(output)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use neuron_tool::{ToolDyn, ToolError};
    use serde_json::json;
    use std::sync::Arc;

    struct Echo;
    impl ToolDyn for Echo {
        fn name(&self) -> &str {
            "echo"
        }
        fn description(&self) -> &str {
            "echoes input"
        }
        fn input_schema(&self) -> serde_json::Value {
            json!({"type":"object"})
        }
        fn call(
            &self,
            input: serde_json::Value,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<serde_json::Value, ToolError>> + Send + '_>,
        > {
            Box::pin(async move { Ok(json!({"ok": true, "input": input})) })
        }
    }

    #[tokio::test]
    async fn batches_and_injects() {
        let mut reg = ToolRegistry::new();
        reg.register(Arc::new(Echo));
        let op = BarrierOperator::new(reg);

        let input = OperatorInput::new(
            Content::Blocks(vec![
                ContentBlock::ToolUse {
                    id: "1".into(),
                    name: "echo".into(),
                    input: json!({"a":1}),
                },
                ContentBlock::ToolUse {
                    id: "2".into(),
                    name: "echo".into(),
                    input: json!({"b":2}),
                },
                ContentBlock::Text {
                    text: "BARRIER".into(),
                },
                ContentBlock::ToolUse {
                    id: "3".into(),
                    name: "echo".into(),
                    input: json!({"c":3}),
                },
            ]),
            layer0::operator::TriggerType::Task,
        );

        let out = op.execute(input).await.unwrap();
        match out.message {
            Content::Blocks(blocks) => {
                // Expect 4 tool results + 2 steering texts (after each flush)
                // First batch: 2 results + steer; second batch: 1 result + steer
                let texts = blocks
                    .iter()
                    .filter(|b| matches!(b, ContentBlock::Text { .. }))
                    .count();
                let results = blocks
                    .iter()
                    .filter(|b| matches!(b, ContentBlock::ToolResult { .. }))
                    .count();
                assert_eq!(texts, 2);
                assert_eq!(results, 3);
            }
            _ => panic!("expected blocks"),
        }
        assert_eq!(out.exit_reason, ExitReason::Complete);
        assert_eq!(out.effects.len(), 1);
    }
}
