// --------------------------------------------------------------------------------------------------------------------
// Identity of the running build, for log headers, crash reports and about boxes.

/// Crate name, as declared in Cargo.toml.
pub fn app_name() -> &'static str {
    env!("CARGO_PKG_NAME")
}

/// Crate version, as declared in Cargo.toml.
pub fn app_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// Name and version on one line, the form the startup log entry uses.
pub fn banner() -> String {
    format!("{} {}", app_name(), app_version())
}

// --------------------------------------------------------------------------------------------------------------------
// --------------------------------------------------------------------------------------------------------------------
// --------------------------------------------------------------------------------------------------------------------
// Tests

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn banner_carries_name_and_version() {
        let text = banner();
        assert!(text.starts_with(app_name()));
        assert!(text.ends_with(app_version()));
    }
}
