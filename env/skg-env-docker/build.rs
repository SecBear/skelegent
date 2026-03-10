fn main() -> Result<(), Box<dyn std::error::Error>> {
    tonic_build::configure()
        .build_server(false)
        .build_client(true)
        .compile_protos(
            &[
                "../../proto/skg_runner_v1.proto",
                "../../proto/skg_state_proxy_v1.proto",
            ],
            &["../../proto"],
        )?;
    Ok(())
}
