#[path = "src/registration_server/release_tag.rs"]
mod release_tag;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    compile_release_tag();
    compile_sync_in_flight();
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

/// `REGISTRATION_RELEASE_TAG=v0.1.0 cargo build` bakes that tag. Unset bakes `v` plus the Cargo version.
fn compile_release_tag() {
    println!("cargo:rerun-if-env-changed=REGISTRATION_RELEASE_TAG");
    let cargo_version = std::env::var("CARGO_PKG_VERSION").expect("CARGO_PKG_VERSION");
    let raw = std::env::var("REGISTRATION_RELEASE_TAG").ok();
    let tag = release_tag::bake_release_tag(raw.as_deref(), &cargo_version)
        .unwrap_or_else(|err| panic!("{err}"));
    println!("cargo:rustc-env=REGISTRATION_RELEASE_TAG={tag}");
}

/// `SYNC_IN_FLIGHT=2 cargo build` bakes that count into the binary. Unset stays 100.
fn compile_sync_in_flight() {
    println!("cargo:rerun-if-env-changed=SYNC_IN_FLIGHT");
    let raw = std::env::var("SYNC_IN_FLIGHT").unwrap_or_else(|_| "100".into());
    let value: usize = raw
        .parse()
        .unwrap_or_else(|_| panic!("SYNC_IN_FLIGHT must be a positive integer, got {raw}"));
    if value == 0 {
        panic!("SYNC_IN_FLIGHT must be greater than zero");
    }
    println!("cargo:rustc-env=SYNC_IN_FLIGHT={value}");
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
