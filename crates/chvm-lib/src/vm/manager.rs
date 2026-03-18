use std::collections::HashMap;
use std::path::{Path, PathBuf};

use chrono::Utc;
use tracing::{debug, info, warn};

use crate::config::Config;
use crate::utils::{atomic_write, FileLock};

/// Maximum retries when probing for mycelium IPv6 address after VM boot.
const MYCELIUM_PROBE_MAX_RETRIES: usize = 10;
/// Delay between mycelium IP probe retries.
const MYCELIUM_PROBE_RETRY_DELAY: std::time::Duration = std::time::Duration::from_secs(2);
use crate::error::{ChvmError, Result};
use crate::hypervisor;
use crate::network::mycelium::resolve_default_peers;
use crate::network::{self, NetworkBackend, SetupConfig};
use crate::oci;
use crate::paths::Paths;
use crate::storage::{self, PrepareConfig, StorageBackend, StorageResult};
use crate::vm::builder::{generate_vsock_cid, VmBuilder};
use crate::vm::state::*;

/// StateStore manages VM state persistence on disk.
pub struct StateStore {
    state_dir: PathBuf,
    states: HashMap<String, RuntimeState>,
}

impl StateStore {
    pub fn new(state_dir: &Path) -> Result<Self> {
        std::fs::create_dir_all(state_dir)?;
        let mut store = Self {
            state_dir: state_dir.to_path_buf(),
            states: HashMap::new(),
        };
        store.load_all()?;
        Ok(store)
    }

    fn state_path(&self, vm_id: &str) -> PathBuf {
        self.state_dir.join(vm_id).join("state.json")
    }

    fn vm_dir(&self, vm_id: &str) -> PathBuf {
        self.state_dir.join(vm_id)
    }

    fn load_all(&mut self) -> Result<()> {
        let entries = match std::fs::read_dir(&self.state_dir) {
            Ok(e) => e,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(e.into()),
        };

        for entry in entries.flatten() {
            if !entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false) {
                continue;
            }
            let vm_id = entry.file_name().to_string_lossy().to_string();
            if let Ok(state) = self.load_state(&vm_id) {
                self.states.insert(vm_id, state);
            }
        }
        Ok(())
    }

    fn load_state(&self, vm_id: &str) -> Result<RuntimeState> {
        let state_path = self.state_path(vm_id);
        let _lock = FileLock::shared(&state_path)?;
        self.load_state_inner(vm_id)
    }

    /// Load state from disk without acquiring a lock.
    /// Use when the caller already holds the lock.
    fn load_state_inner(&self, vm_id: &str) -> Result<RuntimeState> {
        let state_path = self.state_path(vm_id);
        let data = std::fs::read_to_string(&state_path)?;
        Ok(serde_json::from_str(&data)?)
    }

    pub fn save(&mut self, state: &RuntimeState) -> Result<()> {
        let vm_dir = self.vm_dir(&state.id);
        std::fs::create_dir_all(&vm_dir)?;

        let state_path = self.state_path(&state.id);
        let _lock = FileLock::exclusive(&state_path)?;

        let data = serde_json::to_string_pretty(state)?;
        atomic_write(&state_path, data.as_bytes())?;
        self.states.insert(state.id.clone(), state.clone());
        Ok(())
    }

    pub fn get(&self, vm_id: &str) -> Result<&RuntimeState> {
        self.states
            .get(vm_id)
            .ok_or_else(|| ChvmError::VmNotFound(vm_id.to_string()))
    }

    pub fn get_by_name(&self, name: &str) -> Result<&RuntimeState> {
        self.states
            .values()
            .find(|s| s.name == name)
            .ok_or_else(|| ChvmError::VmNotFound(name.to_string()))
    }

    /// Match a VM by unique ID prefix.
    fn get_by_id_prefix(&self, prefix: &str) -> Result<&RuntimeState> {
        let matches: Vec<&RuntimeState> = self
            .states
            .iter()
            .filter(|(id, _)| id.starts_with(prefix))
            .map(|(_, state)| state)
            .collect();

        match matches.len() {
            0 => Err(ChvmError::VmNotFound(prefix.to_string())),
            1 => Ok(matches[0]),
            _ => {
                let mut ids: Vec<&str> = matches.iter().map(|s| s.id.as_str()).collect();
                ids.sort_unstable();
                Err(ChvmError::VmAmbiguous(prefix.to_string(), ids.join(", ")))
            }
        }
    }

    /// Resolve a VM by ID or name, returning the state.
    ///
    /// Resolution order: exact ID → exact name → unique ID prefix.
    pub fn resolve(&self, id_or_name: &str) -> Result<&RuntimeState> {
        // 1. Exact ID match
        if let Ok(state) = self.get(id_or_name) {
            return Ok(state);
        }
        // 2. Exact name match
        if let Ok(state) = self.get_by_name(id_or_name) {
            return Ok(state);
        }
        // 3. ID prefix match
        self.get_by_id_prefix(id_or_name)
    }

    /// Resolve a VM by ID or name, returning a mutable clone.
    /// This also reloads the state from disk to get the latest version.
    pub fn resolve_clone(&mut self, id_or_name: &str) -> Result<RuntimeState> {
        let vm_id = self.resolve(id_or_name)?.id.clone();
        let state = self.load_state(&vm_id)?;
        self.states.insert(vm_id.clone(), state);
        self.states
            .get(&vm_id)
            .cloned()
            .ok_or_else(|| ChvmError::VmNotFound(id_or_name.to_string()))
    }

    pub fn list(&self) -> Vec<&RuntimeState> {
        self.states.values().collect()
    }

    pub fn delete(&mut self, vm_id: &str) -> Result<()> {
        let vm_dir = self.vm_dir(vm_id);
        if vm_dir.exists() {
            std::fs::remove_dir_all(&vm_dir)?;
        }
        self.states.remove(vm_id);
        Ok(())
    }

    pub fn exists_by_name(&self, name: &str) -> bool {
        self.states.values().any(|s| s.name == name)
    }

    pub fn update_status(&mut self, vm_id: &str, status: VmState) -> Result<()> {
        let state_path = self.state_path(vm_id);
        let _lock = FileLock::exclusive(&state_path)?;

        let state = self.load_state_inner(vm_id)?;
        self.states.insert(vm_id.to_string(), state);

        let state = self
            .states
            .get_mut(vm_id)
            .ok_or_else(|| ChvmError::VmNotFound(vm_id.to_string()))?;
        state.status = status;
        let data = serde_json::to_string_pretty(state)?;
        atomic_write(&state_path, data.as_bytes())?;
        Ok(())
    }
}

/// Options for creating a VM.
pub struct CreateOptions {
    pub name: Option<String>,
    pub image_ref: Option<String>,
    pub rootfs_path: Option<String>,
    pub vcpu: Option<u32>,
    pub memory_mb: Option<u32>,
    pub storage_backend: Option<String>,
    pub network_mode: Option<NetworkMode>,
    pub port_forwards: Vec<PortForward>,
    pub env: Vec<String>,
    pub cmd: Vec<String>,
    pub mounts: Vec<MountConfig>,
    pub kernel_path: Option<String>,
    pub detach: bool,
    pub remove_after: bool,
    pub mycelium_peers: Vec<String>,
    pub credentials: Option<oci::RegistryCredentials>,
}

pub struct ExecOptions {
    pub cmd: Vec<String>,
    pub env: Vec<String>,
    pub working_dir: Option<String>,
}

/// VmManager orchestrates VM lifecycle operations.
pub struct VmManager {
    pub config: Config,
    pub paths: Paths,
    pub state_store: StateStore,
    pub image_cache: oci::ImageCache,
}

impl VmManager {
    pub fn new(config: Config, paths: Paths) -> Result<Self> {
        let state_store = StateStore::new(&paths.vms)?;
        let image_cache = oci::ImageCache::new(&paths.images)?;

        Ok(Self {
            config,
            paths,
            state_store,
            image_cache,
        })
    }

    /// Pull an OCI image into the cache.
    pub async fn pull(
        &mut self,
        image_ref: &str,
        credentials: &oci::RegistryCredentials,
    ) -> Result<oci::CachedImage> {
        info!(image = image_ref, "Pulling image");

        // Try registry pull first
        match oci::pull::pull_image(image_ref, &self.paths.images, credentials).await {
            Ok(result) => {
                let cached = self.image_cache.add(&result)?;
                Ok(cached)
            }
            Err(registry_err) => {
                // Registry pull failed — try local container runtimes (docker, podman)
                if let Some(runtime) = oci::docker_import::find_local_runtime(image_ref) {
                    info!(
                        image = image_ref,
                        runtime = runtime,
                        "Registry pull failed, importing from local {runtime}"
                    );
                    let result = oci::docker_import::import_from_local(
                        runtime,
                        image_ref,
                        &self.paths.images,
                    )
                    .map_err(|e| ChvmError::OciError(format!("{runtime} import failed: {e}")))?;
                    let cached = self.image_cache.add(&result)?;
                    Ok(cached)
                } else {
                    Err(ChvmError::OciError(registry_err.to_string()))
                }
            }
        }
    }

    /// Create a new VM.
    pub async fn create(&mut self, opts: CreateOptions) -> Result<RuntimeState> {
        if opts.image_ref.is_none() && opts.rootfs_path.is_none() {
            return Err(ChvmError::Other(
                "either image_ref or rootfs_path must be specified".into(),
            ));
        }

        let vm_id = uuid::Uuid::new_v4().simple().to_string()[..12].to_string();
        let name = opts.name.clone().unwrap_or_else(|| vm_id.clone());

        if self.state_store.exists_by_name(&name) {
            return Err(ChvmError::VmAlreadyExists(name));
        }

        let credentials = opts.credentials.as_ref().cloned().unwrap_or_default();
        let (rootfs_path, image_ref, image_digest, image_config) =
            self.resolve_image_or_rootfs(&opts, &credentials).await?;

        let vm_dir = self.paths.vm_dir(&vm_id);
        std::fs::create_dir_all(&vm_dir)?;

        let vm_config =
            self.build_vm_config(&vm_id, &name, &opts, rootfs_path, image_config, &vm_dir)?;

        let state = RuntimeState {
            id: vm_id,
            name,
            image_ref,
            image_digest,
            vm_config,
            status: VmState::Created,
            vmm_pid: 0,
            exit_code: None,
            network_state: None,
            storage_state: None,
            created_at: Utc::now(),
            started_at: None,
            stopped_at: None,
        };

        self.state_store.save(&state)?;
        info!(vm_id = %state.id, "VM created");
        Ok(state)
    }

    /// Start a created VM.
    pub async fn start(&mut self, id_or_name: &str) -> Result<()> {
        let mut state = self.state_store.resolve_clone(id_or_name)?;

        if state.status == VmState::Running {
            return Err(ChvmError::VmAlreadyRunning);
        }

        info!(vm_id = %state.id, "Starting VM");

        let vm_dir = self.paths.vm_dir(&state.id);
        std::fs::create_dir_all(&vm_dir)?;

        // Prepare storage
        let storage_result = self.prepare_storage(&state, &vm_dir).await?;
        state.storage_state = Some(StorageState {
            virtiofs_socket: storage_result.socket_path.clone(),
            virtiofs_pid: storage_result.virtiofs_pid,
            block_device: storage_result.block_device.clone(),
            overlay_dir: storage_result.overlay_dir.clone(),
        });

        // Update rootfs source to point to prepared storage
        if let Some(ref block) = storage_result.block_device {
            state.vm_config.rootfs.source = block.clone();
        } else if let Some(ref sock) = storage_result.socket_path {
            state.vm_config.rootfs.source = sock.clone();
        }

        // Setup network
        if state.vm_config.network.mode != NetworkMode::None {
            match self.setup_network(&mut state, &vm_dir).await {
                Ok(()) => {}
                Err(e) => warn!(error = %e, "Failed to setup network, continuing without"),
            }
        }

        // Prepare virtiofs for volume mounts
        self.prepare_volume_mounts(&mut state, &vm_dir).await?;

        // Launch Cloud Hypervisor process
        let pid = hypervisor::process::launch(&state.vm_config, &vm_dir).await?;

        state.status = VmState::Running;
        state.started_at = Some(Utc::now());
        state.vmm_pid = pid;

        self.state_store.save(&state)?;
        info!(vm_id = %state.id, pid = pid, "VM started");

        // Probe mycelium IP after boot with retries
        if state.vm_config.network.mode == NetworkMode::Mycelium {
            if let Some(ip) = self.probe_mycelium_ip_with_retries(&state).await {
                if let Some(ref mut ns) = state.network_state {
                    ns.mycelium_ip = Some(ip.clone());
                }
                info!(vm_id = %state.id, mycelium_ip = %ip, "Mycelium IP detected");
                self.state_store.save(&state)?;
            }
        }

        Ok(())
    }

    /// Stop a running VM.
    pub async fn stop(&mut self, id_or_name: &str) -> Result<()> {
        let mut state = self.state_store.resolve_clone(id_or_name)?;

        if state.status != VmState::Running {
            return Err(ChvmError::VmNotRunning);
        }

        info!(vm_id = %state.id, "Stopping VM");

        // Try graceful shutdown via API
        let api_socket = state.vm_config.api_socket_path().unwrap_or_default();

        if !process_running(state.vmm_pid) {
            debug!(vm_id = %state.id, "VM process already exited");
        } else if let Err(e) = hypervisor::api::shutdown(&api_socket).await {
            warn!(error = %e, "Graceful shutdown failed, killing VM");
            kill_process(state.vmm_pid);
        } else {
            // Wait for process to exit
            wait_for_exit(state.vmm_pid, std::time::Duration::from_secs(30)).await;
        }

        // Cleanup network
        if state.vm_config.network.mode != NetworkMode::None {
            if let Err(e) = self.teardown_network(&state).await {
                warn!(error = %e, "Failed to teardown network");
            }
        }

        // Remove stale API socket so a subsequent start doesn't get EADDRINUSE
        let vm_dir = self.paths.vm_dir(&state.id);
        let _ = std::fs::remove_file(vm_dir.join("api.sock"));

        // Cleanup storage (virtiofsd processes, overlays)
        self.cleanup_storage(&state);

        state.status = VmState::Stopped;
        state.stopped_at = Some(Utc::now());
        self.state_store.save(&state)?;

        info!(vm_id = %state.id, "VM stopped");
        Ok(())
    }

    /// Remove a VM.
    pub async fn remove(&mut self, id_or_name: &str, force: bool) -> Result<()> {
        let state = self.state_store.resolve_clone(id_or_name)?;

        if state.status == VmState::Running {
            if !force {
                return Err(ChvmError::Other(
                    "VM is running, use --force to remove".into(),
                ));
            }
            self.stop(&state.id).await?;
        }

        // Always teardown network (idempotent — safe even if already torn down by stop())
        if state.vm_config.network.mode != NetworkMode::None {
            if let Err(e) = self.teardown_network(&state).await {
                warn!(error = %e, "Failed to teardown network during remove");
            }
        }

        info!(vm_id = %state.id, "Removing VM");
        self.cleanup_storage(&state);
        self.state_store.delete(&state.id)?;
        info!(vm_id = %state.id, "VM removed");
        Ok(())
    }

    /// List all VMs.
    pub fn list(&self) -> Vec<&RuntimeState> {
        self.state_store.list()
    }

    /// Get a specific VM by ID or name.
    pub fn get(&mut self, id_or_name: &str) -> Result<RuntimeState> {
        self.state_store.resolve_clone(id_or_name)
    }

    /// Read VM logs.
    pub fn logs(&self, id_or_name: &str) -> Result<String> {
        let state = self.state_store.resolve(id_or_name)?;
        let default_log = self.paths.vm_dir(&state.id).join("console.log");
        let default_log_str = default_log.to_string_lossy();
        let log_path = state
            .vm_config
            .log_path
            .as_deref()
            .unwrap_or(&default_log_str);
        match std::fs::read_to_string(log_path) {
            Ok(data) => Ok(data),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
            Err(e) => Err(e.into()),
        }
    }

    /// Execute a command in a running VM via vsock.
    pub async fn exec(&self, id_or_name: &str, opts: ExecOptions) -> Result<i32> {
        let state = self.state_store.resolve(id_or_name)?;
        if state.status != VmState::Running {
            return Err(ChvmError::VmNotRunning);
        }

        let vsock_path = state
            .vm_config
            .vsock_socket_path()
            .ok_or_else(|| ChvmError::VsockError("no state dir".into()))?;

        crate::vsock::proxy::exec_command(&vsock_path, &opts).await
    }

    /// Attach an interactive shell to a running VM via vsock.
    pub async fn attach(&self, id_or_name: &str) -> Result<i32> {
        let state = self.state_store.resolve(id_or_name)?;
        if state.status != VmState::Running {
            return Err(ChvmError::VmNotRunning);
        }

        let vsock_path = state
            .vm_config
            .vsock_socket_path()
            .ok_or_else(|| ChvmError::VsockError("no state dir".into()))?;

        crate::vsock::proxy::interactive_session(&vsock_path).await
    }

    /// Retrieve runtime counters (network, block, etc.) for a running VM.
    pub async fn stats(&self, id_or_name: &str) -> Result<(RuntimeState, serde_json::Value)> {
        let state = self.state_store.resolve(id_or_name)?;
        if state.status != VmState::Running {
            return Err(ChvmError::VmNotRunning);
        }

        let socket_path = state
            .vm_config
            .api_socket_path()
            .ok_or_else(|| ChvmError::ApiError("no state dir for API socket".into()))?;

        let counters = crate::hypervisor::api::vm_counters(&socket_path).await?;
        Ok((state.clone(), counters))
    }

    /// Refresh VM statuses by checking process liveness.
    /// When a VM's process is found dead, cleans up network and storage resources
    /// before marking it as stopped.
    pub async fn refresh_status(&mut self) -> Result<()> {
        let ids: Vec<String> = self.state_store.states.keys().cloned().collect();
        for id in ids {
            let Some(state) = self.state_store.states.get(&id).cloned() else {
                continue;
            };
            if state.status == VmState::Running
                && state.vmm_pid > 0
                && !process_running(state.vmm_pid)
            {
                info!(vm_id = %id, "Detected dead VM process, cleaning up resources");

                if state.vm_config.network.mode != NetworkMode::None {
                    if let Err(e) = self.teardown_network(&state).await {
                        warn!(vm_id = %id, error = %e, "Failed to teardown network for dead VM");
                    }
                }

                self.cleanup_storage(&state);

                let mut updated = state;
                updated.status = VmState::Stopped;
                updated.stopped_at = Some(Utc::now());
                if let Some(ref mut ns) = updated.network_state {
                    ns.mycelium_health = None;
                }
                if let Err(e) = self.state_store.save(&updated) {
                    warn!(vm_id = %id, error = %e, "Failed to persist stopped status");
                }
            }
        }
        Ok(())
    }

    /// Check mycelium health for a running VM by probing the guest.
    /// Uses a single-attempt probe (no retry loop) for fast results.
    /// Returns the health status and updates persisted state.
    pub async fn check_mycelium_health(&mut self, id_or_name: &str) -> Result<MyceliumHealth> {
        let mut state = self.state_store.resolve_clone(id_or_name)?;

        if state.status != VmState::Running {
            return Ok(MyceliumHealth::Unknown);
        }

        let mycelium_enabled = state
            .network_state
            .as_ref()
            .is_some_and(|ns| ns.mycelium_enabled);
        if !mycelium_enabled {
            return Ok(MyceliumHealth::Unknown);
        }

        let vsock_path = match state.vm_config.vsock_socket_path() {
            Some(p) => p,
            None => return Ok(MyceliumHealth::Unknown),
        };

        // Single-attempt probe with timeout
        let health_timeout = std::time::Duration::from_secs(5);

        let probed_ip = tokio::time::timeout(health_timeout, self.probe_mycelium_ip(&vsock_path))
            .await
            .ok()
            .flatten();

        let health = if probed_ip.is_some() {
            MyceliumHealth::Healthy
        } else {
            tokio::time::timeout(
                health_timeout,
                self.check_mycelium_process(&vsock_path, &state.id),
            )
            .await
            .unwrap_or(MyceliumHealth::Unknown)
        };

        // Update persisted state
        let Some(ref mut ns) = state.network_state else {
            return Ok(health);
        };

        if let Some(ref ip) = probed_ip {
            if ns.mycelium_ip.as_deref() != Some(ip) {
                info!(vm_id = %state.id, mycelium_ip = %ip, "Mycelium IP updated");
                ns.mycelium_ip = Some(ip.clone());
            }
        }
        ns.mycelium_health = Some(health);

        if let Err(e) = self.state_store.save(&state) {
            warn!(vm_id = %state.id, error = %e, "Failed to persist mycelium health update");
        }

        Ok(health)
    }

    /// Prune stopped VMs.
    pub async fn prune(&mut self) -> Result<Vec<String>> {
        let stopped: Vec<String> = self
            .state_store
            .list()
            .iter()
            .filter(|s| s.status == VmState::Stopped)
            .map(|s| s.id.clone())
            .collect();

        let mut pruned = Vec::new();
        for id in stopped {
            if let Err(e) = self.remove(&id, false).await {
                warn!(vm_id = %id, error = %e, "Failed to prune VM");
            } else {
                pruned.push(id);
            }
        }
        Ok(pruned)
    }

    // -- Private helpers --

    /// Resolve image or local rootfs, returning (rootfs_path, image_ref, digest, config).
    async fn resolve_image_or_rootfs(
        &mut self,
        opts: &CreateOptions,
        credentials: &oci::RegistryCredentials,
    ) -> Result<(String, String, Option<String>, Option<oci::ImageConfig>)> {
        if let Some(ref rootfs) = opts.rootfs_path {
            let abs = std::fs::canonicalize(rootfs)?;
            validate_rootfs(&abs)?;
            Ok((
                abs.to_string_lossy().to_string(),
                format!("local:{}", abs.display()),
                None,
                None,
            ))
        } else {
            let ref_str = opts.image_ref.as_deref().ok_or_else(|| {
                ChvmError::Other("image_ref is required when rootfs_path is not set".into())
            })?;
            let cached = match self.image_cache.get(ref_str) {
                Ok(cached) => cached,
                Err(_) => {
                    info!(image = ref_str, "Image not in cache, pulling...");
                    self.pull(ref_str, credentials).await?
                }
            };
            let rootfs = self.image_cache.ensure_rootfs(&cached)?;
            Ok((
                rootfs,
                ref_str.to_string(),
                Some(cached.digest.clone()),
                cached.config.clone(),
            ))
        }
    }

    /// Build the VmConfig from create options using VmBuilder.
    fn build_vm_config(
        &self,
        vm_id: &str,
        name: &str,
        opts: &CreateOptions,
        rootfs_path: String,
        image_config: Option<oci::ImageConfig>,
        vm_dir: &Path,
    ) -> Result<VmConfig> {
        let storage_type = match opts
            .storage_backend
            .as_deref()
            .unwrap_or(&self.config.default_storage_backend)
        {
            "virtiofs" => RootfsType::VirtioFs,
            _ => RootfsType::VirtioBlock,
        };

        let net_mode = opts
            .network_mode
            .unwrap_or_else(|| self.config.default_network_mode.parse().unwrap_or_default());
        let kernel_path =
            self.resolve_kernel_path(opts.kernel_path.as_deref(), &net_mode.to_string())?;

        let mut builder = VmBuilder::new(vm_id.to_string(), name.to_string())
            .rootfs(rootfs_path, storage_type)
            .vsock_cid(generate_vsock_cid(vm_id))
            .state_dir(vm_dir.to_string_lossy().to_string())
            .log_path(vm_dir.join("console.log").to_string_lossy().to_string())
            .kernel_path(kernel_path)
            .cmd(opts.cmd.clone())
            .env(opts.env.clone())
            .port_forwards(opts.port_forwards.clone())
            .mounts(opts.mounts.clone())
            .mycelium_peers({
                let effective_mode = opts.network_mode.unwrap_or_default();
                if effective_mode == NetworkMode::Mycelium && opts.mycelium_peers.is_empty() {
                    resolve_default_peers(&self.config.default_mycelium_peers)
                } else {
                    opts.mycelium_peers.clone()
                }
            });

        if let Some(ic) = image_config {
            builder = builder.image_config(ic);
        }
        if let Some(vcpu) = opts.vcpu {
            builder = builder.vcpu(vcpu);
        }
        if let Some(mem) = opts.memory_mb {
            builder = builder.memory_mb(mem);
        }
        if let Some(net) = opts.network_mode {
            builder = builder.network_mode(net);
        }
        if let Some(ref storage) = opts.storage_backend {
            builder = builder.storage_backend(storage.clone());
        }

        Ok(builder.build(&self.config))
    }

    fn resolve_kernel_path(&self, preferred: Option<&str>, network_mode: &str) -> Result<String> {
        if let Some(path) = preferred {
            if Path::new(path).exists() {
                return Ok(path.to_string());
            }
            return Err(ChvmError::KernelNotFound(path.to_string()));
        }

        // Use kernel manager to select
        let kernel_mgr = crate::kernel::KernelManager::new(&self.paths.kernels);
        kernel_mgr.select_kernel(network_mode)
    }

    async fn prepare_storage(&self, state: &RuntimeState, vm_dir: &Path) -> Result<StorageResult> {
        let backend: Box<dyn StorageBackend> = match state.vm_config.rootfs.rootfs_type {
            RootfsType::VirtioBlock => Box::new(storage::block::BlockStorage),
            RootfsType::VirtioFs => Box::new(storage::virtiofs::VirtioFsStorage),
        };

        let config = PrepareConfig {
            vm_id: state.id.clone(),
            rootfs_path: state.vm_config.rootfs.source.clone(),
            state_dir: vm_dir.to_path_buf(),
            read_only: state.vm_config.rootfs.read_only,
            tag: "rootfs".into(),
            inject_mycelium: state.vm_config.network.mode == NetworkMode::Mycelium,
        };

        backend
            .prepare(&config)
            .await
            .map_err(|e| ChvmError::StorageError(e.to_string()))
    }

    async fn setup_network(&self, state: &mut RuntimeState, vm_dir: &Path) -> Result<()> {
        let backend: Box<dyn NetworkBackend> = match state.vm_config.network.mode {
            NetworkMode::Tap => Box::new(network::tap::TapNetwork),
            NetworkMode::Mycelium => Box::new(network::mycelium::MyceliumNetwork),
            NetworkMode::None => return Ok(()),
        };

        let config = SetupConfig {
            vm_id: state.id.clone(),
            mode: state.vm_config.network.mode.to_string(),
            state_dir: vm_dir.to_path_buf(),
            mac_address: state.vm_config.network.mac_address.clone(),
            port_forwards: state.vm_config.network.port_forwards.clone(),
            mycelium_peers: state.vm_config.network.mycelium_peers.clone(),
            bridge: Some(self.config.default_bridge.clone()),
        };

        let result = backend
            .setup(&config)
            .await
            .map_err(|e| ChvmError::NetworkError(e.to_string()))?;

        state.network_state = Some(NetworkState {
            tap_device: result.tap_device.clone(),
            bridge: result.bridge.clone(),
            mac_address: result.mac_address.clone(),
            ip_address: result.ip_address.clone(),
            gateway: result.gateway.clone(),
            mycelium_enabled: result.mycelium_enabled,
            mycelium_peers: result.mycelium_peers.clone(),
            mycelium_ip: None,
            mycelium_health: None,
        });

        state.vm_config.network.tap_device = result.tap_device;
        state.vm_config.network.ip_address = result.ip_address.clone();
        state.vm_config.network.gateway = result.gateway.clone();

        // Update kernel cmdline with network info
        if let Some(ref ip) = result.ip_address {
            state
                .vm_config
                .kernel
                .cmdline
                .push_str(&format!(" chvm.ip={}", ip));
        }
        if let Some(ref gw) = result.gateway {
            state
                .vm_config
                .kernel
                .cmdline
                .push_str(&format!(" chvm.gw={}", gw));
        }
        if result.mycelium_enabled && !result.mycelium_peers.is_empty() {
            let peers_str = result.mycelium_peers.join(",");
            state
                .vm_config
                .kernel
                .cmdline
                .push_str(&format!(" chvm.mycelium_peer={}", peers_str));
        }

        Ok(())
    }

    async fn check_mycelium_process(&self, vsock_path: &str, vm_id: &str) -> MyceliumHealth {
        let ps_opts = ExecOptions {
            cmd: vec![
                "sh".into(),
                "-c".into(),
                "ps aux 2>/dev/null | grep '[m]ycelium' || ps 2>/dev/null | grep '[m]ycelium'"
                    .into(),
            ],
            env: vec![],
            working_dir: None,
        };

        let process_alive = matches!(
            crate::vsock::proxy::exec_capture(vsock_path, &ps_opts).await,
            Ok((0, _))
        );

        if process_alive {
            MyceliumHealth::Healthy
        } else {
            warn!(vm_id = %vm_id, "Mycelium process not running in guest");
            MyceliumHealth::Unhealthy
        }
    }

    /// Probe the guest for its mycelium IPv6 address via vsock.
    /// Tries `ip` command first, falls back to reading /proc/net/if_inet6
    /// for images that don't have iproute2 installed (e.g. nginx, redis).
    async fn probe_mycelium_ip(&self, vsock_path: &str) -> Option<String> {
        // Method 1: try `ip` command
        let ip_opts = ExecOptions {
            cmd: vec![
                "ip".into(),
                "-6".into(),
                "-o".into(),
                "addr".into(),
                "show".into(),
                "dev".into(),
                "mycelium".into(),
                "scope".into(),
                "global".into(),
            ],
            env: vec![],
            working_dir: None,
        };

        if let Ok((0, output)) = crate::vsock::proxy::exec_capture(vsock_path, &ip_opts).await {
            if let Some(ip) = parse_ip_command_output(&output) {
                return Some(ip);
            }
        }

        // Method 2: fall back to /proc/net/if_inet6 (works without iproute2)
        let proc_opts = ExecOptions {
            cmd: vec!["cat".into(), "/proc/net/if_inet6".into()],
            env: vec![],
            working_dir: None,
        };

        if let Ok((0, output)) = crate::vsock::proxy::exec_capture(vsock_path, &proc_opts).await {
            if let Some(ip) = parse_if_inet6_output(&output, "mycelium") {
                return Some(ip);
            }
        }

        None
    }

    /// Probe mycelium IP with retries. Used at boot time to wait for
    /// the mycelium daemon to start and acquire an address.
    async fn probe_mycelium_ip_with_retries(&self, state: &RuntimeState) -> Option<String> {
        let vsock_path = state.vm_config.vsock_socket_path()?;

        for attempt in 0..MYCELIUM_PROBE_MAX_RETRIES {
            if attempt > 0 {
                tokio::time::sleep(MYCELIUM_PROBE_RETRY_DELAY).await;
            }

            if let Some(ip) = self.probe_mycelium_ip(&vsock_path).await {
                return Some(ip);
            }

            debug!(
                attempt = attempt,
                "Mycelium IP probe attempt failed, retrying"
            );
        }

        warn!("Could not detect mycelium IP after retries");
        None
    }

    async fn teardown_network(&self, state: &RuntimeState) -> Result<()> {
        let backend: Box<dyn NetworkBackend> = match state.vm_config.network.mode {
            NetworkMode::Tap => Box::new(network::tap::TapNetwork),
            NetworkMode::Mycelium => Box::new(network::mycelium::MyceliumNetwork),
            NetworkMode::None => return Ok(()),
        };

        let teardown_config = network::TeardownConfig {
            vm_id: state.id.clone(),
            ip_address: state
                .network_state
                .as_ref()
                .and_then(|ns| ns.ip_address.clone()),
            port_forwards: state.vm_config.network.port_forwards.clone(),
        };

        backend
            .teardown(&teardown_config)
            .await
            .map_err(|e| ChvmError::NetworkError(e.to_string()))
    }

    async fn prepare_volume_mounts(&self, state: &mut RuntimeState, vm_dir: &Path) -> Result<()> {
        if state.vm_config.mounts.is_empty() {
            return Ok(());
        }

        let virtiofs = storage::virtiofs::VirtioFsStorage;
        let mut mount_infos = Vec::new();

        for (i, mount) in state.vm_config.mounts.iter_mut().enumerate() {
            let tag = format!("mount{}", i);
            let mount_dir = vm_dir.join(format!("mount-{}", i));
            std::fs::create_dir_all(&mount_dir)?;

            let config = PrepareConfig {
                vm_id: state.id.clone(),
                rootfs_path: mount.source.clone(),
                state_dir: mount_dir,
                read_only: mount.read_only,
                tag: tag.clone(),
                inject_mycelium: false,
            };

            match virtiofs.prepare(&config).await {
                Ok(result) => {
                    if let Some(ref sock) = result.socket_path {
                        mount.source = sock.clone();
                    }
                    mount.mount_type = Some("virtiofs".into());
                    mount.tag = Some(tag.clone());
                    mount_infos.push(format!("{}={}", tag, mount.target));
                }
                Err(e) => {
                    warn!(mount_source = %mount.source, error = %e, "Failed to prepare mount");
                }
            }
        }

        if !mount_infos.is_empty() {
            state
                .vm_config
                .kernel
                .cmdline
                .push_str(&format!(" chvm.mounts={}", mount_infos.join(",")));
        }

        Ok(())
    }

    fn cleanup_storage(&self, state: &RuntimeState) {
        // Kill virtiofsd processes and unmount overlays
        if let Some(ref ss) = state.storage_state {
            if let Some(pid) = ss.virtiofs_pid {
                kill_process(pid);
            }
            if let Some(ref overlay) = ss.overlay_dir {
                let merged = Path::new(overlay).join("merged");
                if let Err(e) = nix::mount::umount2(&merged, nix::mount::MntFlags::MNT_DETACH) {
                    warn!(path = %merged.display(), error = %e, "Failed to unmount overlay during cleanup");
                }
            }
        }
    }
}

fn validate_rootfs(path: &Path) -> Result<()> {
    if !path.is_dir() {
        return Err(ChvmError::InvalidRootfs(format!(
            "not a directory: {}",
            path.display()
        )));
    }
    let essential = ["bin", "usr", "lib", "etc"];
    let found = essential.iter().any(|d| path.join(d).exists());
    if !found {
        return Err(ChvmError::InvalidRootfs(
            "directory does not appear to be a valid rootfs (missing bin, usr, lib, or etc)".into(),
        ));
    }
    Ok(())
}

/// Parse `ip -6 -o addr show` output for a non-link-local IPv6 address.
fn parse_ip_command_output(output: &str) -> Option<String> {
    for line in output.lines() {
        let Some((_, rest)) = line.split_once("inet6 ") else {
            continue;
        };
        let Some((ip_raw, _)) = rest.split_once('/') else {
            continue;
        };
        let ip = ip_raw.trim();
        if !ip.starts_with("fe80") {
            return Some(ip.to_string());
        }
    }
    None
}

/// Parse `/proc/net/if_inet6` output for a non-link-local IPv6 on the given interface.
fn parse_if_inet6_output(output: &str, iface: &str) -> Option<String> {
    for line in output.lines() {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() < 6 || parts[5] != iface {
            continue;
        }
        let hex_addr = parts[0];
        if hex_addr.len() != 32 {
            continue;
        }
        let groups: [u16; 8] = std::array::from_fn(|i| {
            u16::from_str_radix(&hex_addr[i * 4..i * 4 + 4], 16).unwrap_or(0)
        });
        let ip = std::net::Ipv6Addr::from(groups).to_string();
        if !ip.starts_with("fe80") {
            return Some(ip);
        }
    }
    None
}

fn process_running(pid: i32) -> bool {
    if pid <= 0 {
        return false;
    }
    // Read /proc/<pid>/cmdline to verify it's actually cloud-hypervisor
    let cmdline_path = format!("/proc/{}/cmdline", pid);
    match std::fs::read(&cmdline_path) {
        Ok(data) => {
            let cmdline = String::from_utf8_lossy(&data);
            cmdline.contains("cloud-hypervisor")
        }
        Err(_) => false,
    }
}

/// Kill a process, but only if it's actually a cloud-hypervisor instance.
fn kill_process(pid: i32) {
    if pid <= 0 {
        return;
    }
    if !process_running(pid) {
        return;
    }
    let _ = nix::sys::signal::kill(
        nix::unistd::Pid::from_raw(pid),
        nix::sys::signal::Signal::SIGKILL,
    );
}

async fn wait_for_exit(pid: i32, timeout: std::time::Duration) {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        if !process_running(pid) {
            return;
        }
        if tokio::time::Instant::now() > deadline {
            kill_process(pid);
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_state(id: &str, name: &str) -> RuntimeState {
        RuntimeState {
            id: id.to_string(),
            name: name.to_string(),
            image_ref: "test:latest".to_string(),
            image_digest: None,
            vm_config: VmConfig {
                id: id.to_string(),
                name: name.to_string(),
                vcpu: 1,
                memory_mb: 512,
                kernel: KernelConfig {
                    path: "/kernel".to_string(),
                    cmdline: "console=ttyS0".to_string(),
                    initrd_path: None,
                },
                rootfs: RootfsConfig {
                    source: "/rootfs".to_string(),
                    rootfs_type: RootfsType::VirtioBlock,
                    read_only: false,
                },
                network: NetworkConfig {
                    mode: NetworkMode::None,
                    tap_device: None,
                    mac_address: None,
                    ip_address: None,
                    gateway: None,
                    port_forwards: vec![],
                    mycelium_peers: vec![],
                    mycelium_key: None,
                },
                init: InitConfig {
                    cmd: vec!["/bin/sh".to_string()],
                    env: vec![],
                    working_dir: None,
                },
                mounts: vec![],
                log_path: None,
                vsock_cid: 3,
                state_dir: None,
            },
            status: VmState::Created,
            vmm_pid: 0,
            exit_code: None,
            network_state: None,
            storage_state: None,
            created_at: Utc::now(),
            started_at: None,
            stopped_at: None,
        }
    }

    fn setup_store_with_states(states: &[(&str, &str)]) -> (tempfile::TempDir, StateStore) {
        let tmp = tempfile::tempdir().unwrap();
        let mut store = StateStore::new(tmp.path()).unwrap();
        for (id, name) in states {
            let state = make_state(id, name);
            store.save(&state).unwrap();
        }
        (tmp, store)
    }

    #[test]
    fn resolve_exact_id() {
        let (_tmp, store) = setup_store_with_states(&[("a1b2c3d4e5f6", "myvm")]);
        let state = store.resolve("a1b2c3d4e5f6").unwrap();
        assert_eq!(state.id, "a1b2c3d4e5f6");
    }

    #[test]
    fn resolve_exact_name() {
        let (_tmp, store) = setup_store_with_states(&[("a1b2c3d4e5f6", "myvm")]);
        let state = store.resolve("myvm").unwrap();
        assert_eq!(state.id, "a1b2c3d4e5f6");
    }

    #[test]
    fn resolve_id_prefix_unique() {
        let (_tmp, store) =
            setup_store_with_states(&[("a1b2c3d4e5f6", "vm1"), ("ffffff000000", "vm2")]);
        let state = store.resolve("a1b").unwrap();
        assert_eq!(state.id, "a1b2c3d4e5f6");
    }

    #[test]
    fn resolve_id_prefix_ambiguous() {
        let (_tmp, store) =
            setup_store_with_states(&[("a1b2c3d4e5f6", "vm1"), ("a1b2ffffff00", "vm2")]);
        let result = store.resolve("a1b2");
        assert!(matches!(result, Err(ChvmError::VmAmbiguous(_, _))));
    }

    #[test]
    fn resolve_not_found() {
        let (_tmp, store) = setup_store_with_states(&[("a1b2c3d4e5f6", "myvm")]);
        let result = store.resolve("zzz");
        assert!(matches!(result, Err(ChvmError::VmNotFound(_))));
    }

    #[test]
    fn resolve_name_takes_priority_over_prefix() {
        // VM1 has ID starting with "a1b", VM2 is named "a1b"
        let (_tmp, store) =
            setup_store_with_states(&[("a1b2c3d4e5f6", "vm1"), ("ffffff000000", "a1b")]);
        // "a1b" should resolve to VM2 (exact name match) not VM1 (prefix match)
        let state = store.resolve("a1b").unwrap();
        assert_eq!(state.id, "ffffff000000");
    }
}
