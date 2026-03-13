//! RAII guard for container cleanup.
//!
//! Ensures containers are stopped and removed even on panic or early return.
//! Uses `Drop` with a spawned blocking task for best-effort cleanup.

use bollard::Docker;
use bollard::container::{RemoveContainerOptions, StopContainerOptions};

/// RAII guard that stops and removes a Docker container on drop.
///
/// When `should_remove` is true (the default for `ReusePolicy::Fresh`),
/// dropping this guard spawns a blocking tokio task to stop and remove
/// the container. Errors during cleanup are logged but not propagated.
pub struct ContainerGuard {
    docker: Docker,
    container_id: String,
    should_remove: bool,
}

impl ContainerGuard {
    /// Create a new cleanup guard.
    pub fn new(docker: Docker, container_id: String, should_remove: bool) -> Self {
        Self {
            docker,
            container_id,
            should_remove,
        }
    }
}

impl Drop for ContainerGuard {
    fn drop(&mut self) {
        if !self.should_remove {
            tracing::debug!(
                container = %self.container_id,
                "skipping container cleanup (reuse policy)"
            );
            return;
        }

        let docker = self.docker.clone();
        let id = self.container_id.clone();

        // Best-effort: spawn a task to clean up. If the runtime is shutting
        // down this may not complete, but container labels allow external
        // garbage collection.
        tokio::task::spawn(async move {
            tracing::debug!(container = %id, "stopping container");
            if let Err(e) = docker
                .stop_container(&id, Some(StopContainerOptions { t: 10 }))
                .await
            {
                tracing::warn!(container = %id, error = %e, "failed to stop container");
            }

            tracing::debug!(container = %id, "removing container");
            if let Err(e) = docker
                .remove_container(
                    &id,
                    Some(RemoveContainerOptions {
                        force: true,
                        ..Default::default()
                    }),
                )
                .await
            {
                tracing::warn!(container = %id, error = %e, "failed to remove container");
            }
        });
    }
}
