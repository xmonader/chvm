use crate::cli::PullArgs;
use chvm_lib::oci::{registry_from_image_ref, resolve_credentials};
use chvm_lib::vm::VmManager;

pub async fn execute(manager: &mut VmManager, args: &PullArgs) -> anyhow::Result<()> {
    let registry = registry_from_image_ref(&args.image);
    let credentials = resolve_credentials(&registry);
    let cached = manager.pull(&args.image, &credentials).await?;
    let _short_digest = if cached.digest.len() > 19 {
        &cached.digest[..19]
    } else {
        &cached.digest
    };
    println!(
        "{}: Pulling from {}",
        &args.image,
        cached.refs.first().unwrap_or(&args.image)
    );
    println!("Digest: {}", cached.digest);
    println!("Status: Downloaded newer image for {}", &args.image);
    Ok(())
}
