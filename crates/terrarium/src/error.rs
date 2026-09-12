use thiserror::Error;

#[derive(Debug, Error)]
pub enum TerrariumError {
    #[error("project profile not found at {path}")]
    ProfileNotFound { path: String },

    #[error("workspace config not found at {path}")]
    WorkspaceNotFound { path: String },

    #[error("invalid preset: {name}")]
    InvalidPreset { name: String },

    #[error("instance not found: {id}")]
    InstanceNotFound { id: String },

    #[error("sandbox-exec failed: {reason}")]
    SandboxExecFailed { reason: String },

    #[error("registry lock failed")]
    RegistryLockFailed,

    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error(transparent)]
    Toml(#[from] toml::de::Error),

    #[error(transparent)]
    TomlSer(#[from] toml::ser::Error),
}

pub type Result<T> = std::result::Result<T, TerrariumError>;
