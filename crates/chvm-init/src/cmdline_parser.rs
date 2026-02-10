use base64::Engine;

pub struct InitConfig {
    pub cmd: Vec<String>,
    pub env: Vec<String>,
    pub working_dir: String,
}

pub fn load_config() -> InitConfig {
    let mut config = InitConfig {
        cmd: vec!["/bin/sh".into()],
        env: Vec::new(),
        working_dir: "/".into(),
    };

    let cmdline = match std::fs::read_to_string("/proc/cmdline") {
        Ok(c) => c,
        Err(_) => return config,
    };

    for part in cmdline.split_whitespace() {
        if let Some(encoded) = part.strip_prefix("chvm.cmd=") {
            if let Ok(decoded) = base64::engine::general_purpose::STANDARD.decode(encoded) {
                if let Ok(s) = String::from_utf8(decoded) {
                    let args: Vec<String> = s.split('\0')
                        .filter(|a| !a.is_empty())
                        .map(|a| a.to_string())
                        .collect();
                    if !args.is_empty() {
                        config.cmd = args;
                    }
                }
            }
        }

        if let Some(encoded) = part.strip_prefix("chvm.env=") {
            if let Ok(decoded) = base64::engine::general_purpose::STANDARD.decode(encoded) {
                if let Ok(s) = String::from_utf8(decoded) {
                    for env_var in s.split('\0') {
                        if !env_var.is_empty() {
                            config.env.push(env_var.to_string());
                        }
                    }
                }
            }
        }

        if let Some(workdir) = part.strip_prefix("chvm.workdir=") {
            config.working_dir = workdir.to_string();
        }
    }

    config
}
