/// Parse and normalize an OCI image reference.
///
/// Handles Docker Hub shorthand:
///   "alpine" -> "docker.io/library/alpine:latest"
///   "alpine:3.18" -> "docker.io/library/alpine:3.18"
///   "nginx" -> "docker.io/library/nginx:latest"
///   "myuser/myimage" -> "docker.io/myuser/myimage:latest"
///   "ghcr.io/user/repo:v1" -> "ghcr.io/user/repo:v1"
pub fn normalize_reference(reference: &str) -> String {
    let mut r = reference.to_string();

    // If no registry specified, assume Docker Hub
    if !r.contains('/') || (!r.contains('.') && !r.contains(':') && r.matches('/').count() == 0) {
        // Simple name like "alpine" -> "docker.io/library/alpine"
        r = format!("docker.io/library/{r}");
    } else if r.matches('/').count() == 1 && !r.split('/').next().unwrap_or("").contains('.') {
        // "user/repo" -> "docker.io/user/repo"
        r = format!("docker.io/{r}");
    }

    // Add :latest if no tag or digest
    if !r.contains(':') && !r.contains('@') {
        r.push_str(":latest");
    }

    r
}

/// Sanitize a digest string for use as a directory name.
pub fn sanitize_digest(digest: &str) -> String {
    digest.trim_start_matches("sha256:").replace(':', "-")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_simple() {
        assert_eq!(
            normalize_reference("alpine"),
            "docker.io/library/alpine:latest"
        );
    }

    #[test]
    fn test_normalize_with_tag() {
        assert_eq!(
            normalize_reference("alpine:3.18"),
            "docker.io/library/alpine:3.18"
        );
    }

    #[test]
    fn test_normalize_with_user() {
        assert_eq!(
            normalize_reference("myuser/myimage"),
            "docker.io/myuser/myimage:latest"
        );
    }

    #[test]
    fn test_normalize_full_ref() {
        assert_eq!(
            normalize_reference("ghcr.io/user/repo:v1"),
            "ghcr.io/user/repo:v1"
        );
    }
}
