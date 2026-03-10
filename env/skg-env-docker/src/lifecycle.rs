//! Container lifecycle management: pull, create, start, discover, teardown.
//!
//! Each function takes `&Docker` and the necessary config, returning typed
//! results. The caller composes these into the full lifecycle in
//! `DockerEnvironment::run`.

use bollard::Docker;
use bollard::container::{
    Config, CreateContainerOptions, InspectContainerOptions, RemoveContainerOptions,
    StartContainerOptions, StopContainerOptions,
};
use bollard::image::CreateImageOptions;
use bollard::models::{HostConfig, PortBinding, PortMap};
use futures_util::StreamExt;
use layer0::error::EnvError;
use std::collections::HashMap;
use tracing;

use crate::config::PullPolicy;

/// Ensure a container image is present locally, pulling if needed.
pub async fn ensure_image(
    docker: &Docker,
    image: &str,
    pull_policy: &PullPolicy,
) -> Result<(), EnvError> {
    let should_pull = match pull_policy {
        PullPolicy::Always => true,
        PullPolicy::Never => false,
        PullPolicy::IfMissing => docker.inspect_image(image).await.is_err(),
    };

    if !should_pull {
        // For Never policy, verify the image exists.
        if matches!(pull_policy, PullPolicy::Never) {
            docker
                .inspect_image(image)
                .await
                .map_err(|e| EnvError::ProvisionFailed(format!("image not found locally: {e}")))?;
        }
        return Ok(());
    }

    tracing::info!(image = %image, "pulling container image");

    let opts = CreateImageOptions {
        from_image: image,
        ..Default::default()
    };

    let mut stream = docker.create_image(Some(opts), None, None);
    while let Some(result) = stream.next().await {
        match result {
            Ok(info) => {
                if let Some(status) = &info.status {
                    tracing::debug!(image = %image, status = %status, "pull progress");
                }
            }
            Err(e) => {
                return Err(EnvError::ProvisionFailed(format!(
                    "image pull failed for {image}: {e}"
                )));
            }
        }
    }

    Ok(())
}

/// Container creation parameters collected by the caller.
pub struct CreateContainerParams<'a> {
    /// Image to run.
    pub image: &'a str,
    /// Environment variables (credential-injected + system).
    pub env_vars: Vec<(String, String)>,
    /// Container port the runner listens on (gRPC).
    pub container_port: u16,
    /// Hardened host config (from `security::hardened_host_config`).
    pub host_config: HostConfig,
    /// Label: operator ID.
    pub operator_label: &'a str,
    /// Label: session key.
    pub session_label: &'a str,
    /// Host callback URL the container uses to reach the host.
    pub host_callback_url: &'a str,
}

/// Create a container with security hardening, port mapping, and labels.
/// Returns the container ID.
pub async fn create_container(
    docker: &Docker,
    params: CreateContainerParams<'_>,
) -> Result<String, EnvError> {
    // Environment variables: user creds + system vars
    let mut env: Vec<String> = params
        .env_vars
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect();
    env.push(format!("SKG_CALLBACK_URL={}", params.host_callback_url));

    // Port mapping: container_port → ephemeral host port
    let container_port_key = format!("{}/tcp", params.container_port);

    let mut port_bindings: PortMap = HashMap::new();
    port_bindings.insert(
        container_port_key.clone(),
        Some(vec![PortBinding {
            host_ip: Some("0.0.0.0".to_string()),
            host_port: Some("0".to_string()), // ephemeral
        }]),
    );

    // Merge port bindings + extra_hosts into the host config
    let mut host_config = params.host_config;
    host_config.port_bindings = Some(port_bindings);
    // Allow container to reach the host via host.docker.internal
    host_config.extra_hosts = Some(vec!["host.docker.internal:host-gateway".to_string()]);

    // Exposed ports declaration
    let mut exposed_ports: HashMap<String, HashMap<(), ()>> = HashMap::new();
    exposed_ports.insert(container_port_key, HashMap::new());

    // Labels for identification and external garbage collection
    let mut labels: HashMap<String, String> = HashMap::new();
    labels.insert(
        "skg.operator".to_string(),
        params.operator_label.to_string(),
    );
    labels.insert("skg.session".to_string(), params.session_label.to_string());
    labels.insert("skg.managed-by".to_string(), "skg-env-docker".to_string());

    let config = Config {
        image: Some(params.image.to_string()),
        env: Some(env),
        exposed_ports: Some(exposed_ports),
        host_config: Some(host_config),
        labels: Some(labels),
        ..Default::default()
    };

    let response = docker
        .create_container(None::<CreateContainerOptions<String>>, config)
        .await
        .map_err(|e| EnvError::ProvisionFailed(format!("container create failed: {e}")))?;

    tracing::info!(container = %response.id, image = %params.image, "container created");
    Ok(response.id)
}

/// Start a container and wait briefly for it to be running.
pub async fn start_container(docker: &Docker, container_id: &str) -> Result<(), EnvError> {
    docker
        .start_container(container_id, None::<StartContainerOptions<String>>)
        .await
        .map_err(|e| EnvError::ProvisionFailed(format!("container start failed: {e}")))?;

    tracing::info!(container = %container_id, "container started");
    Ok(())
}

/// Inspect the container to find the host port mapped to `container_port`.
/// Returns the `host:port` endpoint string.
pub async fn discover_grpc_endpoint(
    docker: &Docker,
    container_id: &str,
    container_port: u16,
) -> Result<String, EnvError> {
    let info = docker
        .inspect_container(container_id, None::<InspectContainerOptions>)
        .await
        .map_err(|e| EnvError::ProvisionFailed(format!("container inspect failed: {e}")))?;

    let port_key = format!("{container_port}/tcp");

    let host_port = info
        .network_settings
        .as_ref()
        .and_then(|ns| ns.ports.as_ref())
        .and_then(|ports| ports.get(&port_key))
        .and_then(|bindings| bindings.as_ref())
        .and_then(|bindings| bindings.first())
        .and_then(|binding| binding.host_port.as_ref())
        .ok_or_else(|| {
            EnvError::ProvisionFailed(format!(
                "no host port mapping found for container port {container_port}"
            ))
        })?;

    let endpoint = format!("http://127.0.0.1:{host_port}");
    tracing::info!(container = %container_id, endpoint = %endpoint, "discovered gRPC endpoint");
    Ok(endpoint)
}

#[allow(dead_code)] // Public API for explicit teardown; not called internally yet.
/// Stop and remove a container. Used both by the cleanup guard and explicit teardown.
pub async fn stop_and_remove(docker: &Docker, container_id: &str) -> Result<(), EnvError> {
    docker
        .stop_container(container_id, Some(StopContainerOptions { t: 10 }))
        .await
        .map_err(|e| EnvError::ProvisionFailed(format!("container stop failed: {e}")))?;

    docker
        .remove_container(
            container_id,
            Some(RemoveContainerOptions {
                force: true,
                ..Default::default()
            }),
        )
        .await
        .map_err(|e| EnvError::ProvisionFailed(format!("container remove failed: {e}")))?;

    tracing::info!(container = %container_id, "container stopped and removed");
    Ok(())
}
