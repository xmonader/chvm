use thiserror::Error;

#[derive(Error, Debug)]
pub enum ChvmError {
    #[error("VM not found: {0}")]
    VmNotFound(String),

    #[error("VM already exists: {0}")]
    VmAlreadyExists(String),

    #[error("VM is not running")]
    VmNotRunning,

    #[error("VM is already running")]
    VmAlreadyRunning,

    #[error("Image not found: {0}")]
    ImageNotFound(String),

    #[error("Invalid image reference: {0}")]
    InvalidImageRef(String),

    #[error("Kernel not found: {0}")]
    KernelNotFound(String),

    #[error("Cloud Hypervisor not found: {0}")]
    HypervisorNotFound(String),

    #[error("API error: {0}")]
    ApiError(String),

    #[error("Network error: {0}")]
    NetworkError(String),

    #[error("Storage error: {0}")]
    StorageError(String),

    #[error("OCI error: {0}")]
    OciError(String),

    #[error("Vsock error: {0}")]
    VsockError(String),

    #[error("Process error: {0}")]
    ProcessError(String),

    #[error("Config error: {0}")]
    ConfigError(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("Invalid rootfs: {0}")]
    InvalidRootfs(String),

    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, ChvmError>;
