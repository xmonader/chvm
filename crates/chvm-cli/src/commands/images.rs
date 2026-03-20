use crate::cli::{ImageCommands, ImageRmArgs, ImagesArgs};
use crate::commands::truncate;
use chvm_lib::vm::VmManager;

pub fn execute(manager: &mut VmManager, args: &ImagesArgs) -> anyhow::Result<()> {
    match &args.command {
        Some(ImageCommands::Rm(args)) => execute_remove(manager, args),
        None => execute_list(manager),
    }
}

pub fn execute_remove(manager: &mut VmManager, args: &ImageRmArgs) -> anyhow::Result<()> {
    let result = manager.remove_image(&args.image)?;

    if result.deleted_image {
        println!("Removed cached image {}", result.digest);
    } else {
        let removed_ref = result
            .removed_refs
            .first()
            .map(String::as_str)
            .unwrap_or(&args.image);
        println!(
            "Removed ref {} from cached digest {} ({} refs remain)",
            removed_ref, result.digest, result.remaining_refs
        );
    }

    Ok(())
}

fn execute_list(manager: &VmManager) -> anyhow::Result<()> {
    let images = manager.image_cache.list();

    println!(
        "{:<50} {:<20} {:<12} {:<24}",
        "REPOSITORY:TAG", "DIGEST", "SIZE", "CREATED"
    );

    for img in images {
        let refs = img.refs.first().map(|r| r.as_str()).unwrap_or("<none>");
        let digest = if img.digest.len() > 18 {
            &img.digest[..18]
        } else {
            &img.digest
        };
        let size = format_size(img.size);
        let created = img.created_at.format("%Y-%m-%d %H:%M:%S").to_string();

        println!(
            "{:<50} {:<20} {:<12} {:<24}",
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
