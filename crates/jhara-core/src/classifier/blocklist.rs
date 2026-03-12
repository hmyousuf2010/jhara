use std::ffi::OsStr;
use std::path::Path;

pub const BLOCKLIST_PATTERNS: &[&str] = &[
    "terraform.tfstate",
    "terraform.tfstate.backup",
    ".terraform/terraform.tfstate",
    ".vagrant/machines",
    "*.pem",
    "*.key",
    ".env",
    ".env.local",
    ".env.production",
    ".env.staging",
];

/// An advanced, zero-allocation-on-check blocklist matcher.
pub struct Blocklist {
    exact_names: Vec<&'static OsStr>,
    extensions: Vec<&'static OsStr>,
    path_suffixes: Vec<&'static str>,
}

impl Blocklist {
    /// Initializes the blocklist by pre-computing match categories for $O(1)$ or fast $O(N)$ lookups.
    pub fn new() -> Self {
        let mut exact_names = Vec::new();
        let mut extensions = Vec::new();
        let mut path_suffixes = Vec::new();

        for &pattern in BLOCKLIST_PATTERNS {
            if let Some(ext) = pattern.strip_prefix("*.") {
                extensions.push(OsStr::new(ext));
            } else if pattern.contains('/') {
                path_suffixes.push(pattern);
            } else {
                exact_names.push(OsStr::new(pattern));
            }
        }

        Self {
            exact_names,
            extensions,
            path_suffixes,
        }
    }

    /// Checks if a path matches the absolute blocklist.
    /// Optimized to check the cheapest conditions (extensions, file names) first.
    pub fn is_blocked(&self, path: &Path) -> bool {
        // 1. Check Extensions (e.g., *.pem, *.key)
        if let Some(ext) = path.extension() {
            if self.extensions.contains(&ext) {
                return true;
            }
        }

        // 2. Check Exact File Names (e.g., .env)
        if let Some(file_name) = path.file_name() {
            if self.exact_names.contains(&file_name) {
                return true;
            }
        }

        // 3. Check Path Suffixes (e.g., .terraform/terraform.tfstate)
        // This is the most expensive check, done last.
        if !self.path_suffixes.is_empty() {
            let path_str = path.to_string_lossy();
            if self.path_suffixes.iter().any(|suffix| path_str.ends_with(suffix)) {
                return true;
            }
        }

        false
    }
}

impl Default for Blocklist {
    fn default() -> Self {
        Self::new()
    }
}
