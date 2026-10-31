fn main() -> Result<(), Box<dyn std::error::Error>> {
    ensure_protoc();
    let manifest = std::path::PathBuf::from(std::env::var("CARGO_MANIFEST_DIR")?);
    let proto_root = manifest.join("../../proto");
    let service = proto_root.join("irbis/google_tables/v1/google_tables.proto");
    let table = proto_root.join("irbis/table/v1/table.proto");
    println!("cargo:rerun-if-changed={}", service.display());
    println!("cargo:rerun-if-changed={}", table.display());
    let out = std::path::PathBuf::from(std::env::var("OUT_DIR")?);
    tonic_prost_build::configure()
        .file_descriptor_set_path(out.join("google_tables_descriptor.bin"))
        .compile_protos(&[service], &[proto_root])?;
    Ok(())
}

fn ensure_protoc() {
    if std::env::var_os("PROTOC").is_some() {
        return;
    }
    let path = std::env::var_os("PATH").unwrap_or_default();
    if std::env::split_paths(&path).any(|dir| dir.join("protoc").is_file()) {
        return;
    }
    for candidate in ["/usr/local/bin/protoc", "/opt/homebrew/bin/protoc"] {
        if std::path::Path::new(candidate).is_file() {
            std::env::set_var("PROTOC", candidate);
            return;
        }
    }
}
