// error.rs
use thiserror::Error;

#[derive(Debug, Error)]
pub enum JharaError {
    #[error("IO error at {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },

    #[error("JSON parse error in {path}: {source}")]
    Json {
        path: String,
        #[source]
        source: serde_json::Error,
    },

    #[error("Plist parse error in {path}: {source}")]
    Plist {
        path: String,
        #[source]
        source: plist::Error,
    },

    #[error("Signature database load error: {0}")]
    SignatureLoad(String),
}

impl JharaError {
    pub fn io(path: impl Into<String>, source: std::io::Error) -> Self {
        JharaError::Io { path: path.into(), source }
    }

    pub fn json(path: impl Into<String>, source: serde_json::Error) -> Self {
        JharaError::Json { path: path.into(), source }
    }

    pub fn plist(path: impl Into<String>, source: plist::Error) -> Self {
        JharaError::Plist { path: path.into(), source }
    }
}
