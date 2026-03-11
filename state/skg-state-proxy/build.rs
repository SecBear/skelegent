fn main() {
    tonic_build::configure()
        .build_server(true)
        .build_client(true)
        .compile_protos(&["../../proto/skg_state_proxy_v1.proto"], &["../../proto/"])
        .unwrap();
}
