//! Bidirectional conversions between A2A wire types and skelegent-native types.
//!
//! These are standalone functions (not `From`/`Into` impls) because the
//! conversions are lossy or context-dependent: A2A `Canceled` vs skelegent
//! `Cancelled`, base64 images with no clean A2A mapping, ToolUse/ToolResult
//! blocks that don't exist in the A2A content model.

use layer0::content::{Content, ContentBlock, ContentSource};
use layer0::dispatch::Artifact;
use layer0::operator::{OperatorInput, OperatorOutput, TriggerType};
use serde_json::json;
use skg_run_core::model::RunStatus;

use crate::types::*;

// ---------------------------------------------------------------------------
// Part ↔ ContentBlock
// ---------------------------------------------------------------------------

/// Convert a single [`Part`] into a [`ContentBlock`].
///
/// Because `PartContent::Url` can map to either `ContentBlock::Image` or
/// `ContentBlock::File`, the media type from the parent `Part` is used to
/// disambiguate: media types starting with `image/` become images.
fn part_to_content_block(part: &Part) -> ContentBlock {
    match &part.content {
        PartContent::Text { text } => ContentBlock::Text { text: text.clone() },
        PartContent::Url { url } => {
            let media_type = part
                .media_type
                .clone()
                .unwrap_or_else(|| "application/octet-stream".into());
            if media_type.starts_with("image/") {
                ContentBlock::Image {
                    source: ContentSource::Url { url: url.clone() },
                    media_type,
                }
            } else {
                ContentBlock::File {
                    source: ContentSource::Url { url: url.clone() },
                    media_type,
                    filename: part.filename.clone(),
                }
            }
        }
        PartContent::Raw { raw } => {
            let media_type = part
                .media_type
                .clone()
                .unwrap_or_else(|| "application/octet-stream".into());
            ContentBlock::File {
                source: ContentSource::Base64 { data: raw.clone() },
                media_type,
                filename: part.filename.clone(),
            }
        }
        PartContent::Data { data } => ContentBlock::Data {
            data: data.clone(),
            media_type: Some("application/json".into()),
        },
    }
}

/// Convert a [`ContentBlock`] into a [`Part`].
///
/// Lossy conversions:
/// - `ContentBlock::Image` with `Base64` source maps to a `Raw` part carrying
///   the base64 string directly in `raw`; media type is preserved.
/// - `ContentBlock::ToolUse` is serialized as a structured `Data` part.
/// - `ContentBlock::ToolResult` is flattened to a text part.
/// - `ContentBlock::Custom` is serialized as a `Data` part.
fn content_block_to_part(block: &ContentBlock) -> Part {
    match block {
        ContentBlock::Text { text } => Part::text(text.clone()),

        ContentBlock::Image {
            source: ContentSource::Url { url },
            media_type,
        } => Part {
            content: PartContent::Url { url: url.clone() },
            media_type: Some(media_type.clone()),
            filename: None,
            metadata: None,
        },

        ContentBlock::Image {
            source: ContentSource::Base64 { data },
            media_type,
        } => Part {
            content: PartContent::Raw { raw: data.clone() },
            media_type: Some(media_type.clone()),
            filename: None,
            metadata: None,
        },

        ContentBlock::File {
            source: ContentSource::Url { url },
            media_type,
            filename,
        } => Part {
            content: PartContent::Url { url: url.clone() },
            media_type: Some(media_type.clone()),
            filename: filename.clone(),
            metadata: None,
        },

        ContentBlock::File {
            source: ContentSource::Base64 { data },
            media_type,
            filename,
        } => Part {
            content: PartContent::Raw { raw: data.clone() },
            media_type: Some(media_type.clone()),
            filename: filename.clone(),
            metadata: None,
        },

        ContentBlock::Data { data, media_type } => Part {
            content: PartContent::Data { data: data.clone() },
            media_type: media_type.clone(),
            filename: None,
            metadata: None,
        },

        ContentBlock::ToolUse { id, name, input } => Part {
            content: PartContent::Data {
                data: json!({
                    "tool_use": {
                        "id": id,
                        "name": name,
                        "input": input,
                    }
                }),
            },
            media_type: Some("application/json".into()),
            filename: None,
            metadata: None,
        },

        ContentBlock::ToolResult {
            tool_use_id,
            content,
            is_error,
        } => {
            let text = if *is_error {
                format!("[error] {content}")
            } else {
                content.clone()
            };
            let mut part = Part::text(text);
            let mut map = serde_json::Map::new();
            map.insert(
                "tool_use_id".into(),
                serde_json::Value::String(tool_use_id.clone()),
            );
            part.metadata = Some(map);
            part
        }

        ContentBlock::Custom { content_type, data } => Part {
            content: PartContent::Data { data: data.clone() },
            media_type: Some(content_type.clone()),
            filename: None,
            metadata: None,
        },

        // Non-exhaustive future variants: best-effort as empty data.
        _ => Part {
            content: PartContent::Data {
                data: serde_json::Value::Null,
            },
            media_type: None,
            filename: None,
            metadata: None,
        },
    }
}

// ---------------------------------------------------------------------------
// Vec<Part> ↔ Content
// ---------------------------------------------------------------------------

/// Convert A2A message parts into skelegent [`Content`].
///
/// If there is exactly one text part, the result is `Content::Text`.
/// Otherwise each part is converted to a [`ContentBlock`] and wrapped
/// in `Content::Blocks`.
pub fn parts_to_content(parts: &[Part]) -> Content {
    if parts.len() == 1
        && let PartContent::Text { text } = &parts[0].content
    {
        return Content::Text(text.clone());
    }
    Content::Blocks(parts.iter().map(part_to_content_block).collect())
}

/// Convert skelegent [`Content`] into A2A message parts.
///
/// `Content::Text` yields a single text part. `Content::Blocks` converts
/// each block individually.
pub fn content_to_parts(content: &Content) -> Vec<Part> {
    match content {
        Content::Text(s) => vec![Part::text(s.clone())],
        Content::Blocks(blocks) => blocks.iter().map(content_block_to_part).collect(),
        _ => vec![Part::text(String::new())],
    }
}

// ---------------------------------------------------------------------------
// TaskState ↔ RunStatus
// ---------------------------------------------------------------------------

/// Convert an A2A [`TaskState`] to a skelegent [`RunStatus`].
///
/// Lossy mappings:
/// - `Unspecified` / `Submitted` → `Running` (not-yet-known defaults to running)
/// - `Rejected` → `Failed`
/// - `InputRequired` / `AuthRequired` → `Waiting`
/// - `Canceled` (one L) → `Cancelled` (two L's)
pub fn task_state_to_run_status(state: TaskState) -> RunStatus {
    #[allow(unreachable_patterns)]
    match state {
        TaskState::Unspecified | TaskState::Working | TaskState::Submitted => RunStatus::Running,
        TaskState::InputRequired | TaskState::AuthRequired => RunStatus::Waiting,
        TaskState::Completed => RunStatus::Completed,
        TaskState::Failed | TaskState::Rejected => RunStatus::Failed,
        TaskState::Canceled => RunStatus::Cancelled,
        _ => RunStatus::Running,
    }
}

/// Convert a skelegent [`RunStatus`] to an A2A [`TaskState`].
///
/// Lossy: `Waiting` maps to `InputRequired` (could also be `AuthRequired`
/// depending on wait reason, but we lose that distinction here).
pub fn run_status_to_task_state(status: RunStatus) -> TaskState {
    #[allow(unreachable_patterns)]
    match status {
        RunStatus::Running => TaskState::Working,
        RunStatus::Waiting => TaskState::InputRequired,
        RunStatus::Completed => TaskState::Completed,
        RunStatus::Failed => TaskState::Failed,
        RunStatus::Cancelled => TaskState::Canceled,
        _ => TaskState::Working,
    }
}

// ---------------------------------------------------------------------------
// A2aArtifact ↔ Artifact
// ---------------------------------------------------------------------------

/// Convert an A2A [`A2aArtifact`] into a layer0 [`Artifact`].
///
/// Parts are grouped into a single [`Content`] value via [`parts_to_content`].
/// Streaming fields (`append`, `last_chunk`) default to `false` and `true`
/// respectively since A2A has no streaming artifact semantics.
pub fn a2a_artifact_to_artifact(artifact: &A2aArtifact) -> Artifact {
    let parts = vec![parts_to_content(&artifact.parts)];
    let mut a = Artifact::new(artifact.artifact_id.clone(), parts);
    if let Some(name) = &artifact.name {
        a = a.with_name(name.clone());
    }
    if let Some(desc) = &artifact.description {
        a = a.with_description(desc.clone());
    }
    if let Some(meta) = &artifact.metadata {
        a = a.with_metadata(meta.clone());
    }
    a
}

/// Convert a layer0 [`Artifact`] into an A2A [`A2aArtifact`].
///
/// Each `Content` in `parts` is flattened into A2A parts via [`content_to_parts`].
/// The `append` and `last_chunk` fields are dropped (no A2A equivalent).
pub fn artifact_to_a2a_artifact(artifact: &Artifact) -> A2aArtifact {
    let parts: Vec<Part> = artifact.parts.iter().flat_map(content_to_parts).collect();
    A2aArtifact {
        artifact_id: artifact.id.clone(),
        name: artifact.name.clone(),
        description: artifact.description.clone(),
        parts,
        metadata: artifact.metadata.clone(),
        extensions: Vec::new(),
    }
}

// ---------------------------------------------------------------------------
// A2aMessage → OperatorInput
// ---------------------------------------------------------------------------

/// Convert an A2A [`A2aMessage`] into a skelegent [`OperatorInput`].
///
/// The message role determines the trigger type:
/// - `A2aRole::Unspecified` / `A2aRole::User` → `TriggerType::User`
/// - `A2aRole::Agent` → `TriggerType::Task`
///
/// Metadata from the A2A message is passed through as-is.
pub fn a2a_message_to_operator_input(msg: &A2aMessage) -> OperatorInput {
    #[allow(unreachable_patterns)]
    let trigger = match msg.role {
        A2aRole::Unspecified | A2aRole::User => TriggerType::User,
        A2aRole::Agent => TriggerType::Task,
        _ => TriggerType::User,
    };
    let mut input = OperatorInput::new(parts_to_content(&msg.parts), trigger);
    input.metadata = msg.metadata.clone().unwrap_or(serde_json::Value::Null);
    input
}

// ---------------------------------------------------------------------------
// OperatorOutput → A2aMessage
// ---------------------------------------------------------------------------

/// Convert a skelegent [`OperatorOutput`] into an A2A [`A2aMessage`].
///
/// The resulting message always has `A2aRole::Agent` and a freshly generated
/// `message_id`. Context and task IDs are left empty — the caller fills them
/// from request context.
pub fn operator_output_to_a2a_message(output: &OperatorOutput) -> A2aMessage {
    A2aMessage {
        message_id: uuid::Uuid::new_v4().to_string(),
        role: A2aRole::Agent,
        parts: content_to_parts(&output.message),
        context_id: None,
        task_id: None,
        metadata: Some(serde_json::to_value(&output.metadata).unwrap_or_default()),
        extensions: Vec::new(),
        reference_task_ids: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_round_trip() {
        let content = Content::Text("hello world".into());
        let parts = content_to_parts(&content);
        let back = parts_to_content(&parts);
        assert_eq!(content, back);
    }

    #[test]
    fn single_text_part_collapses() {
        let parts = vec![Part::text("single")];
        let content = parts_to_content(&parts);
        assert!(matches!(content, Content::Text(ref s) if s == "single"));
    }

    #[test]
    fn multi_part_becomes_blocks() {
        let parts = vec![Part::text("a"), Part::text("b")];
        let content = parts_to_content(&parts);
        assert!(matches!(content, Content::Blocks(ref bs) if bs.len() == 2));
    }

    #[test]
    fn task_state_covers_all_variants() {
        let cases = [
            (TaskState::Unspecified, RunStatus::Running),
            (TaskState::Submitted, RunStatus::Running),
            (TaskState::Working, RunStatus::Running),
            (TaskState::Completed, RunStatus::Completed),
            (TaskState::Failed, RunStatus::Failed),
            (TaskState::Canceled, RunStatus::Cancelled),
            (TaskState::InputRequired, RunStatus::Waiting),
            (TaskState::AuthRequired, RunStatus::Waiting),
            (TaskState::Rejected, RunStatus::Failed),
        ];
        for (state, expected) in cases {
            assert_eq!(task_state_to_run_status(state), expected, "{state:?}");
        }
    }

    #[test]
    fn run_status_round_trips_where_lossless() {
        // Running, Completed, Failed, Cancelled are lossless
        for status in [
            RunStatus::Running,
            RunStatus::Completed,
            RunStatus::Failed,
            RunStatus::Cancelled,
        ] {
            let back = task_state_to_run_status(run_status_to_task_state(status));
            assert_eq!(status, back, "{status:?}");
        }
    }

    #[test]
    fn url_part_image_detection() {
        let part = Part {
            content: PartContent::Url {
                url: "https://example.com/cat.png".into(),
            },
            media_type: Some("image/png".into()),
            filename: None,
            metadata: None,
        };
        let block = part_to_content_block(&part);
        assert!(matches!(block, ContentBlock::Image { .. }));
    }

    #[test]
    fn url_part_file_detection() {
        let part = Part {
            content: PartContent::Url {
                url: "https://example.com/doc.pdf".into(),
            },
            media_type: Some("application/pdf".into()),
            filename: None,
            metadata: None,
        };
        let block = part_to_content_block(&part);
        assert!(matches!(block, ContentBlock::File { .. }));
    }

    #[test]
    fn data_part_conversion() {
        let part = Part {
            content: PartContent::Data {
                data: json!({"key": "value"}),
            },
            media_type: None,
            filename: None,
            metadata: None,
        };
        let block = part_to_content_block(&part);
        match block {
            ContentBlock::Data { data, media_type } => {
                assert_eq!(data, json!({"key": "value"}));
                assert_eq!(media_type.as_deref(), Some("application/json"));
            }
            other => panic!("expected Data, got {other:?}"),
        }
    }

    #[test]
    fn raw_part_becomes_file_block() {
        let part = Part {
            content: PartContent::Raw {
                raw: "c29tZSBiaW5hcnk=".into(),
            },
            media_type: Some("application/pdf".into()),
            filename: Some("doc.pdf".into()),
            metadata: None,
        };
        let block = part_to_content_block(&part);
        match block {
            ContentBlock::File {
                source: ContentSource::Base64 { data },
                media_type,
                filename,
            } => {
                assert_eq!(data, "c29tZSBiaW5hcnk=");
                assert_eq!(media_type, "application/pdf");
                assert_eq!(filename.as_deref(), Some("doc.pdf"));
            }
            other => panic!("expected File with Base64, got {other:?}"),
        }
    }

    #[test]
    fn unspecified_role_maps_to_user_trigger() {
        let msg = A2aMessage {
            message_id: "test".into(),
            context_id: None,
            task_id: None,
            role: A2aRole::Unspecified,
            parts: vec![Part::text("hello")],
            metadata: None,
            extensions: Vec::new(),
            reference_task_ids: Vec::new(),
        };
        let input = a2a_message_to_operator_input(&msg);
        assert!(matches!(input.trigger, TriggerType::User));
    }
    #[test]
    fn base64_image_block_maps_to_raw_part() {
        // Regression: base64 image must become Raw, not Data({"base64": ...})
        let block = ContentBlock::Image {
            source: ContentSource::Base64 {
                data: "SGVsbG8=".into(),
            },
            media_type: "image/png".into(),
        };
        let part = content_block_to_part(&block);
        assert_eq!(part.media_type.as_deref(), Some("image/png"));
        match part.content {
            PartContent::Raw { raw } => assert_eq!(raw, "SGVsbG8="),
            other => panic!("expected Raw, got {other:?}"),
        }
    }

}
