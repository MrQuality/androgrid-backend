//! Build script for compiling Protocol Buffer schemas into Rust stubs.
//!
//! Traces to: [REQ-OP-001], [REQ-OP-002], [REQ-OP-003], [REQ-OP-004]

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Set PROTOC path using precompiled protoc-bin-vendored
    let protoc_path = protoc_bin_vendored::protoc_bin_path()
        .map_err(|e| format!("Failed to locate protoc binary: {}", e))?;
    std::env::set_var("PROTOC", protoc_path);

    tonic_build::configure()
        .build_server(true)
        .build_client(true)
        .compile_protos(&["proto/androgrid/v1/orchestrator.proto"], &["proto"])?;

    println!("cargo:rerun-if-changed=proto/androgrid/v1/orchestrator.proto");
    Ok(())
}
