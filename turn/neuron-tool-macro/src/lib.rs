//! Proc-macro for deriving `ToolDyn` from annotated async functions.
//!
//! # Usage
//!
//! ```rust,ignore
//! #[neuron_tool(name = "get_weather", description = "Get weather for a location")]
//! async fn get_weather(location: String, units: Option<String>) -> Result<serde_json::Value, ToolError> {
//!     // ...
//! }
//! ```
//!
//! This generates a `GetWeatherTool` struct implementing `neuron_tool::ToolDyn`.

use proc_macro::TokenStream;
use proc_macro2::Span;
use quote::quote;
use syn::{parse_macro_input, ItemFn, LitStr, Type};

/// Parsed arguments from `#[neuron_tool(...)]`.
struct MacroArgs {
    name: String,
    description: String,
    concurrent: bool,
}

impl syn::parse::Parse for MacroArgs {
    fn parse(input: syn::parse::ParseStream<'_>) -> syn::Result<Self> {
        let mut name: Option<String> = None;
        let mut description: Option<String> = None;
        let mut concurrent = false;

        while !input.is_empty() {
            let ident: syn::Ident = input.parse()?;
            match ident.to_string().as_str() {
                "name" => {
                    input.parse::<syn::Token![=]>()?;
                    let lit: LitStr = input.parse()?;
                    name = Some(lit.value());
                }
                "description" => {
                    input.parse::<syn::Token![=]>()?;
                    let lit: LitStr = input.parse()?;
                    description = Some(lit.value());
                }
                "concurrent" => {
                    concurrent = true;
                }
                other => {
                    return Err(syn::Error::new(
                        ident.span(),
                        format!("unknown attribute key: `{other}`"),
                    ));
                }
            }
            if !input.is_empty() {
                input.parse::<syn::Token![,]>()?;
            }
        }

        Ok(MacroArgs {
            name: name.ok_or_else(|| {
                syn::Error::new(Span::call_site(), "missing required attribute `name`")
            })?,
            description: description.ok_or_else(|| {
                syn::Error::new(Span::call_site(), "missing required attribute `description`")
            })?,
            concurrent,
        })
    }
}

/// Convert a `snake_case` identifier string to `PascalCase`.
fn snake_to_pascal(s: &str) -> String {
    s.split('_')
        .filter(|part| !part.is_empty())
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                None => String::new(),
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
            }
        })
        .collect()
}

/// If `ty` is `Option<T>`, return `Some(T)`; otherwise return `None`.
fn extract_option_inner(ty: &Type) -> Option<&Type> {
    if let Type::Path(tp) = ty
        && tp.qself.is_none()
        && let Some(last) = tp.path.segments.last()
        && last.ident == "Option"
        && let syn::PathArguments::AngleBracketed(args) = &last.arguments
        && let Some(syn::GenericArgument::Type(inner)) = args.args.first()
    {
        return Some(inner);
    }
    None
}

/// Return `true` if `ty` is `&ToolCallContext` (any qualifying path ending in `ToolCallContext`).
fn is_tool_call_context_ref(ty: &Type) -> bool {
    if let Type::Reference(r) = ty
        && let Type::Path(tp) = r.elem.as_ref()
        && let Some(last) = tp.path.segments.last()
    {
        return last.ident == "ToolCallContext";
    }
    false
}

/// Map a Rust type to its JSON Schema property object (as a `TokenStream`).
///
/// `Option<T>` is unwrapped — requiredness is tracked separately.
fn type_to_schema(ty: &Type) -> proc_macro2::TokenStream {
    // Unwrap Option<T> → schema for T
    if let Some(inner) = extract_option_inner(ty) {
        return type_to_schema(inner);
    }

    // Dereference
    if let Type::Reference(r) = ty {
        return type_to_schema(&r.elem);
    }

    if let Type::Path(tp) = ty
        && tp.qself.is_none()
    {
        let last_name = tp.path.segments.last().map(|s| s.ident.to_string());
        match last_name.as_deref() {
            Some("String") | Some("str") => {
                return quote! { ::serde_json::json!({"type": "string"}) };
            }
            Some("i8")
            | Some("i16")
            | Some("i32")
            | Some("i64")
            | Some("i128")
            | Some("u8")
            | Some("u16")
            | Some("u32")
            | Some("u64")
            | Some("u128")
            | Some("isize")
            | Some("usize") => {
                return quote! { ::serde_json::json!({"type": "integer"}) };
            }
            Some("f32") | Some("f64") => {
                return quote! { ::serde_json::json!({"type": "number"}) };
            }
            Some("bool") => {
                return quote! { ::serde_json::json!({"type": "boolean"}) };
            }
            Some("Value") => {
                // serde_json::Value → any
                return quote! { ::serde_json::json!({}) };
            }
            _ => {}
        }
    }

    // Fallback: any
    quote! { ::serde_json::json!({}) }
}

/// Per-parameter metadata extracted from the function signature.
struct ParamInfo {
    ident: syn::Ident,
    ty: Type,
    /// True if the parameter type is `Option<_>` (not included in `required`).
    is_optional: bool,
    /// True if the parameter is `&ToolCallContext` (not included in schema, passed through).
    is_ctx: bool,
}

/// Derive a `ToolDyn` implementation from an annotated async function.
///
/// # Attributes
///
/// - `name = "..."` — tool name returned by `ToolDyn::name`
/// - `description = "..."` — tool description returned by `ToolDyn::description`
/// - `concurrent` — if present, `ToolDyn::concurrency_hint` returns `Shared`; otherwise `Exclusive`
///
/// # Generated output
///
/// - The original `async fn` is kept intact.
/// - A `pub struct <PascalCase>Tool` struct is generated.
/// - `impl ToolDyn for <PascalCase>Tool` is generated.
/// - A `fn new() -> Self` constructor is generated.
///
/// # Parameter handling
///
/// - Parameters of type `&ToolCallContext` are excluded from the JSON schema and
///   passed through to the underlying function via the `call()` context argument.
/// - `Option<T>` parameters are included in the schema but omitted from `required`.
/// - All other parameters are required in the schema.
#[proc_macro_attribute]
pub fn neuron_tool(attr: TokenStream, item: TokenStream) -> TokenStream {
    let args = parse_macro_input!(attr as MacroArgs);
    let func = parse_macro_input!(item as ItemFn);

    let fn_name = &func.sig.ident;
    let fn_name_str = fn_name.to_string();
    let struct_name_str = format!("{}Tool", snake_to_pascal(&fn_name_str));
    let struct_ident = syn::Ident::new(&struct_name_str, fn_name.span());

    let tool_name = &args.name;
    let tool_desc = &args.description;

    let concurrency_hint = if args.concurrent {
        quote! { ::neuron_tool::ToolConcurrencyHint::Shared }
    } else {
        quote! { ::neuron_tool::ToolConcurrencyHint::Exclusive }
    };

    // Collect parameter info from the function signature
    let mut params: Vec<ParamInfo> = Vec::new();
    for arg in &func.sig.inputs {
        match arg {
            syn::FnArg::Receiver(_) => {
                // Skip `self` — tool functions should not take self
            }
            syn::FnArg::Typed(pat_ty) => {
                let ident = match pat_ty.pat.as_ref() {
                    syn::Pat::Ident(pi) => pi.ident.clone(),
                    _ => {
                        return syn::Error::new_spanned(
                            &pat_ty.pat,
                            "#[neuron_tool]: only simple identifier patterns are supported in parameters",
                        )
                        .into_compile_error()
                        .into();
                    }
                };
                let ty = *pat_ty.ty.clone();
                let is_ctx = is_tool_call_context_ref(&ty);
                let is_optional = extract_option_inner(&ty).is_some();
                params.push(ParamInfo {
                    ident,
                    ty,
                    is_optional,
                    is_ctx,
                });
            }
        }
    }

    // Schema: properties object entries
    let schema_props: Vec<proc_macro2::TokenStream> = params
        .iter()
        .filter(|p| !p.is_ctx)
        .map(|p| {
            let name_str = p.ident.to_string();
            let schema = type_to_schema(&p.ty);
            quote! { #name_str: #schema, }
        })
        .collect();

    // Schema: required array entries (non-optional, non-ctx params)
    let required_fields: Vec<String> = params
        .iter()
        .filter(|p| !p.is_ctx && !p.is_optional)
        .map(|p| p.ident.to_string())
        .collect();

    let input_schema = quote! {
        ::serde_json::json!({
            "type": "object",
            "properties": {
                #(#schema_props)*
            },
            "required": [#(#required_fields),*]
        })
    };

    // call() body: determine whether ctx parameter is used
    let has_ctx = params.iter().any(|p| p.is_ctx);

    // Bind name for the `ctx` parameter in the generated `call()` method:
    // prefix with `_` when unused to silence dead-code warnings.
    let ctx_param_name: proc_macro2::TokenStream = if has_ctx {
        quote! { ctx }
    } else {
        quote! { _ctx }
    };

    // When the original function takes `&ToolCallContext`, the `call()` implementation
    // must clone `ctx` into an owned value before the `async move` block.
    // The trait's `'_` lifetime on the return type ties to `&self`, not to `ctx`,
    // so capturing the raw `ctx` reference in the async block causes a lifetime conflict.
    let ctx_clone_stmt: proc_macro2::TokenStream = if has_ctx {
        quote! { let __neuron_ctx = ctx.clone(); }
    } else {
        quote! {}
    };
    let ctx_reborrow_stmt: proc_macro2::TokenStream = if has_ctx {
        quote! { let ctx = &__neuron_ctx; }
    } else {
        quote! {}
    };
    // Deserialise each non-ctx parameter from the JSON input
    let param_deserializations: Vec<proc_macro2::TokenStream> = params
        .iter()
        .filter(|p| !p.is_ctx)
        .map(|p| {
            let name = &p.ident;
            let name_str = name.to_string();
            let ty = &p.ty;
            quote! {
                let #name: #ty = ::serde_json::from_value(
                    input.get(#name_str).cloned().unwrap_or(::serde_json::Value::Null)
                )
                .map_err(|e| ::neuron_tool::ToolError::InvalidInput(e.to_string()))?;
            }
        })
        .collect();

    // Arguments forwarded to the original function
    let call_args: Vec<proc_macro2::TokenStream> = params
        .iter()
        .map(|p| {
            if p.is_ctx {
                quote! { ctx }
            } else {
                let name = &p.ident;
                quote! { #name }
            }
        })
        .collect();

    let expanded = quote! {
        #func

        /// Generated tool struct from `#[neuron_tool]`.
        pub struct #struct_ident;

        impl #struct_ident {
            /// Create a new instance of this tool.
            pub fn new() -> Self {
                Self
            }
        }

        impl ::std::default::Default for #struct_ident {
            fn default() -> Self {
                Self::new()
            }
        }

        impl ::neuron_tool::ToolDyn for #struct_ident {
            fn name(&self) -> &str {
                #tool_name
            }

            fn description(&self) -> &str {
                #tool_desc
            }

            fn input_schema(&self) -> ::serde_json::Value {
                #input_schema
            }

            fn call(
                &self,
                input: ::serde_json::Value,
                #ctx_param_name: &::neuron_tool::ToolCallContext,
            ) -> ::std::pin::Pin<Box<dyn ::std::future::Future<Output = Result<::serde_json::Value, ::neuron_tool::ToolError>> + Send + '_>>
            {
                // When ctx is needed by the original function, clone it into an owned
                // value so the returned future has no lifetime dependency on the
                // `ctx: &ToolCallContext` borrow (whose lifetime is not reflected in
                // the trait's return-type `'_`).
                #ctx_clone_stmt
                Box::pin(async move {
                    #ctx_reborrow_stmt
                    #(#param_deserializations)*
                    #fn_name(#(#call_args),*).await
                })
            }

            fn concurrency_hint(&self) -> ::neuron_tool::ToolConcurrencyHint {
                #concurrency_hint
            }
        }
    };

    expanded.into()
}
