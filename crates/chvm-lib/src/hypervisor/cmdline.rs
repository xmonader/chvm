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
        let decoded = base64::engine::general_purpose::STANDARD.decode(encoded).unwrap();
        assert_eq!(String::from_utf8(decoded).unwrap(), "echo\0hello");
    }
}
