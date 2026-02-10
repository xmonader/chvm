use crate::network::traits::*;

/// Mycelium networking backend.
/// Uses network namespaces with a mycelium daemon for IPv6 mesh networking.
pub struct MyceliumNetwork;

#[async_trait::async_trait]
impl NetworkBackend for MyceliumNetwork {
    fn name(&self) -> &str {
        "mycelium"
    }

    async fn setup(&self, config: &SetupConfig) -> anyhow::Result<NetworkResult> {
        // Mycelium networking setup:
        // 1. Create network namespace for the VM
        // 2. Create veth pair
        // 3. Start mycelium daemon in namespace
        // 4. The guest will also run mycelium with the configured peers

        // For now, we pass the configuration through to the guest via kernel cmdline.
        // The guest init will start the mycelium daemon.

        // We still use TAP for basic connectivity (needed for vsock proxy)
        let tap = super::tap::TapNetwork;
        let mut result = tap.setup(config).await?;

        result.mycelium_enabled = true;
        result.mycelium_peers = config.mycelium_peers.clone();

        Ok(result)
    }

    async fn teardown(&self, config: &TeardownConfig) -> anyhow::Result<()> {
        // Teardown TAP
        let tap = super::tap::TapNetwork;
        tap.teardown(config).await
    }
}
