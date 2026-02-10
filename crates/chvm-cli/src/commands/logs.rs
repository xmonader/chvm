use std::io::{Read, Seek, SeekFrom};

use crate::cli::LogsArgs;
use chvm_lib::vm::VmManager;

pub fn execute(manager: &VmManager, args: &LogsArgs) -> anyhow::Result<()> {
    let logs = manager.logs(&args.container)?;
    print!("{logs}");

    if args.follow {
        // Get the log file path to tail
        let state = manager.get(&args.container)?;
        let vm_dir = manager.paths.vm_dir(&state.id);
        let default_log = vm_dir.join("console.log");
        let default_log_str = default_log.to_string_lossy();
        let log_path = state.vm_config.log_path.as_deref()
            .unwrap_or(&default_log_str);

        let mut file = std::fs::File::open(log_path)?;
        // Seek to current end
        let mut pos = file.seek(SeekFrom::End(0))?;

        loop {
            std::thread::sleep(std::time::Duration::from_millis(250));

            let meta = std::fs::metadata(log_path)?;
            let len = meta.len();
            if len > pos {
                file.seek(SeekFrom::Start(pos))?;
                let mut buf = vec![0u8; (len - pos) as usize];
                let n = file.read(&mut buf)?;
                if n > 0 {
                    print!("{}", String::from_utf8_lossy(&buf[..n]));
                }
                pos = len;
            }
        }
    }

    Ok(())
}
