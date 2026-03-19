#![deny(missing_docs)]
//! Convert OpenAPI 3.x specifications to [`ToolDyn`] implementations.
//!
//! Parse an OpenAPI spec (JSON or YAML) and generate a [`ToolDyn`] per
//! operation. Each generated tool makes HTTP calls to the API using the
//! supplied [`ApiAuth`] provider and a shared [`reqwest::Client`].
//!
//! # Example
//!
//! ```no_run
//! use skg_tool_openapi::{from_openapi, BearerAuth, OpenApiConfig, NoAuth};
//! use std::sync::Arc;
//!
//! let spec = r#"{"openapi":"3.0.0","info":{"title":"T","version":"1"},"paths":{}}"#;
//! let config = OpenApiConfig {
//!     base_url: "https://api.example.com".into(),
//!     tag_filter: None,
//!     path_prefix: None,
//!     transform_schemas: false,
//! };
//! let tools = from_openapi(spec, config, Arc::new(NoAuth)).unwrap();
//! assert!(tools.is_empty());
//! ```

use std::collections::HashSet;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use layer0::DispatchContext;
use openapiv3::{
    OpenAPI, Parameter, ParameterSchemaOrContent, PathItem, ReferenceOr, RequestBody, Schema,
};
use serde_json::{json, Value};
use skg_tool::{schema as skg_schema, ToolDyn, ToolError};

// ─── Public API ───────────────────────────────────────────────────────────────

/// Configuration for OpenAPI tool generation.
#[derive(Debug, Clone)]
pub struct OpenApiConfig {
    /// Base URL for API calls (e.g. `"https://api.example.com"`).
    pub base_url: String,
    /// Optional tag filter — only generate tools for operations that carry at
    /// least one of these tags.  `None` means no filtering.
    pub tag_filter: Option<Vec<String>>,
    /// Optional path prefix filter — only include paths that start with this
    /// string (e.g. `"/v2"`).  `None` means no filtering.
    pub path_prefix: Option<String>,
    /// When `true`, run generated input schemas through
    /// [`skg_tool::schema::transform_for_llm`] to strip unsupported keywords
    /// and simplify single-variant unions before storing them.
    pub transform_schemas: bool,
}

/// Auth provider for HTTP calls made by generated tools.
///
/// Implementations receive a [`reqwest::RequestBuilder`] and return it with
/// any required authentication applied (header injection, query params, etc.).
pub trait ApiAuth: Send + Sync {
    /// Apply authentication to a request builder.
    fn apply(&self, request: reqwest::RequestBuilder) -> reqwest::RequestBuilder;
}

/// Bearer-token authentication: injects `Authorization: Bearer <token>`.
pub struct BearerAuth(
    /// The bearer token value (without the `Bearer ` prefix).
    pub String,
);

impl ApiAuth for BearerAuth {
    fn apply(&self, request: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        request.bearer_auth(&self.0)
    }
}

/// No-op auth provider — passes requests through unchanged.
pub struct NoAuth;

impl ApiAuth for NoAuth {
    fn apply(&self, request: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        request
    }
}

/// Errors returned by [`from_openapi`].
#[non_exhaustive]
#[derive(Debug, thiserror::Error)]
pub enum OpenApiError {
    /// The spec string could not be parsed as valid OpenAPI JSON or YAML.
    #[error("failed to parse OpenAPI spec: {0}")]
    ParseError(String),

    /// The spec was parsed but contained structural problems.
    #[error("invalid OpenAPI spec: {0}")]
    InvalidSpec(String),
}

/// Parse an OpenAPI spec string (JSON or YAML) and generate one [`ToolDyn`]
/// per matching operation.
///
/// Tool names are derived from the HTTP method and URL path:
/// `GET /users/{id}` → `get_users_id`.  Operations without an `operationId`
/// always use the method+path name.
///
/// # Filtering
/// - If [`OpenApiConfig::tag_filter`] is set, only operations tagged with at
///   least one of those tags are included.
/// - If [`OpenApiConfig::path_prefix`] is set, only paths that start with the
///   prefix are included.
///
/// # Schema composition
/// The input schema for each tool is a JSON Schema `object` whose properties
/// are the union of path parameters, query parameters, and (for
/// POST/PUT/PATCH) a `"body"` property derived from `application/json` content
/// in the request body.  Required parameters become `required` array entries.
///
/// # `$ref` resolution
/// Inline schemas are converted directly.  `$ref` references are resolved one
/// level deep from `#/components/schemas/…`.  References pointing elsewhere
/// (external URLs, deep nesting) produce an empty schema rather than an error.
pub fn from_openapi(
    spec: &str,
    config: OpenApiConfig,
    auth: Arc<dyn ApiAuth>,
) -> Result<Vec<Arc<dyn ToolDyn>>, OpenApiError> {
    let openapi = parse_spec(spec)?;
    let client = reqwest::Client::new();
    let mut tools: Vec<Arc<dyn ToolDyn>> = Vec::new();

    for (path, path_item_ref) in &openapi.paths.paths {
        // Path prefix filter.
        if let Some(prefix) = &config.path_prefix
            && !path.starts_with(prefix.as_str()) {
                continue;
            }

        let path_item = match path_item_ref {
            ReferenceOr::Item(item) => item,
            // $ref path items are rare and complex; skip.
            ReferenceOr::Reference { .. } => continue,
        };

        // Collect path-level parameter definitions (operation overrides them).
        let path_level_params: Vec<&Parameter> = path_item
            .parameters
            .iter()
            .filter_map(ref_or_item)
            .collect();

        for (method_str, maybe_op) in method_fields(path_item) {
            let Some(op) = maybe_op else { continue };

            // Tag filter.
            if let Some(tag_filter) = &config.tag_filter
                && !op.tags.iter().any(|t| tag_filter.contains(t)) {
                    continue;
                }

            // Operation-level params override path-level params with the same name.
            let op_params: Vec<&Parameter> =
                op.parameters.iter().filter_map(ref_or_item).collect();

            let mut merged_params: Vec<&Parameter> = path_level_params.clone();
            for op_param in &op_params {
                let op_name = param_data(op_param).name.as_str();
                merged_params.retain(|p| param_data(p).name != op_name);
                merged_params.push(op_param);
            }

            let name = tool_name(method_str, path);
            let description = op
                .summary
                .as_deref()
                .or(op.description.as_deref())
                .unwrap_or("")
                .to_string();

            let input_schema =
                build_input_schema(&merged_params, op.request_body.as_ref(), &openapi, &config);

            let tool = OpenApiTool {
                name,
                description,
                input_schema,
                method: method_from_str(method_str),
                path_template: path.clone(),
                base_url: config.base_url.clone(),
                auth: Arc::clone(&auth),
                client: client.clone(),
            };

            tools.push(Arc::new(tool));
        }
    }

    Ok(tools)
}

// ─── Internal tool struct ─────────────────────────────────────────────────────

/// A single API operation wrapped as a [`ToolDyn`].
struct OpenApiTool {
    name: String,
    description: String,
    input_schema: Value,
    method: reqwest::Method,
    /// Path template, e.g. `/users/{id}`.
    path_template: String,
    base_url: String,
    auth: Arc<dyn ApiAuth>,
    client: reqwest::Client,
}

impl ToolDyn for OpenApiTool {
    fn name(&self) -> &str {
        &self.name
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn input_schema(&self) -> Value {
        self.input_schema.clone()
    }

    fn call(
        &self,
        input: Value,
        _ctx: &DispatchContext,
    ) -> Pin<Box<dyn Future<Output = Result<Value, ToolError>> + Send + '_>> {
        // Clone everything we need so the future owns its data and is 'static.
        let method = self.method.clone();
        let path_template = self.path_template.clone();
        let base_url = self.base_url.clone();
        let auth = Arc::clone(&self.auth);
        let client = self.client.clone();

        Box::pin(async move {
            // Substitute path parameters.
            let mut path = path_template.clone();
            let path_param_names: HashSet<String> = extract_path_params(&path_template);
            for name in &path_param_names {
                if let Some(val) = input.get(name) {
                    path = path.replace(&format!("{{{}}}", name), &value_to_str(val));
                }
            }

            let url = format!("{}{}", base_url.trim_end_matches('/'), path);
            let mut builder = client.request(method.clone(), &url);

            // Distribute remaining input fields to query params or body.
            if let Some(obj) = input.as_object() {
                let is_body_method = matches!(
                    method,
                    reqwest::Method::POST | reqwest::Method::PUT | reqwest::Method::PATCH
                );

                if is_body_method {
                    // Explicit `"body"` key → send as JSON body directly.
                    // Otherwise, collect non-path fields into the body object.
                    if let Some(body_val) = obj.get("body") {
                        builder = builder.json(body_val);
                    } else {
                        let body: serde_json::Map<String, Value> = obj
                            .iter()
                            .filter(|(k, _)| !path_param_names.contains(*k))
                            .map(|(k, v)| (k.clone(), v.clone()))
                            .collect();
                        if !body.is_empty() {
                            builder = builder.json(&Value::Object(body));
                        }
                    }
                } else {
                    // GET / DELETE / HEAD / etc. → query string.
                    for (key, val) in obj {
                        if !path_param_names.contains(key) {
                            builder = builder.query(&[(key.as_str(), value_to_str(val))]);
                        }
                    }
                }
            }

            builder = auth.apply(builder);

            let response = builder
                .send()
                .await
                .map_err(|e| ToolError::Transient(e.to_string()))?;

            let status = response.status();
            if !status.is_success() {
                let body = response.text().await.unwrap_or_default();
                return Err(ToolError::ExecutionFailed(format!(
                    "HTTP {}: {}",
                    status.as_u16(),
                    body
                )));
            }

            let text = response
                .text()
                .await
                .map_err(|e| ToolError::ExecutionFailed(e.to_string()))?;

            // Return parsed JSON when possible, raw string otherwise.
            let value: Value =
                serde_json::from_str(&text).unwrap_or(Value::String(text));
            Ok(value)
        })
    }
}

// ─── Helpers ──────────────────────────────────────────────────────────────────

/// Try JSON first (fast path), then YAML.
fn parse_spec(spec: &str) -> Result<OpenAPI, OpenApiError> {
    let trimmed = spec.trim_start();
    if trimmed.starts_with('{') {
        serde_json::from_str(spec).map_err(|e| OpenApiError::ParseError(e.to_string()))
    } else {
        serde_yaml::from_str(spec).map_err(|e| OpenApiError::ParseError(e.to_string()))
    }
}

/// Generate a stable tool name from HTTP method and URL path.
///
/// Segments are lowercased; `{braces}` are stripped; hyphens become
/// underscores.  E.g. `GET /users/{id}` → `get_users_id`.
fn tool_name(method: &str, path: &str) -> String {
    let segments: Vec<String> = path
        .split('/')
        .filter(|s| !s.is_empty())
        .map(|s| {
            s.trim_start_matches('{')
                .trim_end_matches('}')
                .replace('-', "_")
                .to_lowercase()
        })
        .collect();

    let path_part = segments.join("_");
    if path_part.is_empty() {
        method.to_lowercase()
    } else {
        format!("{}_{}", method.to_lowercase(), path_part)
    }
}

/// Build a JSON Schema `object` combining path/query parameters and the
/// request body into a single flat input schema.
fn build_input_schema(
    params: &[&Parameter],
    request_body: Option<&ReferenceOr<RequestBody>>,
    spec: &OpenAPI,
    config: &OpenApiConfig,
) -> Value {
    let mut properties = serde_json::Map::new();
    let mut required: Vec<Value> = Vec::new();

    for param in params {
        let data = param_data(param);
        let mut prop = param_schema_to_json(&data.format, spec);

        // Inject description if the schema object doesn't already have one.
        if let Some(desc) = &data.description
            && let Some(obj) = prop.as_object_mut() {
                obj.entry("description")
                    .or_insert_with(|| Value::String(desc.clone()));
            }

        properties.insert(data.name.clone(), prop);
        if data.required {
            required.push(Value::String(data.name.clone()));
        }
    }

    // Attach request body as a `"body"` property (application/json preferred).
    if let Some(body_ref) = request_body
        && let Some(body) = ref_or_item(body_ref)
            && let Some(media) = body.content.get("application/json")
                && let Some(schema_ref) = &media.schema {
                    let schema_val = schema_ref_to_json(schema_ref, spec);
                    properties.insert("body".to_string(), schema_val);
                    if body.required {
                        required.push(Value::String("body".to_string()));
                    }
                }

    let mut schema = json!({
        "type": "object",
        "properties": Value::Object(properties),
    });

    if !required.is_empty() {
        schema["required"] = Value::Array(required);
    }

    if config.transform_schemas {
        skg_schema::transform_for_llm(&schema)
    } else {
        schema
    }
}

/// Convert a [`ParameterSchemaOrContent`] to a JSON Schema value.
fn param_schema_to_json(psc: &ParameterSchemaOrContent, spec: &OpenAPI) -> Value {
    match psc {
        ParameterSchemaOrContent::Schema(ref_or_schema) => schema_ref_to_json(ref_or_schema, spec),
        // Content-typed parameters carry schema per media type; skip for now.
        ParameterSchemaOrContent::Content(_) => json!({}),
    }
}

/// Dereference a `ReferenceOr<Schema>` to a JSON Schema value.
///
/// Inline schemas are converted via `serde_json::to_value` (which round-trips
/// correctly because `openapiv3::Schema` uses `#[serde(flatten)]`).
/// `$ref` references are resolved one level from `#/components/schemas/…`.
fn schema_ref_to_json(ref_or: &ReferenceOr<Schema>, spec: &OpenAPI) -> Value {
    match ref_or {
        ReferenceOr::Item(schema) => {
            serde_json::to_value(schema).unwrap_or_else(|_| json!({}))
        }
        ReferenceOr::Reference { reference } => resolve_component_schema(reference, spec),
    }
}

/// Resolve a `#/components/schemas/<Name>` reference to a JSON Schema value.
/// Returns an empty schema for unresolvable references.
fn resolve_component_schema(reference: &str, spec: &OpenAPI) -> Value {
    // Only handle the canonical local form: #/components/schemas/<name>
    let name = reference
        .strip_prefix("#/components/schemas/")
        .unwrap_or("");

    if name.is_empty() {
        return json!({});
    }

    spec.components
        .as_ref()
        .and_then(|c| c.schemas.get(name))
        .and_then(|r| ref_or_item(r))
        .and_then(|s| serde_json::to_value(s).ok())
        .unwrap_or_else(|| json!({}))
}

/// Extract `{param}` names from a path template.
fn extract_path_params(path: &str) -> HashSet<String> {
    let mut names = HashSet::new();
    let mut rest = path;
    while let Some(start) = rest.find('{') {
        rest = &rest[start + 1..];
        if let Some(end) = rest.find('}') {
            names.insert(rest[..end].to_string());
            rest = &rest[end + 1..];
        } else {
            break;
        }
    }
    names
}

/// Stringify a JSON value for use in URL path substitution or query strings.
fn value_to_str(val: &Value) -> String {
    match val {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Null => "null".to_string(),
        other => other.to_string(),
    }
}

/// Unwrap the `Item` arm of a [`ReferenceOr`], returning `None` for refs.
fn ref_or_item<T>(r: &ReferenceOr<T>) -> Option<&T> {
    match r {
        ReferenceOr::Item(t) => Some(t),
        ReferenceOr::Reference { .. } => None,
    }
}

/// Extract the `ParameterData` from any `Parameter` variant.
fn param_data(param: &Parameter) -> &openapiv3::ParameterData {
    match param {
        Parameter::Query { parameter_data, .. }
        | Parameter::Path { parameter_data, .. }
        | Parameter::Header { parameter_data, .. }
        | Parameter::Cookie { parameter_data, .. } => parameter_data,
    }
}

/// Enumerate all (method-string, Option<&Operation>) pairs from a `PathItem`.
fn method_fields(item: &PathItem) -> [(&'static str, Option<&openapiv3::Operation>); 8] {
    [
        ("get", item.get.as_ref()),
        ("post", item.post.as_ref()),
        ("put", item.put.as_ref()),
        ("delete", item.delete.as_ref()),
        ("patch", item.patch.as_ref()),
        ("head", item.head.as_ref()),
        ("options", item.options.as_ref()),
        ("trace", item.trace.as_ref()),
    ]
}

/// Convert a method string to a [`reqwest::Method`].
fn method_from_str(method: &str) -> reqwest::Method {
    match method {
        "get" => reqwest::Method::GET,
        "post" => reqwest::Method::POST,
        "put" => reqwest::Method::PUT,
        "delete" => reqwest::Method::DELETE,
        "patch" => reqwest::Method::PATCH,
        "head" => reqwest::Method::HEAD,
        "options" => reqwest::Method::OPTIONS,
        "trace" => reqwest::Method::TRACE,
        _ => reqwest::Method::GET,
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Minimal Petstore-like spec used across multiple tests.
    ///
    /// Operations:
    ///  - GET  /pets          → `get_pets`          (tag: pets)
    ///  - POST /pets          → `post_pets`          (tag: pets)
    ///  - GET  /pets/{petId}  → `get_pets_petid`     (tag: pets)
    ///  - DELETE /pets/{petId}→ `delete_pets_petid`  (tag: management)
    const PETSTORE_SPEC: &str = r#"
openapi: "3.0.0"
info:
  title: Petstore
  version: "1.0.0"
paths:
  /pets:
    get:
      summary: List all pets
      operationId: listPets
      tags:
        - pets
      parameters:
        - name: limit
          in: query
          required: false
          schema:
            type: integer
      responses:
        "200":
          description: A list of pets
    post:
      summary: Create a pet
      operationId: createPet
      tags:
        - pets
      requestBody:
        required: true
        content:
          application/json:
            schema:
              type: object
              properties:
                name:
                  type: string
              required:
                - name
      responses:
        "201":
          description: Pet created
  /pets/{petId}:
    get:
      summary: Info for a specific pet
      operationId: showPetById
      tags:
        - pets
      parameters:
        - name: petId
          in: path
          required: true
          schema:
            type: string
      responses:
        "200":
          description: Pet info
    delete:
      summary: Delete a specific pet
      operationId: deletePet
      tags:
        - management
      parameters:
        - name: petId
          in: path
          required: true
          schema:
            type: string
      responses:
        "204":
          description: Pet deleted
"#;

    fn base_config() -> OpenApiConfig {
        OpenApiConfig {
            base_url: "https://api.example.com".into(),
            tag_filter: None,
            path_prefix: None,
            transform_schemas: false,
        }
    }

    #[test]
    fn parse_petstore_generates_tools() {
        let tools = from_openapi(PETSTORE_SPEC, base_config(), Arc::new(NoAuth))
            .expect("parse should succeed");

        // Four operations in the spec → four tools.
        assert_eq!(tools.len(), 4, "expected 4 tools, got {:?}", {
            tools.iter().map(|t| t.name()).collect::<Vec<_>>()
        });

        let names: Vec<&str> = tools.iter().map(|t| t.name()).collect();
        assert!(names.contains(&"get_pets"), "missing get_pets");
        assert!(names.contains(&"post_pets"), "missing post_pets");
        assert!(names.contains(&"get_pets_petid"), "missing get_pets_petid");
        assert!(
            names.contains(&"delete_pets_petid"),
            "missing delete_pets_petid"
        );
    }

    #[test]
    fn tool_schema_matches_params() {
        let tools = from_openapi(PETSTORE_SPEC, base_config(), Arc::new(NoAuth))
            .expect("parse should succeed");

        // GET /pets/{petId} should have `petId` in its schema properties.
        let pet_tool = tools
            .iter()
            .find(|t| t.name() == "get_pets_petid")
            .expect("get_pets_petid not found");

        let schema = pet_tool.input_schema();
        let props = schema
            .get("properties")
            .and_then(Value::as_object)
            .expect("schema must have properties");

        assert!(
            props.contains_key("petId"),
            "petId must appear in schema properties; got: {:?}",
            props.keys().collect::<Vec<_>>()
        );

        let required = schema.get("required").and_then(Value::as_array);
        let empty = vec![];
        let required_names: Vec<&str> = required
            .unwrap_or(&empty)
            .iter()
            .filter_map(|v| v.as_str())
            .collect();
        assert!(
            required_names.contains(&"petId"),
            "petId must be required; required={:?}",
            required_names
        );

        // GET /pets should have `limit` as an optional query parameter.
        let list_tool = tools
            .iter()
            .find(|t| t.name() == "get_pets")
            .expect("get_pets not found");

        let list_schema = list_tool.input_schema();
        let list_props = list_schema
            .get("properties")
            .and_then(Value::as_object)
            .expect("schema must have properties");
        assert!(
            list_props.contains_key("limit"),
            "limit must appear in list schema; got: {:?}",
            list_props.keys().collect::<Vec<_>>()
        );
    }

    #[test]
    fn tag_filter_works() {
        let config = OpenApiConfig {
            tag_filter: Some(vec!["pets".to_string()]),
            ..base_config()
        };
        let tools =
            from_openapi(PETSTORE_SPEC, config, Arc::new(NoAuth)).expect("parse should succeed");

        // Three operations tagged "pets" (list, create, show-by-id).
        assert_eq!(
            tools.len(),
            3,
            "expected 3 tools after pets-tag filter; got {:?}",
            tools.iter().map(|t| t.name()).collect::<Vec<_>>()
        );

        // "management"-tagged DELETE should be excluded.
        assert!(
            !tools.iter().any(|t| t.name() == "delete_pets_petid"),
            "delete_pets_petid must be excluded by tag filter"
        );
    }

    #[test]
    fn bearer_auth_applied() {
        let auth = BearerAuth("super-secret-token".to_string());
        let client = reqwest::Client::new();
        let builder = client.get("http://example.com/test");
        let builder = auth.apply(builder);

        let request = builder.build().expect("request build should succeed");
        let auth_header = request
            .headers()
            .get("authorization")
            .and_then(|v| v.to_str().ok());

        assert_eq!(
            auth_header,
            Some("Bearer super-secret-token"),
            "Authorization header must be set correctly"
        );
    }
}
