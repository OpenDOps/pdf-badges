fn main() -> Result<(), Box<dyn std::error::Error>> {
    // rust-analyzer is launched with a short PATH, so a Homebrew protoc is invisible.
    ensure_protoc();
    let out = std::path::PathBuf::from(std::env::var("OUT_DIR")?);

    println!("cargo:rerun-if-changed=proto/irbis/pdf/v1/pdf.proto");
    tonic_prost_build::configure()
        .file_descriptor_set_path(out.join("pdf_descriptor.bin"))
        .compile_protos(&["proto/irbis/pdf/v1/pdf.proto"], &["proto"])?;

    println!("cargo:rerun-if-changed=proto/irbis/registration/v1/registration.proto");
    tonic_prost_build::configure()
        .file_descriptor_set_path(out.join("registration_descriptor.bin"))
        .compile_protos(
            &["proto/irbis/registration/v1/registration.proto"],
            &["proto"],
        )?;
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
