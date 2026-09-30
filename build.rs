fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("cargo:rerun-if-changed=proto/irbis/pdf/v1/pdf.proto");
    let descriptor = std::path::PathBuf::from(std::env::var("OUT_DIR")?).join("pdf_descriptor.bin");
    tonic_build::configure()
        .file_descriptor_set_path(descriptor)
        .compile_protos(&["proto/irbis/pdf/v1/pdf.proto"], &["proto"])?;
    Ok(())
}
