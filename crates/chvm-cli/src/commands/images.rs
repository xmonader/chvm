use crate::cli::ImagesArgs;
use crate::commands::truncate;
use chvm_lib::vm::VmManager;

pub fn execute(manager: &VmManager, _args: &ImagesArgs) -> anyhow::Result<()> {
    let images = manager.image_cache.list();

    println!("{:<50} {:<20} {:<12} {:<24}",
        "REPOSITORY:TAG", "DIGEST", "SIZE", "CREATED");

    for img in images {
        let refs = img.refs.first().map(|r| r.as_str()).unwrap_or("<none>");
        let digest = if img.digest.len() > 18 {
            &img.digest[..18]
        } else {
            &img.digest
        };
        let size = format_size(img.size);
        let created = img.created_at.format("%Y-%m-%d %H:%M:%S").to_string();

        println!("{:<50} {:<20} {:<12} {:<24}",
            truncate(refs, 48),
            digest,
            size,
            created,
        );
    }

    Ok(())
}

fn format_size(bytes: u64) -> String {
    if bytes >= 1_073_741_824 {
        format!("{:.1}GB", bytes as f64 / 1_073_741_824.0)
    } else if bytes >= 1_048_576 {
        format!("{:.1}MB", bytes as f64 / 1_048_576.0)
    } else if bytes >= 1024 {
        format!("{:.1}KB", bytes as f64 / 1024.0)
    } else {
        format!("{bytes}B")
    }
}

