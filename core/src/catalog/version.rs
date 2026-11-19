// HX Edit version names for `catalogs/<version>/`. Shared with `core/build.rs` (through `include!`), so the
// folder the build embeds and the name `--export-catalog` accepts follow the same rule.

/// A version name is numbers separated by dots, like `3.82` or `3.9.1`.
pub fn is_version(name: &str) -> bool {
    !name.is_empty() && name.split('.').all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
}

/// Sort key: the numbers, compared as numbers (3.9 before 3.82 before 3.100).
pub fn version_key(name: &str) -> Vec<u64> {
    name.split('.').map(|p| p.parse().unwrap_or(0)).collect()
}
