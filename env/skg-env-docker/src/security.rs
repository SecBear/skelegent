//! Hardened container configuration for defense-in-depth isolation.
//!
//! Applies: no-new-privileges, read-only rootfs, dropped capabilities,
//! writable tmpfs, optional GPU passthrough, and resource limits.

use bollard::models::{DeviceRequest, HostConfig};
use layer0::environment::ResourceLimits;
use std::collections::HashMap;

/// Build a hardened `HostConfig` from an `EnvironmentSpec`'s resource limits.
///
/// Security posture:
/// - `no-new-privileges`: prevents SUID escalation
/// - Read-only rootfs: immutable filesystem except explicit tmpfs mounts
/// - All capabilities dropped: minimal kernel surface
/// - `/tmp` mounted as `rw,noexec,nosuid,size=64m`: operators get scratch space
pub fn hardened_host_config(resources: Option<&ResourceLimits>) -> HostConfig {
    let security_opt = vec!["no-new-privileges".to_string()];

    let mut tmpfs: HashMap<String, String> = HashMap::new();
    tmpfs.insert("/tmp".to_string(), "rw,noexec,nosuid,size=64m".to_string());

    let mut host_config = HostConfig {
        security_opt: Some(security_opt),
        readonly_rootfs: Some(true),
        cap_drop: Some(vec!["ALL".to_string()]),
        tmpfs: Some(tmpfs),
        ..Default::default()
    };

    if let Some(limits) = resources {
        apply_resource_limits(&mut host_config, limits);
    }

    host_config
}

/// Map `ResourceLimits` to bollard's `HostConfig` fields.
fn apply_resource_limits(host_config: &mut HostConfig, limits: &ResourceLimits) {
    // CPU: "1.0" → 1_000_000_000 NanoCPUs, "500m" → 500_000_000
    if let Some(ref cpu) = limits.cpu {
        if let Some(nano) = parse_cpu_to_nanos(cpu) {
            host_config.nano_cpus = Some(nano);
        }
    }

    // Memory: "2Gi" → bytes, "512Mi" → bytes
    if let Some(ref mem) = limits.memory {
        if let Some(bytes) = parse_memory_to_bytes(mem) {
            host_config.memory = Some(bytes);
        }
    }

    // GPU: request nvidia runtime device
    if let Some(ref gpu) = limits.gpu {
        let count = gpu.parse::<i64>().unwrap_or(1);
        host_config.device_requests = Some(vec![DeviceRequest {
            driver: Some("nvidia".to_string()),
            count: Some(count),
            capabilities: Some(vec![vec!["gpu".to_string()]]),
            ..Default::default()
        }]);
    }
}

/// Parse CPU string to nanoseconds. Supports "1.0" (cores) and "500m" (millicores).
fn parse_cpu_to_nanos(cpu: &str) -> Option<i64> {
    if let Some(milli) = cpu.strip_suffix('m') {
        let m: f64 = milli.parse().ok()?;
        Some((m * 1_000_000.0) as i64)
    } else {
        let cores: f64 = cpu.parse().ok()?;
        Some((cores * 1_000_000_000.0) as i64)
    }
}

/// Parse memory string to bytes. Supports "Gi", "Mi", "Ki" suffixes and plain bytes.
fn parse_memory_to_bytes(mem: &str) -> Option<i64> {
    if let Some(val) = mem.strip_suffix("Gi") {
        let g: f64 = val.parse().ok()?;
        Some((g * 1024.0 * 1024.0 * 1024.0) as i64)
    } else if let Some(val) = mem.strip_suffix("Mi") {
        let m: f64 = val.parse().ok()?;
        Some((m * 1024.0 * 1024.0) as i64)
    } else if let Some(val) = mem.strip_suffix("Ki") {
        let k: f64 = val.parse().ok()?;
        Some((k * 1024.0) as i64)
    } else {
        mem.parse::<i64>().ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_cpu_cores() {
        assert_eq!(parse_cpu_to_nanos("1.0"), Some(1_000_000_000));
        assert_eq!(parse_cpu_to_nanos("0.5"), Some(500_000_000));
    }

    #[test]
    fn parse_cpu_millicores() {
        assert_eq!(parse_cpu_to_nanos("500m"), Some(500_000_000));
        assert_eq!(parse_cpu_to_nanos("250m"), Some(250_000_000));
    }

    #[test]
    fn parse_memory_gi() {
        assert_eq!(parse_memory_to_bytes("2Gi"), Some(2 * 1024 * 1024 * 1024));
    }

    #[test]
    fn parse_memory_mi() {
        assert_eq!(parse_memory_to_bytes("512Mi"), Some(512 * 1024 * 1024));
    }

    #[test]
    fn hardened_defaults() {
        let hc = hardened_host_config(None);
        assert_eq!(hc.readonly_rootfs, Some(true));
        assert_eq!(hc.cap_drop.as_deref(), Some(&["ALL".to_string()][..]));
        assert!(
            hc.security_opt
                .as_ref()
                .unwrap()
                .contains(&"no-new-privileges".to_string())
        );
        assert!(hc.tmpfs.as_ref().unwrap().contains_key("/tmp"));
    }

    #[test]
    fn gpu_device_request() {
        let mut limits = ResourceLimits::default();
        limits.gpu = Some("2".to_string());
        let hc = hardened_host_config(Some(&limits));
        let devs = hc.device_requests.unwrap();
        assert_eq!(devs.len(), 1);
        assert_eq!(devs[0].count, Some(2));
    }
}
