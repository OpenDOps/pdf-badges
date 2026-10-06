/// `raw` is accepted only when it is `v` plus `cargo_version`.
pub fn parse_release_tag(raw: &str, cargo_version: &str) -> Result<String, String> {
    let expected = format!("v{cargo_version}");
    if raw == expected {
        Ok(expected)
    } else {
        Err(format!(
            "REGISTRATION_RELEASE_TAG must be {expected}, got {raw}"
        ))
    }
}

/// Unset bakes `v` plus the Cargo version. A set value must be that tag.
pub fn bake_release_tag(raw: Option<&str>, cargo_version: &str) -> Result<String, String> {
    match raw {
        None => Ok(format!("v{cargo_version}")),
        Some(raw) => parse_release_tag(raw, cargo_version),
    }
}
