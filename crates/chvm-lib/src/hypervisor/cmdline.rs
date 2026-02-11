use base64::Engine;

/// Builder for constructing kernel command line strings.
///
/// Passes configuration to guest init via `chvm.*` kernel parameters.
/// Complex data (cmd, env) is base64-encoded with null-byte separators.
pub struct KernelCmdlineBuilder {
    parts: Vec<String>,
}

impl KernelCmdlineBuilder {
    pub fn new() -> Self {
        Self { parts: Vec::new() }
    }

    pub fn console(mut self, device: &str) -> Self {
        self.parts.push(format!("console={device}"));
        self
    }

    pub fn root(mut self, device: &str) -> Self {
        self.parts.push(format!("root={device}"));
        self
    }

    pub fn rw(mut self, rw: bool) -> Self {
        if rw {
            self.parts.push("rw".into());
        } else {
            self.parts.push("ro".into());
        }
        self
    }

    pub fn init(mut self, path: &str) -> Self {
        self.parts.push(format!("init={path}"));
        self
    }

    /// Encode command as base64 with null-byte separated args.
    pub fn cmd(mut self, cmd: &[String]) -> Self {
        if !cmd.is_empty() {
            let joined = cmd.join("\0");
            let encoded = base64::engine::general_purpose::STANDARD.encode(joined.as_bytes());
            self.parts.push(format!("chvm.cmd={encoded}"));
        }
        self
    }

    /// Encode environment variables as base64 with null-byte separators.
    pub fn env(mut self, env: &[String]) -> Self {
        if !env.is_empty() {
            let joined = env.join("\0");
            let encoded = base64::engine::general_purpose::STANDARD.encode(joined.as_bytes());
            self.parts.push(format!("chvm.env={encoded}"));
        }
        self
    }

    pub fn workdir(mut self, dir: &str) -> Self {
        if !dir.is_empty() {
            self.parts.push(format!("chvm.workdir={dir}"));
        }
        self
    }

    pub fn raw(mut self, param: &str) -> Self {
        self.parts.push(param.to_string());
        self
    }

    pub fn build(self) -> String {
        self.parts.join(" ")
    }
}

impl Default for KernelCmdlineBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_cmdline() {
        let cmdline = KernelCmdlineBuilder::new()
            .console("hvc0")
            .root("/dev/vda")
            .rw(true)
            .init("/chvm-init")
            .build();
        assert_eq!(cmdline, "console=hvc0 root=/dev/vda rw init=/chvm-init");
    }

    #[test]
    fn test_cmd_encoding() {
        let cmdline = KernelCmdlineBuilder::new()
            .cmd(&["echo".into(), "hello".into()])
            .build();
        assert!(cmdline.starts_with("chvm.cmd="));
        // Decode and verify
        let encoded = cmdline.strip_prefix("chvm.cmd=").unwrap();
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .unwrap();
        assert_eq!(String::from_utf8(decoded).unwrap(), "echo\0hello");
    }

    #[test]
    fn test_env_encoding() {
        let cmdline = KernelCmdlineBuilder::new()
            .env(&["VAR1=value1".into(), "VAR2=value2".into()])
            .build();
        assert!(cmdline.starts_with("chvm.env="));
        let encoded = cmdline.strip_prefix("chvm.env=").unwrap();
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .unwrap();
        assert_eq!(
            String::from_utf8(decoded).unwrap(),
            "VAR1=value1\0VAR2=value2"
        );
    }

    #[test]
    fn test_workdir() {
        let cmdline = KernelCmdlineBuilder::new().workdir("/app").build();
        assert_eq!(cmdline, "chvm.workdir=/app");
    }

    #[test]
    fn test_workdir_empty() {
        let cmdline = KernelCmdlineBuilder::new().workdir("").build();
        assert_eq!(cmdline, "");
    }

    #[test]
    fn test_raw_parameter() {
        let cmdline = KernelCmdlineBuilder::new()
            .raw("custom.param=value")
            .build();
        assert_eq!(cmdline, "custom.param=value");
    }

    #[test]
    fn test_rw_false() {
        let cmdline = KernelCmdlineBuilder::new().rw(false).build();
        assert_eq!(cmdline, "ro");
    }

    #[test]
    fn test_combined_cmdline() {
        let cmdline = KernelCmdlineBuilder::new()
            .console("hvc0")
            .root("/dev/vda")
            .rw(true)
            .init("/chvm-init")
            .cmd(&["/bin/sh".into()])
            .env(&["PATH=/usr/bin".into()])
            .workdir("/app")
            .raw("quiet")
            .build();

        assert!(cmdline.contains("console=hvc0"));
        assert!(cmdline.contains("root=/dev/vda"));
        assert!(cmdline.contains("rw"));
        assert!(cmdline.contains("init=/chvm-init"));
        assert!(cmdline.contains("chvm.cmd="));
        assert!(cmdline.contains("chvm.env="));
        assert!(cmdline.contains("chvm.workdir=/app"));
        assert!(cmdline.contains("quiet"));
    }

    #[test]
    fn test_empty_cmd() {
        let cmdline = KernelCmdlineBuilder::new().cmd(&[]).build();
        assert_eq!(cmdline, "");
    }

    #[test]
    fn test_empty_env() {
        let cmdline = KernelCmdlineBuilder::new().env(&[]).build();
        assert_eq!(cmdline, "");
    }

    #[test]
    fn test_default_builder() {
        let builder = KernelCmdlineBuilder::default();
        assert_eq!(builder.build(), "");
    }
}
