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

    /// Set the primary console device (e.g. "hvc0").
    ///
    /// Kernel messages and panics are additionally mirrored to the serial
    /// port (ttyS0, backed by the hypervisor's `--serial pty`) so
    /// `chvm console` works as a rescue console when guest init or the
    /// vsock path is broken. The kernel opens the *last* console= parameter
    /// as /dev/console, so the serial console is inserted *before* the
    /// primary one to keep /dev/console (guest init stdio) on `device`.
    pub fn console(mut self, device: &str) -> Self {
        if device != "ttyS0" && !self.parts.iter().any(|p| p == "console=ttyS0") {
            self.parts.push("console=ttyS0".into());
        }
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
            let encoded = base64::engine::general_purpose::STANDARD.encode(dir.as_bytes());
            self.parts.push(format!("chvm.workdir={encoded}"));
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
        assert_eq!(
            cmdline,
            "console=ttyS0 console=hvc0 root=/dev/vda rw init=/chvm-init"
        );
    }

    #[test]
    fn test_console_mirrors_kernel_messages_to_serial() {
        let cmdline = KernelCmdlineBuilder::new().console("hvc0").build();

        // Kernel messages/panics also go to the serial PTY...
        assert!(cmdline.contains("console=ttyS0"));
        // ...but the primary console (which the kernel uses for
        // /dev/console) stays hvc0, so it must come last.
        let hvc0_pos = cmdline.find("console=hvc0").unwrap();
        let ttys0_pos = cmdline.find("console=ttyS0").unwrap();
        assert!(ttys0_pos < hvc0_pos, "hvc0 must stay the primary console");
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
        assert_eq!(cmdline, "chvm.workdir=L2FwcA==");
    }

    #[test]
    fn test_workdir_empty() {
        let cmdline = KernelCmdlineBuilder::new().workdir("").build();
        assert_eq!(cmdline, "");
    }

    #[test]
    fn test_workdir_encoding_preserves_spaces() {
        let cmdline = KernelCmdlineBuilder::new()
            .workdir("/app with spaces")
            .build();
        assert_eq!(cmdline, "chvm.workdir=L2FwcCB3aXRoIHNwYWNlcw==");
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
        assert!(cmdline.contains("chvm.workdir=L2FwcA=="));
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
