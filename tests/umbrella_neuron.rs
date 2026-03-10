#[test]
fn umbrella_crate_prelude_compiles() {
    use neuron::prelude::*;

    let _tools = ToolRegistry::new();
}
