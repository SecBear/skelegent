fn main() -> Result<(), Box<dyn std::error::Error>> {
    tonic_build::configure()
        .build_server(true)
        .build_client(false)
        .compile_protos(
            &["../../proto/skg_runner_v1.proto"],
            &["../../proto"],
        )?;
    Ok(())
}
