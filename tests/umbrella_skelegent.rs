#[test]
fn umbrella_crate_prelude_compiles() {
    use skelegent::prelude::*;

    let _tools = ToolRegistry::new();
}
