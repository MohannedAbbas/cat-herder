//! Generate the `cat_herder.v1` gRPC types from `proto/control.proto` at build time.
//!
//! Uses a vendored `protoc` binary (via `protoc-bin-vendored`) so the build needs no
//! system `protoc`/`buf` (see README "Toolchain notes").

use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // The vendored protoc binary (bundled by protoc-bin-vendored).
    let protoc = protoc_bin_vendored::protoc_bin_path()?;
    // SAFETY: build scripts are single-threaded; setting PROTOC here is the standard
    // prost-build/protoc-bin-vendored integration and has no concurrent observers.
    unsafe { std::env::set_var("PROTOC", protoc) };

    // Repo root is four levels up from this crate's manifest dir.
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../../")
        .canonicalize()?;

    tonic_build::configure()
        .build_server(true)
        .build_client(false)
        .compile_protos(
            &[
                root.join("proto/control.proto"),
                root.join("proto/budget.proto"),
            ],
            &[root.join("proto")],
        )?;

    // Re-run the build script if either proto changes.
    println!(
        "cargo:rerun-if-changed={}",
        root.join("proto/control.proto").display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        root.join("proto/budget.proto").display()
    );
    Ok(())
}
