/// `RUST_REG_UPGRADE_PROBE=B cargo build` bakes letter `B` into the tray binary.
/// Unset / invalid → empty (shipping). Env at runtime still wins when set.
fn main() {
    println!("cargo:rerun-if-env-changed=RUST_REG_UPGRADE_PROBE");
    let raw = std::env::var("RUST_REG_UPGRADE_PROBE").unwrap_or_default();
    let trimmed = raw.trim();
    let baked = if trimmed.len() == 1 && trimmed.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
    {
        trimmed.to_ascii_uppercase()
    } else {
        String::new()
    };
    println!("cargo:rustc-env=RUST_REG_UPGRADE_PROBE_BAKED={baked}");
}
