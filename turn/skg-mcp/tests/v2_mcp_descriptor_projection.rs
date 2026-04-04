use rmcp::model::{Annotated, Prompt, RawResource, Tool};
use serde_json::json;
use skg_mcp::client::{
    descriptor_from_mcp_prompt, descriptor_from_mcp_resource, descriptor_from_mcp_tool,
};
use std::borrow::Cow;
use std::sync::Arc;

#[test]
fn mcp_tool_projects_to_canonical_descriptor() {
    let tool = Tool {
        name: Cow::Owned("web_search".to_string()),
        title: None,
        description: Some(Cow::Owned("Search the web".to_string())),
        input_schema: Arc::new(
            json!({
                "type": "object",
                "properties": {"query": {"type": "string"}}
            })
            .as_object()
            .expect("schema object")
            .clone(),
        ),
        output_schema: Some(Arc::new(
            json!({
                "type": "object",
                "properties": {"result": {"type": "string"}}
            })
            .as_object()
            .expect("schema object")
            .clone(),
        )),
        annotations: None,
        execution: None,
        icons: None,
        meta: None,
    };

    let descriptor = descriptor_from_mcp_tool(&tool);
    assert_eq!(descriptor.kind, layer0::CapabilityKind::Tool);
    assert_eq!(descriptor.id.as_str(), "mcp-tool:web_search");
    assert_eq!(descriptor.extensions["mcp"]["kind"], json!("tool"));

    let got = serde_json::to_value(&descriptor).expect("serialize");
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("golden/v2/mcp-tool-descriptor.json"))
            .expect("fixture json");
    assert_eq!(got, fixture);
}

#[test]
fn mcp_prompt_and_resource_project_to_canonical_descriptors() {
    let prompt = Prompt {
        name: "greet".to_string(),
        title: None,
        description: Some("Greeting prompt".to_string()),
        arguments: None,
        icons: None,
        meta: None,
    };
    let prompt_descriptor = descriptor_from_mcp_prompt(&prompt);
    assert_eq!(prompt_descriptor.kind, layer0::CapabilityKind::Prompt);
    assert_eq!(prompt_descriptor.extensions["mcp"]["kind"], json!("prompt"));

    let prompt_json = serde_json::to_value(&prompt_descriptor).expect("serialize");
    let prompt_fixture: serde_json::Value =
        serde_json::from_str(include_str!("golden/v2/mcp-prompt-descriptor.json"))
            .expect("fixture json");
    assert_eq!(prompt_json, prompt_fixture);

    let resource = Annotated::new(
        RawResource {
            uri: "state://global/config".to_string(),
            name: "config".to_string(),
            title: None,
            description: Some("Server configuration".to_string()),
            mime_type: Some("application/json".into()),
            size: None,
            icons: None,
            meta: None,
        },
        None,
    );
    let resource_descriptor = descriptor_from_mcp_resource(&resource);
    assert_eq!(resource_descriptor.kind, layer0::CapabilityKind::Resource);
    assert_eq!(
        resource_descriptor.extensions["mcp"]["kind"],
        json!("resource")
    );

    let resource_json = serde_json::to_value(&resource_descriptor).expect("serialize");
    let resource_fixture: serde_json::Value =
        serde_json::from_str(include_str!("golden/v2/mcp-resource-descriptor.json"))
            .expect("fixture json");
    assert_eq!(resource_json, resource_fixture);
}
