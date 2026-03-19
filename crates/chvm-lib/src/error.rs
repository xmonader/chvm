use thiserror::Error;

#[derive(Error, Debug)]
pub enum ChvmError {
    #[error("VM not found: {0}")]
    VmNotFound(String),

    #[error("Ambiguous VM identifier '{0}', matches: {1}")]
    VmAmbiguous(String, String),

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_vm_not_found_error() {
        let err = ChvmError::VmNotFound("test-vm".to_string());
        assert_eq!(err.to_string(), "VM not found: test-vm");
    }

    #[test]
    fn test_vm_ambiguous_error() {
        let err = ChvmError::VmAmbiguous("abc".to_string(), "abc123, abcdef".to_string());
        assert_eq!(
            err.to_string(),
            "Ambiguous VM identifier 'abc', matches: abc123, abcdef"
        );
    }

    #[test]
    fn test_vm_already_exists_error() {
        let err = ChvmError::VmAlreadyExists("existing-vm".to_string());
        assert_eq!(err.to_string(), "VM already exists: existing-vm");
    }

    #[test]
    fn test_vm_not_running_error() {
        let err = ChvmError::VmNotRunning;
        assert_eq!(err.to_string(), "VM is not running");
    }

    #[test]
    fn test_vm_already_running_error() {
        let err = ChvmError::VmAlreadyRunning;
        assert_eq!(err.to_string(), "VM is already running");
    }

    #[test]
    fn test_image_not_found_error() {
        let err = ChvmError::ImageNotFound("alpine:latest".to_string());
        assert_eq!(err.to_string(), "Image not found: alpine:latest");
    }

    #[test]
    fn test_invalid_image_ref_error() {
        let err = ChvmError::InvalidImageRef("bad::ref".to_string());
        assert_eq!(err.to_string(), "Invalid image reference: bad::ref");
    }

    #[test]
    fn test_kernel_not_found_error() {
        let err = ChvmError::KernelNotFound("/path/to/kernel".to_string());
        assert_eq!(err.to_string(), "Kernel not found: /path/to/kernel");
    }

    #[test]
    fn test_hypervisor_not_found_error() {
        let err = ChvmError::HypervisorNotFound("cloud-hypervisor not in PATH".to_string());
        assert_eq!(
            err.to_string(),
            "Cloud Hypervisor not found: cloud-hypervisor not in PATH"
        );
    }

    #[test]
    fn test_api_error() {
        let err = ChvmError::ApiError("connection refused".to_string());
        assert_eq!(err.to_string(), "API error: connection refused");
    }

    #[test]
    fn test_network_error() {
        let err = ChvmError::NetworkError("failed to create bridge".to_string());
        assert_eq!(err.to_string(), "Network error: failed to create bridge");
    }

    #[test]
    fn test_storage_error() {
        let err = ChvmError::StorageError("disk full".to_string());
        assert_eq!(err.to_string(), "Storage error: disk full");
    }

    #[test]
    fn test_oci_error() {
        let err = ChvmError::OciError("failed to pull image".to_string());
        assert_eq!(err.to_string(), "OCI error: failed to pull image");
    }

    #[test]
    fn test_vsock_error() {
        let err = ChvmError::VsockError("connection timeout".to_string());
        assert_eq!(err.to_string(), "Vsock error: connection timeout");
    }

    #[test]
    fn test_process_error() {
        let err = ChvmError::ProcessError("command not found".to_string());
        assert_eq!(err.to_string(), "Process error: command not found");
    }

    #[test]
    fn test_config_error() {
        let err = ChvmError::ConfigError("invalid JSON".to_string());
        assert_eq!(err.to_string(), "Config error: invalid JSON");
    }

    #[test]
    fn test_invalid_rootfs_error() {
        let err = ChvmError::InvalidRootfs("path does not exist".to_string());
        assert_eq!(err.to_string(), "Invalid rootfs: path does not exist");
    }

    #[test]
    fn test_other_error() {
        let err = ChvmError::Other("unexpected error".to_string());
        assert_eq!(err.to_string(), "unexpected error");
    }

    #[test]
    fn test_io_error_conversion() {
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "file not found");
        let chvm_err: ChvmError = io_err.into();
        assert!(matches!(chvm_err, ChvmError::Io(_)));
    }

    #[test]
    fn test_json_error_conversion() {
        let json_str = "{invalid json}";
        let json_err = serde_json::from_str::<serde_json::Value>(json_str).unwrap_err();
        let chvm_err: ChvmError = json_err.into();
        assert!(matches!(chvm_err, ChvmError::Json(_)));
    }

    #[test]
    fn test_result_type_ok() {
        let result: Result<i32> = Ok(42);
        assert!(matches!(result, Ok(42)));
    }

    #[test]
    fn test_result_type_err() {
        let result: Result<i32> = Err(ChvmError::VmNotFound("test".to_string()));
        assert!(result.is_err());
    }
}
