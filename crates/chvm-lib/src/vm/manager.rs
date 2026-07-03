use std::collections::HashMap;
use std::path::{Path, PathBuf};

use chrono::Utc;
use tracing::{debug, info, warn};

use crate::config::Config;
use crate::kernel::KernelManager;
use crate::utils::{atomic_write, FileLock};

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
            // Skip internal directories (e.g. the `.locks` dir) which are not VMs.
            if vm_id.starts_with('.') {
                continue;
            }
            if let Ok(state) = self.load_state(&vm_id) {
                self.states.insert(vm_id, state);
            }
        }
        Ok(())
    }

    /// Acquire an exclusive lock covering a whole lifecycle operation (start,
    /// stop, remove) on a single VM. Lives outside the per-VM directory so it
    /// survives `delete()` removing that directory.
    pub fn op_lock(&self, vm_id: &str) -> Result<FileLock> {
        let locks_dir = self.state_dir.join(".locks");
        std::fs::create_dir_all(&locks_dir)?;
        Ok(FileLock::exclusive(&locks_dir.join(vm_id))?)
    }

    /// Acquire the global creation lock, serializing the name/ID uniqueness
    /// check with the subsequent save so concurrent creates can't collide.
    pub fn global_lock(&self) -> Result<FileLock> {
        std::fs::create_dir_all(&self.state_dir)?;
        Ok(FileLock::exclusive(&self.state_dir.join(".global"))?)
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

    pub fn reload(&mut self) -> Result<()> {
        self.states.clear();
        self.load_all()
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
        // Hold the state-file lock so a concurrent save() can't recreate the
        // directory we are removing.
        let _lock = FileLock::exclusive(&self.state_path(vm_id))?;
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
    pub block_disk_size_bytes: Option<u64>,
    pub writable_quota_bytes: Option<u64>,
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

        // Early best-effort check for a friendlier error before the (slow) pull.
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

        // Serialize the authoritative name/ID uniqueness check with the save so
        // two concurrent `create --name X` can't both succeed.
        let _global = self.state_store.global_lock()?;
        self.state_store.reload()?;
        if self.state_store.exists_by_name(&name) {
            let _ = std::fs::remove_dir_all(&vm_dir);
            return Err(ChvmError::VmAlreadyExists(name));
        }
        if self.state_store.get(&vm_id).is_ok() {
            let _ = std::fs::remove_dir_all(&vm_dir);
            return Err(ChvmError::VmAlreadyExists(vm_id));
        }

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
            network_ownership: None,
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
        let vm_id = self.state_store.resolve(id_or_name)?.id.clone();
        // Hold the per-VM lock for the whole start so a concurrent start can't
        // double-launch the hypervisor; re-read status from disk under the lock.
        let _op = self.state_store.op_lock(&vm_id)?;
        let mut state = self.state_store.resolve_clone(&vm_id)?;

        if state.status == VmState::Running {
            return Err(ChvmError::VmAlreadyRunning);
        }

        info!(vm_id = %state.id, "Starting VM");

        // Rebuild the runtime kernel cmdline from the pristine base so repeated
        // start/stop cycles don't accumulate stale/duplicate chvm.* parameters.
        strip_runtime_cmdline_params(&mut state.vm_config.kernel.cmdline);

        let vm_dir = self.paths.vm_dir(&state.id);
        std::fs::create_dir_all(&vm_dir)?;
        self.cleanup_runtime_artifacts(&vm_dir);

        // Prepare storage
        let storage_result = self.prepare_storage(&state, &vm_dir).await?;
        state.storage_state = Some(StorageState {
            virtiofs_socket: storage_result.socket_path.clone(),
            virtiofs_pid: storage_result.virtiofs_pid,
            block_device: storage_result.block_device.clone(),
            overlay_dir: storage_result.overlay_dir.clone(),
            overlay_backing_dir: storage_result.overlay_backing_dir.clone(),
        });

        // Update rootfs source to point to prepared storage
        if let Some(ref block) = storage_result.block_device {
            state.vm_config.rootfs.source = block.clone();
        } else if let Some(ref sock) = storage_result.socket_path {
            state.vm_config.rootfs.source = sock.clone();
        }

        // Setup network. A VM the user asked to be networked must not silently
        // boot without it — fail the start and tear down partial resources.
        if state.vm_config.network.mode != NetworkMode::None {
            if let Err(e) = self.setup_network(&mut state, &vm_dir).await {
                let _ = self.teardown_network(&state).await;
                self.cleanup_failed_start(&state, &vm_dir).await;
                return Err(e);
            }
        }

        // Prepare virtiofs for volume mounts
        if let Err(e) = self.prepare_volume_mounts(&mut state, &vm_dir).await {
            self.cleanup_failed_start(&state, &vm_dir).await;
            return Err(e);
        }

        // Launch Cloud Hypervisor process
        let pid = match hypervisor::process::launch(&state.vm_config, &vm_dir).await {
            Ok(pid) => pid,
            Err(e) => {
                self.cleanup_failed_start(&state, &vm_dir).await;
                state.status = VmState::Failed;
                state.stopped_at = Some(Utc::now());
                state.vmm_pid = 0;
                state.network_state = None;
                state.vm_config.network.tap_device = None;
                state.vm_config.network.ip_address = None;
                state.vm_config.network.gateway = None;
                self.state_store.save(&state)?;
                return Err(e);
            }
        };

        state.status = VmState::Running;
        state.started_at = Some(Utc::now());
        state.vmm_pid = pid;

        self.state_store.save(&state)?;
        info!(vm_id = %state.id, pid = pid, "VM started");

        Ok(())
    }

    /// Stop a running VM.
    pub async fn stop(&mut self, id_or_name: &str) -> Result<()> {
        let vm_id = self.state_store.resolve(id_or_name)?.id.clone();
        let _op = self.state_store.op_lock(&vm_id)?;
        let mut state = self.state_store.resolve_clone(&vm_id)?;

        if state.status != VmState::Running {
            return Err(ChvmError::VmNotRunning);
        }

        self.stop_running(&mut state).await
    }

    /// Stop the hypervisor and tear down resources for an already-resolved,
    /// Running VM. Assumes the caller holds the per-VM op lock.
    async fn stop_running(&mut self, state: &mut RuntimeState) -> Result<()> {
        info!(vm_id = %state.id, "Stopping VM");

        // Try graceful shutdown via API
        let api_socket = state
            .vm_config
            .api_socket_path()
            .ok_or_else(|| ChvmError::ProcessError("running VM has no API socket path".into()))?;

        if !process_is_vm(state.vmm_pid, &api_socket) {
            debug!(vm_id = %state.id, "VM process already exited");
        } else if let Err(e) = hypervisor::api::shutdown(&api_socket).await {
            warn!(error = %e, "Graceful shutdown failed, killing VM");
            kill_vm_process(state.vmm_pid, &api_socket);
        } else {
            // Wait for process to exit
            wait_for_exit(
                state.vmm_pid,
                &api_socket,
                std::time::Duration::from_secs(30),
            )
            .await;
        }

        // Confirm the process is actually gone. A SIGKILL is asynchronous — the
        // kernel needs a moment to tear the process down and have it reaped — so
        // poll briefly before declaring it stuck, otherwise we'd race the kill
        // and wrongly mark a cleanly-killed VM as Failed.
        let died =
            wait_until_gone(state.vmm_pid, &api_socket, std::time::Duration::from_secs(5)).await;

        // Cleanup network
        if state.vm_config.network.mode != NetworkMode::None {
            if let Err(e) = self.teardown_network(state).await {
                warn!(error = %e, "Failed to teardown network");
            }
        }

        // Remove stale runtime sockets so a subsequent start doesn't get EADDRINUSE
        let vm_dir = self.paths.vm_dir(&state.id);
        self.cleanup_runtime_artifacts(&vm_dir);

        // Cleanup storage (virtiofsd processes, overlays)
        self.cleanup_storage(state);

        state.status = if died {
            VmState::Stopped
        } else {
            VmState::Failed
        };
        state.stopped_at = Some(Utc::now());
        self.state_store.save(state)?;

        if died {
            info!(vm_id = %state.id, "VM stopped");
            Ok(())
        } else {
            Err(ChvmError::ProcessError(format!(
                "VM {} did not exit after SIGKILL",
                state.id
            )))
        }
    }

    /// Remove a VM.
    pub async fn remove(&mut self, id_or_name: &str, force: bool) -> Result<()> {
        let vm_id = self.state_store.resolve(id_or_name)?.id.clone();
        let _op = self.state_store.op_lock(&vm_id)?;
        let mut state = self.state_store.resolve_clone(&vm_id)?;

        if state.status == VmState::Running {
            if !force {
                return Err(ChvmError::Other(
                    "VM is running, use --force to remove".into(),
                ));
            }
            self.stop_running(&mut state).await?;
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
    pub async fn stats(&mut self, id_or_name: &str) -> Result<(RuntimeState, serde_json::Value)> {
        self.refresh_status().await?;

        let state = self.state_store.resolve_clone(id_or_name)?;
        if state.status != VmState::Running {
            return Err(ChvmError::VmNotRunning);
        }

        let socket_path = state
            .vm_config
            .api_socket_path()
            .ok_or_else(|| ChvmError::ApiError("no state dir for API socket".into()))?;

        let counters = match crate::hypervisor::api::vm_counters(&socket_path).await {
            Ok(counters) => counters,
            Err(_) if !process_is_vm(state.vmm_pid, &socket_path) => {
                self.refresh_status().await?;
                return Err(ChvmError::VmNotRunning);
            }
            Err(err) => return Err(err),
        };
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
            // A VM is alive only if its PID still maps to a cloud-hypervisor
            // whose cmdline carries this VM's unique api-socket path; a bare
            // "cloud-hypervisor" match would false-positive on PID reuse.
            let alive = state.vmm_pid > 0
                && state
                    .vm_config
                    .api_socket_path()
                    .map(|sock| process_is_vm(state.vmm_pid, &sock))
                    .unwrap_or(false);
            if state.status == VmState::Running && !alive {
                info!(vm_id = %id, "Detected dead VM process, cleaning up resources");

                if state.vm_config.network.mode != NetworkMode::None {
                    if let Err(e) = self.teardown_network(&state).await {
                        warn!(vm_id = %id, error = %e, "Failed to teardown network for dead VM");
                    }
                }

                self.cleanup_storage(&state);
                self.cleanup_runtime_artifacts(&self.paths.vm_dir(&id));

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

        let probed_ip = self
            .probe_mycelium_ip_with_timeout(&vsock_path, health_timeout)
            .await;

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

    pub fn remove_image(&mut self, image: &str) -> Result<oci::RemovedImage> {
        let cached = self.image_cache.get(image)?;
        let dependents: Vec<String> = self
            .state_store
            .list()
            .iter()
            .filter(|state| state.image_digest.as_deref() == Some(cached.digest.as_str()))
            .map(|state| format!("{} ({})", state.name, state.id))
            .collect();

        if !dependents.is_empty() {
            return Err(ChvmError::ImageInUse(format!(
                "{} referenced by {}",
                cached.digest,
                dependents.join(", ")
            )));
        }

        if image.starts_with("sha256:") {
            self.image_cache.remove_digest(image)
        } else {
            self.image_cache.remove_reference(image)
        }
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
        let net_mode = opts
            .network_mode
            .unwrap_or_else(|| self.config.default_network_mode.parse().unwrap_or_default());
        let kernel_path =
            self.resolve_kernel_path(opts.kernel_path.as_deref(), &net_mode.to_string())?;
        let storage_backend =
            self.select_storage_backend(opts.storage_backend.as_deref(), &kernel_path)?;
        validate_storage_options(&storage_backend, opts)?;
        let storage_type = match storage_backend.as_str() {
            "virtiofs" => RootfsType::VirtioFs,
            "block" => RootfsType::VirtioBlock,
            other => {
                return Err(ChvmError::ConfigError(format!(
                    "invalid storage backend: {other}"
                )));
            }
        };

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
                if net_mode == NetworkMode::Mycelium && opts.mycelium_peers.is_empty() {
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
        if let Some(size) = opts.block_disk_size_bytes {
            builder = builder.block_disk_size_bytes(size);
        }
        if let Some(size) = opts.writable_quota_bytes {
            builder = builder.writable_quota_bytes(size);
        }
        builder = builder.storage_backend(storage_backend);

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

    async fn prepare_storage(
        &mut self,
        state: &RuntimeState,
        vm_dir: &Path,
    ) -> Result<StorageResult> {
        let backend: Box<dyn StorageBackend> = match state.vm_config.rootfs.rootfs_type {
            RootfsType::VirtioBlock => Box::new(storage::block::BlockStorage),
            RootfsType::VirtioFs => Box::new(storage::virtiofs::VirtioFsStorage),
        };
        let inject_mycelium = state.vm_config.network.mode == NetworkMode::Mycelium;
        let (rootfs_path, inject_runtime_binaries) =
            self.resolve_storage_rootfs_source(state, inject_mycelium)?;

        let config = PrepareConfig {
            vm_id: state.id.clone(),
            rootfs_path,
            state_dir: vm_dir.to_path_buf(),
            read_only: state.vm_config.rootfs.read_only,
            tag: "rootfs".into(),
            inject_mycelium,
            inject_runtime_binaries,
            block_disk_size_bytes: state.vm_config.rootfs.block_disk_size_bytes,
            writable_quota_bytes: state.vm_config.rootfs.writable_quota_bytes,
        };

        backend
            .prepare(&config)
            .await
            .map_err(|e| ChvmError::StorageError(e.to_string()))
    }

    fn select_storage_backend(&self, requested: Option<&str>, kernel_path: &str) -> Result<String> {
        let backend = choose_storage_backend(
            requested,
            &self.config.default_storage_backend,
            self.can_use_virtiofs(Path::new(kernel_path)),
        )?;

        if requested.is_none()
            && self.config.default_storage_backend == "virtiofs"
            && backend == "block"
        {
            warn!(
                kernel = kernel_path,
                "Default virtiofs storage unavailable on this host, falling back to block"
            );
        }

        Ok(backend)
    }

    fn can_use_virtiofs(&self, kernel_path: &Path) -> bool {
        if storage::virtiofs::find_virtiofsd_path(self.config.virtiofsd_bin.as_deref()).is_err() {
            return false;
        }

        let kernel_mgr = KernelManager::new(&self.paths.kernels);
        kernel_mgr.kernel_supports_virtiofs_root(kernel_path)
    }

    fn resolve_storage_rootfs_source(
        &mut self,
        state: &RuntimeState,
        inject_mycelium: bool,
    ) -> Result<(String, bool)> {
        if let Some(ref digest) = state.image_digest {
            let cached = self.image_cache.get(digest)?;
            return match state.vm_config.rootfs.rootfs_type {
                RootfsType::VirtioFs => Ok((
                    self.image_cache
                        .ensure_prepared_rootfs(&cached, inject_mycelium)?,
                    false,
                )),
                RootfsType::VirtioBlock => Ok((self.image_cache.ensure_rootfs(&cached)?, true)),
            };
        }

        if let Some(local_rootfs) = state.image_ref.strip_prefix("local:") {
            return Ok((local_rootfs.to_string(), true));
        }

        Ok((state.vm_config.rootfs.source.clone(), true))
    }

    async fn setup_network(&mut self, state: &mut RuntimeState, vm_dir: &Path) -> Result<()> {
        let _network_lock = FileLock::exclusive(&self.paths.vms.join(".network-allocation"))?;
        self.state_store.reload()?;

        let backend: Box<dyn NetworkBackend> = match state.vm_config.network.mode {
            NetworkMode::Tap => Box::new(network::tap::TapNetwork),
            NetworkMode::Mycelium => Box::new(network::mycelium::MyceliumNetwork),
            NetworkMode::None => return Ok(()),
        };

        let allocated_ip = match state.vm_config.network.mode {
            NetworkMode::Tap | NetworkMode::Mycelium => Some(self.allocate_bridge_ip(state)?),
            NetworkMode::None => None,
        };
        let allocated_gateway = allocated_ip
            .as_ref()
            .map(|_| network::bridge::GATEWAY.to_string());

        let config = SetupConfig {
            vm_id: state.id.clone(),
            mode: state.vm_config.network.mode.to_string(),
            state_dir: vm_dir.to_path_buf(),
            mac_address: state.vm_config.network.mac_address.clone(),
            ip_address: allocated_ip,
            gateway: allocated_gateway,
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
        let dns_servers = crate::utils::discover_host_dns_servers();
        if !dns_servers.is_empty() {
            state
                .vm_config
                .kernel
                .cmdline
                .push_str(&format!(" chvm.dns={}", dns_servers.join(",")));
        }
        if result.mycelium_enabled && !result.mycelium_peers.is_empty() {
            let peers_str = result.mycelium_peers.join(",");
            state
                .vm_config
                .kernel
                .cmdline
                .push_str(&format!(" chvm.mycelium_peer={}", peers_str));
        }

        // Persist the reserved IP before releasing the allocation lock so concurrent
        // starts cannot pick the same address.
        self.state_store.save(state)?;

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

    async fn probe_mycelium_ip_with_timeout(
        &self,
        vsock_path: &str,
        timeout: std::time::Duration,
    ) -> Option<String> {
        tokio::time::timeout(timeout, self.probe_mycelium_ip(vsock_path))
            .await
            .ok()
            .flatten()
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
                inject_runtime_binaries: false,
                block_disk_size_bytes: None,
                writable_quota_bytes: None,
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
                kill_child_process(pid);
            }
            if let Some(ref overlay) = ss.overlay_dir {
                let merged = Path::new(overlay).join("merged");
                if let Err(e) = nix::mount::umount2(&merged, nix::mount::MntFlags::MNT_DETACH) {
                    warn!(path = %merged.display(), error = %e, "Failed to unmount overlay during cleanup");
                }
            }
            if let Some(ref backing) = ss.overlay_backing_dir {
                if let Err(e) =
                    nix::mount::umount2(Path::new(backing), nix::mount::MntFlags::MNT_DETACH)
                {
                    warn!(path = %backing, error = %e, "Failed to unmount quota backing during cleanup");
                }
            }
        }
        let vm_dir = self.paths.vm_dir(&state.id);
        cleanup_virtiofs_state_dir(&vm_dir.join("storage"));
        if let Ok(entries) = std::fs::read_dir(&vm_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if !entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false) {
                    continue;
                }
                if !entry.file_name().to_string_lossy().starts_with("mount-") {
                    continue;
                }
                cleanup_virtiofs_state_dir(&path.join("storage"));
            }
        }
    }

    async fn cleanup_failed_start(&self, state: &RuntimeState, vm_dir: &Path) {
        if state.network_state.is_some() {
            if let Err(e) = self.teardown_network(state).await {
                warn!(vm_id = %state.id, error = %e, "Failed to teardown network after start failure");
            }
        }
        self.cleanup_storage(state);
        self.cleanup_runtime_artifacts(vm_dir);
    }

    fn cleanup_runtime_artifacts(&self, vm_dir: &Path) {
        for socket_name in ["api.sock", "vsock.sock"] {
            let socket_path = vm_dir.join(socket_name);
            if let Err(e) = std::fs::remove_file(&socket_path) {
                if e.kind() != std::io::ErrorKind::NotFound {
                    warn!(path = %socket_path.display(), error = %e, "Failed to remove stale socket");
                }
            }
        }
    }

    fn allocate_bridge_ip(&self, state: &RuntimeState) -> Result<String> {
        let used_octets = self.used_bridge_octets(&state.id);
        let preferred_octet = bridge_ip_octet(state);

        if let Some(octet) = preferred_octet {
            if !used_octets.contains(&octet) {
                return Ok(format!(
                    "{}.{}{}",
                    network::bridge::SUBNET_PREFIX,
                    octet,
                    "/24"
                ));
            }
        }

        for octet in 2u16..255 {
            if !used_octets.contains(&octet) {
                return Ok(format!(
                    "{}.{}{}",
                    network::bridge::SUBNET_PREFIX,
                    octet,
                    "/24"
                ));
            }
        }

        Err(ChvmError::NetworkError(
            "bridge subnet exhausted: no free IPv4 addresses available".into(),
        ))
    }

    fn used_bridge_octets(&self, exclude_vm_id: &str) -> std::collections::HashSet<u16> {
        self.state_store
            .list()
            .into_iter()
            .filter(|state| state.id != exclude_vm_id)
            .filter(|state| matches!(state.status, VmState::Created | VmState::Running))
            .filter_map(bridge_ip_octet)
            .chain(std::iter::once(1))
            .collect()
    }
}

fn bridge_ip_octet(state: &RuntimeState) -> Option<u16> {
    state
        .network_state
        .as_ref()
        .and_then(|ns| ns.ip_address.as_deref())
        .or(state.vm_config.network.ip_address.as_deref())
        .and_then(parse_bridge_ip_octet)
}

fn parse_bridge_ip_octet(ip: &str) -> Option<u16> {
    let addr = ip.split('/').next()?;
    if !addr.starts_with(&format!("{}.", network::bridge::SUBNET_PREFIX)) {
        return None;
    }
    addr.rsplit('.').next()?.parse().ok()
}

fn choose_storage_backend(
    requested_backend: Option<&str>,
    default_backend: &str,
    virtiofs_available: bool,
) -> Result<String> {
    match requested_backend {
        Some("block") => Ok("block".into()),
        Some("virtiofs") if virtiofs_available => Ok("virtiofs".into()),
        Some("virtiofs") => Err(ChvmError::ConfigError(
            "virtiofs storage requested but unavailable for the selected kernel/host".into(),
        )),
        Some(other) => Err(ChvmError::ConfigError(format!(
            "invalid storage backend: {other}"
        ))),
        None => match default_backend {
            "block" => Ok("block".into()),
            "virtiofs" if virtiofs_available => Ok("virtiofs".into()),
            "virtiofs" => Ok("block".into()),
            "auto" if virtiofs_available => Ok("virtiofs".into()),
            "auto" => Ok("block".into()),
            other => Err(ChvmError::ConfigError(format!(
                "invalid storage backend: {other}"
            ))),
        },
    }
}

fn validate_storage_options(storage_backend: &str, opts: &CreateOptions) -> Result<()> {
    match storage_backend {
        "block" if opts.writable_quota_bytes.is_some() => Err(ChvmError::ConfigError(
            "--storage-quota can only be used with the virtiofs storage backend".into(),
        )),
        "virtiofs" if opts.block_disk_size_bytes.is_some() => Err(ChvmError::ConfigError(
            "--disk-size can only be used with the block storage backend".into(),
        )),
        "block" | "virtiofs" => Ok(()),
        other => Err(ChvmError::ConfigError(format!(
            "invalid storage backend: {other}"
        ))),
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

    if let Some((candidate, machine)) = detect_rootfs_elf_machine(path)? {
        const EM_X86_64: u16 = 62;
        if machine != EM_X86_64 {
            return Err(ChvmError::InvalidRootfs(format!(
                "rootfs executable {} targets unsupported ELF machine {} (expected x86_64)",
                candidate, machine
            )));
        }
    }

    Ok(())
}

fn detect_rootfs_elf_machine(path: &Path) -> Result<Option<(String, u16)>> {
    let candidates = ["bin/sh", "bin/bash", "bin/busybox", "bin/ls", "usr/bin/env"];

    for candidate in candidates {
        let Some(resolved) = resolve_rootfs_executable(path, candidate)? else {
            continue;
        };
        let Some(machine) = read_elf_machine(&resolved)? else {
            continue;
        };
        return Ok(Some((candidate.to_string(), machine)));
    }

    Ok(None)
}

fn resolve_rootfs_executable(rootfs: &Path, relative_path: &str) -> Result<Option<PathBuf>> {
    let mut current = rootfs.join(relative_path);
    if std::fs::symlink_metadata(&current).is_err() {
        return Ok(None);
    }

    for _ in 0..8 {
        let metadata = std::fs::symlink_metadata(&current)?;
        if metadata.file_type().is_symlink() {
            let target = std::fs::read_link(&current)?;
            current = if target.is_absolute() {
                rootfs.join(target.strip_prefix("/").unwrap_or(&target))
            } else {
                current
                    .parent()
                    .unwrap_or(rootfs)
                    .join(target)
                    .components()
                    .collect()
            };
            continue;
        }

        if metadata.is_file() {
            return Ok(Some(current));
        }
        return Ok(None);
    }

    Err(ChvmError::InvalidRootfs(format!(
        "too many symlink levels while resolving {}",
        relative_path
    )))
}

fn read_elf_machine(path: &Path) -> Result<Option<u16>> {
    let bytes = std::fs::read(path)?;
    if bytes.len() < 20 || &bytes[..4] != b"\x7fELF" {
        return Ok(None);
    }

    let machine = match bytes[5] {
        1 => u16::from_le_bytes([bytes[18], bytes[19]]),
        2 => u16::from_be_bytes([bytes[18], bytes[19]]),
        _ => {
            return Err(ChvmError::InvalidRootfs(format!(
                "unsupported ELF endianness in {}",
                path.display()
            )))
        }
    };

    Ok(Some(machine))
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

/// Read `/proc/<pid>/cmdline` as a lossy string, or None if the pid is gone.
fn read_proc_cmdline(pid: i32) -> Option<String> {
    if pid <= 0 {
        return None;
    }
    std::fs::read(format!("/proc/{}/cmdline", pid))
        .ok()
        .map(|data| String::from_utf8_lossy(&data).into_owned())
}

/// True if `pid` is *this* VM's cloud-hypervisor, identified by its unique
/// api-socket path in the cmdline. This defeats PID reuse: a recycled PID that
/// happens to be a different VM's hypervisor won't match.
fn process_is_vm(pid: i32, api_socket: &str) -> bool {
    match read_proc_cmdline(pid) {
        Some(cmdline) => cmdline.contains("cloud-hypervisor") && cmdline.contains(api_socket),
        None => false,
    }
}

/// Kill a VM's hypervisor, but only if `pid` is still that exact VM's
/// cloud-hypervisor (identified by its api-socket), never a recycled PID.
fn kill_vm_process(pid: i32, api_socket: &str) {
    if !process_is_vm(pid, api_socket) {
        return;
    }
    let _ = nix::sys::signal::kill(
        nix::unistd::Pid::from_raw(pid),
        nix::sys::signal::Signal::SIGKILL,
    );
}

/// Read `/proc/<pid>/comm` (the executable base name) if available.
fn read_proc_comm(pid: i32) -> Option<String> {
    std::fs::read_to_string(format!("/proc/{}/comm", pid))
        .ok()
        .map(|s| s.trim().to_string())
}

/// Kill a virtiofsd child, verifying both that its cmdline still contains the
/// expected socket fragment AND that its executable is actually `virtiofsd`, so
/// an operator's `tail`/`vim` on a matching path is never killed.
fn kill_child_process(pid: i32) {
    if pid <= 0 {
        return;
    }
    if read_proc_comm(pid).as_deref() != Some("virtiofsd") {
        return;
    }
    let _ = nix::sys::signal::kill(
        nix::unistd::Pid::from_raw(pid),
        nix::sys::signal::Signal::SIGKILL,
    );
}

fn kill_virtiofsd_by_fragment(fragment: &str) {
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return;
    };

    for entry in entries.flatten() {
        let Ok(pid) = entry.file_name().to_string_lossy().parse::<i32>() else {
            continue;
        };
        // Only consider processes whose executable is virtiofsd.
        if read_proc_comm(pid).as_deref() != Some("virtiofsd") {
            continue;
        }
        let cmdline_path = entry.path().join("cmdline");
        let Ok(cmdline) = std::fs::read(&cmdline_path) else {
            continue;
        };
        let cmdline = String::from_utf8_lossy(&cmdline);
        if cmdline.contains(fragment) {
            kill_child_process(pid);
        }
    }
}

/// Remove runtime-injected `chvm.*` kernel parameters from a cmdline so that
/// repeated start/stop cycles rebuild them fresh instead of accumulating stale,
/// duplicated entries (e.g. an old `chvm.ip=` that now belongs to another VM).
fn strip_runtime_cmdline_params(cmdline: &mut String) {
    const RUNTIME_PREFIXES: [&str; 5] = [
        "chvm.ip=",
        "chvm.gw=",
        "chvm.dns=",
        "chvm.mycelium_peer=",
        "chvm.mounts=",
    ];
    let kept: Vec<&str> = cmdline
        .split_whitespace()
        .filter(|tok| !RUNTIME_PREFIXES.iter().any(|p| tok.starts_with(p)))
        .collect();
    *cmdline = kept.join(" ");
}

fn cleanup_virtiofs_state_dir(storage_dir: &Path) {
    let socket_path = storage_dir.join("virtiofsd.sock");
    let socket_fragment = socket_path.to_string_lossy();
    kill_virtiofsd_by_fragment(socket_fragment.as_ref());
    let dir_fragment = storage_dir.to_string_lossy();
    kill_virtiofsd_by_fragment(dir_fragment.as_ref());

    let merged = storage_dir.join("overlay/merged");
    if let Err(e) = nix::mount::umount2(&merged, nix::mount::MntFlags::MNT_DETACH) {
        if e != nix::errno::Errno::EINVAL && e != nix::errno::Errno::ENOENT {
            warn!(path = %merged.display(), error = %e, "Failed to unmount overlay during cleanup");
        }
    }
}

/// Poll until the VM's process is gone, up to `timeout`. Returns true if it
/// exited within the window, false if it is still alive.
async fn wait_until_gone(pid: i32, api_socket: &str, timeout: std::time::Duration) -> bool {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        if !process_is_vm(pid, api_socket) {
            return true;
        }
        if tokio::time::Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    }
}

/// Wait for a VM to exit gracefully; SIGKILL it if it doesn't within `timeout`.
async fn wait_for_exit(pid: i32, api_socket: &str, timeout: std::time::Duration) {
    if wait_until_gone(pid, api_socket, timeout).await {
        return;
    }
    kill_vm_process(pid, api_socket);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::oci::pull::PullResult;
    use crate::paths::Paths;

    #[test]
    fn strip_runtime_cmdline_params_removes_injected_and_is_idempotent() {
        let base = "console=ttyS0 root=/dev/vda rw chvm.cmd=abc";
        let mut cmdline = format!(
            "{base} chvm.ip=192.168.200.5/24 chvm.gw=192.168.200.1 chvm.dns=1.1.1.1 chvm.mounts=mount0=/data chvm.mycelium_peer=tcp://x"
        );
        strip_runtime_cmdline_params(&mut cmdline);
        assert_eq!(cmdline, base);
        // Re-stripping a clean cmdline is a no-op (no unbounded growth).
        let once = cmdline.clone();
        strip_runtime_cmdline_params(&mut cmdline);
        assert_eq!(cmdline, once);
    }

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
                    block_disk_size_bytes: None,
                    writable_quota_bytes: None,
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
            network_ownership: None,
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

    #[test]
    fn parse_bridge_ip_octet_extracts_last_octet() {
        assert_eq!(parse_bridge_ip_octet("192.168.200.42/24"), Some(42));
        assert_eq!(parse_bridge_ip_octet("10.0.0.2/24"), None);
    }

    #[test]
    fn allocate_bridge_ip_uses_existing_state_assignments() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::new(tmp.path().join(".chvm"));
        paths.ensure_dirs().unwrap();

        let config = Config::default();
        let mut manager = VmManager::new(config, paths).unwrap();

        let mut running = make_state("running-vm", "running");
        running.status = VmState::Running;
        running.network_state = Some(NetworkState {
            tap_device: Some("tap-running".into()),
            bridge: Some("chvm0".into()),
            mac_address: None,
            ip_address: Some("192.168.200.2/24".into()),
            gateway: Some("192.168.200.1".into()),
            mycelium_enabled: false,
            mycelium_peers: vec![],
            mycelium_ip: None,
            mycelium_health: None,
        });
        running.vm_config.network.mode = NetworkMode::Tap;
        running.vm_config.network.ip_address = Some("192.168.200.2/24".into());
        running.vm_config.network.gateway = Some("192.168.200.1".into());
        manager.state_store.save(&running).unwrap();

        let mut candidate = make_state("candidate-vm", "candidate");
        candidate.vm_config.network.mode = NetworkMode::Tap;

        assert_eq!(
            manager.allocate_bridge_ip(&candidate).unwrap(),
            "192.168.200.3/24"
        );
    }

    #[test]
    fn allocate_bridge_ip_reuses_existing_ip_for_same_vm_if_still_free() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::new(tmp.path().join(".chvm"));
        paths.ensure_dirs().unwrap();

        let config = Config::default();
        let manager = VmManager::new(config, paths).unwrap();

        let mut candidate = make_state("candidate-vm", "candidate");
        candidate.vm_config.network.mode = NetworkMode::Tap;
        candidate.vm_config.network.ip_address = Some("192.168.200.42/24".into());

        assert_eq!(
            manager.allocate_bridge_ip(&candidate).unwrap(),
            "192.168.200.42/24"
        );
    }

    #[test]
    fn cleanup_runtime_artifacts_removes_stale_sockets() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::new(tmp.path().join("chvm"));
        let manager = VmManager::new(Config::default(), paths).unwrap();
        let vm_dir = tmp.path().join("vm");
        std::fs::create_dir_all(&vm_dir).unwrap();
        std::fs::write(vm_dir.join("api.sock"), b"stale").unwrap();
        std::fs::write(vm_dir.join("vsock.sock"), b"stale").unwrap();

        manager.cleanup_runtime_artifacts(&vm_dir);

        assert!(!vm_dir.join("api.sock").exists());
        assert!(!vm_dir.join("vsock.sock").exists());
    }

    #[test]
    fn build_vm_config_uses_default_mycelium_peers_for_default_network_mode() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::new(tmp.path().join("chvm"));
        let manager = VmManager::new(Config::default(), paths).unwrap();
        let vm_dir = tmp.path().join("vm");
        let kernel_path = tmp.path().join("vmlinux-chvm-test");
        std::fs::create_dir_all(&vm_dir).unwrap();
        std::fs::write(&kernel_path, b"kernel").unwrap();

        let opts = CreateOptions {
            name: Some("vm".into()),
            image_ref: Some("alpine:latest".into()),
            rootfs_path: None,
            vcpu: None,
            memory_mb: None,
            storage_backend: None,
            block_disk_size_bytes: None,
            writable_quota_bytes: None,
            network_mode: None,
            port_forwards: vec![],
            env: vec![],
            cmd: vec![],
            mounts: vec![],
            kernel_path: Some(kernel_path.to_string_lossy().to_string()),
            detach: false,
            remove_after: false,
            mycelium_peers: vec![],
            credentials: None,
        };

        let vm_config = manager
            .build_vm_config("a1b2c3d4e5f6", "vm", &opts, "/rootfs".into(), None, &vm_dir)
            .unwrap();

        assert_eq!(vm_config.network.mode, NetworkMode::Mycelium);
        assert_eq!(
            vm_config.network.mycelium_peers,
            resolve_default_peers(&manager.config.default_mycelium_peers)
        );
        assert_eq!(vm_config.rootfs.rootfs_type, RootfsType::VirtioFs);
    }

    #[test]
    fn choose_storage_backend_falls_back_to_block_when_virtiofs_unavailable() {
        let backend = choose_storage_backend(None, "virtiofs", false).unwrap();
        assert_eq!(backend, "block");
    }

    #[test]
    fn choose_storage_backend_preserves_explicit_virtiofs_request() {
        let backend = choose_storage_backend(Some("virtiofs"), "virtiofs", true).unwrap();
        assert_eq!(backend, "virtiofs");
    }

    #[test]
    fn choose_storage_backend_rejects_explicit_virtiofs_request_when_unavailable() {
        let err = choose_storage_backend(Some("virtiofs"), "virtiofs", false).unwrap_err();
        assert!(err
            .to_string()
            .contains("virtiofs storage requested but unavailable"));
    }

    #[test]
    fn build_vm_config_rejects_storage_quota_for_block_backend() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::new(tmp.path().join("chvm"));
        let manager = VmManager::new(Config::default(), paths).unwrap();
        let vm_dir = tmp.path().join("vm");
        let kernel_path = tmp.path().join("vmlinux-chvm-test");
        std::fs::create_dir_all(&vm_dir).unwrap();
        std::fs::write(&kernel_path, b"kernel").unwrap();

        let opts = CreateOptions {
            name: Some("vm".into()),
            image_ref: Some("alpine:latest".into()),
            rootfs_path: None,
            vcpu: None,
            memory_mb: None,
            storage_backend: Some("block".into()),
            block_disk_size_bytes: None,
            writable_quota_bytes: Some(1024 * 1024),
            network_mode: None,
            port_forwards: vec![],
            env: vec![],
            cmd: vec![],
            mounts: vec![],
            kernel_path: Some(kernel_path.to_string_lossy().to_string()),
            detach: false,
            remove_after: false,
            mycelium_peers: vec![],
            credentials: None,
        };

        let err = manager
            .build_vm_config("a1b2c3d4e5f6", "vm", &opts, "/rootfs".into(), None, &vm_dir)
            .unwrap_err();
        assert!(err
            .to_string()
            .contains("--storage-quota can only be used with the virtiofs storage backend"));
    }

    #[test]
    fn build_vm_config_rejects_disk_size_for_virtiofs_backend() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::new(tmp.path().join("chvm"));
        let manager = VmManager::new(Config::default(), paths).unwrap();
        let vm_dir = tmp.path().join("vm");
        let kernel_path = tmp.path().join("vmlinux-chvm-test");
        std::fs::create_dir_all(&vm_dir).unwrap();
        std::fs::write(&kernel_path, b"kernel").unwrap();

        let opts = CreateOptions {
            name: Some("vm".into()),
            image_ref: Some("alpine:latest".into()),
            rootfs_path: None,
            vcpu: None,
            memory_mb: None,
            storage_backend: Some("virtiofs".into()),
            block_disk_size_bytes: Some(1024 * 1024),
            writable_quota_bytes: None,
            network_mode: None,
            port_forwards: vec![],
            env: vec![],
            cmd: vec![],
            mounts: vec![],
            kernel_path: Some(kernel_path.to_string_lossy().to_string()),
            detach: false,
            remove_after: false,
            mycelium_peers: vec![],
            credentials: None,
        };

        let err = manager
            .build_vm_config("a1b2c3d4e5f6", "vm", &opts, "/rootfs".into(), None, &vm_dir)
            .unwrap_err();
        assert!(err
            .to_string()
            .contains("--disk-size can only be used with the block storage backend"));
    }

    #[test]
    fn resolve_storage_rootfs_source_uses_local_rootfs_metadata_for_virtiofs_restart() {
        let temp = tempfile::tempdir().unwrap();
        let paths = Paths::new(temp.path().join("chvm"));
        let mut manager = VmManager::new(Config::default(), paths).unwrap();
        let rootfs = temp.path().join("rootfs");
        std::fs::create_dir_all(&rootfs).unwrap();

        let mut state = make_state("a1b2c3d4e5f6", "vm");
        state.image_ref = format!("local:{}", rootfs.display());
        state.vm_config.rootfs.rootfs_type = RootfsType::VirtioFs;
        state.vm_config.rootfs.source = temp
            .path()
            .join("vm")
            .join("storage")
            .join("virtiofsd.sock")
            .to_string_lossy()
            .to_string();

        let (resolved, inject_runtime_binaries) = manager
            .resolve_storage_rootfs_source(&state, false)
            .unwrap();

        assert_eq!(resolved, rootfs.to_string_lossy());
        assert!(inject_runtime_binaries);
    }

    fn write_elf(path: &Path, machine: u16) {
        let mut bytes = vec![0u8; 64];
        bytes[..4].copy_from_slice(b"\x7fELF");
        bytes[4] = 2;
        bytes[5] = 1;
        bytes[18..20].copy_from_slice(&machine.to_le_bytes());
        std::fs::write(path, bytes).unwrap();
    }

    #[test]
    fn validate_rootfs_accepts_x86_64_elf() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join("bin")).unwrap();
        std::fs::create_dir_all(tmp.path().join("etc")).unwrap();
        write_elf(&tmp.path().join("bin/busybox"), 62);
        std::os::unix::fs::symlink("/bin/busybox", tmp.path().join("bin/sh")).unwrap();

        assert!(validate_rootfs(tmp.path()).is_ok());
    }

    #[test]
    fn validate_rootfs_rejects_non_x86_64_elf() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join("bin")).unwrap();
        std::fs::create_dir_all(tmp.path().join("etc")).unwrap();
        write_elf(&tmp.path().join("bin/ls"), 183);

        let err = validate_rootfs(tmp.path()).unwrap_err().to_string();
        assert!(err.contains("unsupported ELF machine"));
    }

    #[test]
    fn remove_image_rejects_digest_still_referenced_by_vm() {
        let temp_dir = tempfile::tempdir().unwrap();
        let paths = crate::paths::Paths::new(temp_dir.path().join("chvm"));
        paths.ensure_dirs().unwrap();
        let config = Config::default();
        let mut manager = VmManager::new(config, paths.clone()).unwrap();

        let digest = "sha256:123456";
        let digest_key = crate::oci::registry::sanitize_digest(digest);
        let image_dir = paths.images.join(&digest_key);
        std::fs::create_dir_all(&image_dir).unwrap();
        let layer_path = image_dir.join("layer.tar.gz");
        std::fs::write(&layer_path, b"layer").unwrap();
        manager
            .image_cache
            .add(&PullResult {
                reference: "docker.io/library/ubuntu:24.04".to_string(),
                digest: digest.to_string(),
                config: None,
                layer_paths: vec![layer_path.to_string_lossy().to_string()],
                cache_dir: image_dir,
            })
            .unwrap();

        let mut state = make_state("vm123", "web");
        state.image_digest = Some(digest.to_string());
        manager.state_store.save(&state).unwrap();

        let result = manager.remove_image("ubuntu:24.04");
        assert!(matches!(result, Err(ChvmError::ImageInUse(_))));
    }
}
