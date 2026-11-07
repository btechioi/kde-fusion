//! mDNS discovery of other KDE Fusion peers.
//!
//! The service type is `_kdefusion._tcp.local.`.  Discovery is only a
//! convenience: peers can also be added by address in the GUI.

use anyhow::{Context, Result};
use mdns_sd::{ServiceDaemon, ServiceEvent, ServiceInfo};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::time::Duration;

use crate::util::{hostname, local_ipv4, now_unix, sanitize_service_name};

pub const SERVICE_TYPE: &str = "_kdefusion._tcp.local.";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscoveredPeer {
    pub device_id: String,
    pub name: String,
    pub fullname: String,
    pub host: String,
    pub port: u16,
    pub addresses: Vec<String>,
    pub version: String,
    pub last_seen: u64,
}

#[derive(Clone, Default)]
pub struct DiscoveryRegistry {
    peers: Arc<RwLock<HashMap<String, DiscoveredPeer>>>,
}

impl DiscoveryRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn upsert(&self, peer: DiscoveredPeer) {
        if let Ok(mut peers) = self.peers.write() {
            peers.insert(peer.device_id.clone(), peer);
        }
    }

    pub fn get(&self, device_id: &str) -> Option<DiscoveredPeer> {
        self.peers.read().ok()?.get(device_id).cloned()
    }

    pub fn list(&self) -> Vec<DiscoveredPeer> {
        let mut peers: Vec<_> = self
            .peers
            .read()
            .map(|p| p.values().cloned().collect())
            .unwrap_or_default();
        peers.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        peers
    }

    pub fn remove_fullname(&self, fullname: &str) {
        if let Ok(mut peers) = self.peers.write() {
            peers.retain(|_, peer| peer.fullname != fullname);
        }
    }

    pub fn len(&self) -> usize {
        self.peers.read().map(|p| p.len()).unwrap_or(0)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

pub struct DiscoveryHandle {
    daemon: ServiceDaemon,
    pub fullname: String,
}

impl DiscoveryHandle {
    pub fn shutdown(&self) {
        let _ = self.daemon.unregister(&self.fullname);
        let _ = self.daemon.shutdown();
    }
}

impl Drop for DiscoveryHandle {
    fn drop(&mut self) {
        let _ = self.daemon.unregister(&self.fullname);
        let _ = self.daemon.shutdown();
    }
}

/// Register this daemon and start browsing.  Failure is non-fatal to the rest
/// of the app: the caller logs the error and peers can still be added manually.
pub fn start(
    device_id: &str,
    device_name: &str,
    port: u16,
    registry: DiscoveryRegistry,
) -> Result<DiscoveryHandle> {
    let ip = local_ipv4().context("no LAN IPv4 address available for mDNS")?;
    let host = dns_hostname();
    let instance = sanitize_service_name(device_name);
    let properties = [
        ("device_id", device_id),
        ("name", device_name),
        ("version", crate::VERSION),
        ("proto", "1"),
    ];
    let info = ServiceInfo::new(
        SERVICE_TYPE,
        &instance,
        &host,
        ip.to_string(),
        port,
        &properties[..],
    )
    .context("create mDNS service record")?;
    let fullname = info.get_fullname().to_string();
    let daemon = ServiceDaemon::new().context("start mDNS daemon")?;
    daemon.register(info).context("register mDNS service")?;
    let receiver = daemon
        .browse(SERVICE_TYPE)
        .context("browse for KDE Fusion peers")?;

    let own_id = device_id.to_string();
    tokio::spawn(async move {
        loop {
            match receiver.recv_async().await {
                Ok(event) => match event {
                    ServiceEvent::ServiceResolved(resolved) => {
                        let Some(remote_id) = resolved.get_property_val_str("device_id") else {
                            continue;
                        };
                        if remote_id == own_id {
                            continue;
                        }
                        let mut addresses: Vec<String> = resolved
                            .get_addresses_v4()
                            .into_iter()
                            .map(|ip| ip.to_string())
                            .collect();
                        addresses.sort();
                        let name = resolved
                            .get_property_val_str("name")
                            .unwrap_or_else(|| resolved.get_fullname())
                            .to_string();
                        let version = resolved
                            .get_property_val_str("version")
                            .unwrap_or_default()
                            .to_string();
                        tracing::info!(
                            "discovered {} at {:?}:{}",
                            name,
                            addresses,
                            resolved.get_port()
                        );
                        registry.upsert(DiscoveredPeer {
                            device_id: remote_id.to_string(),
                            name,
                            fullname: resolved.get_fullname().to_string(),
                            host: resolved.get_hostname().to_string(),
                            port: resolved.get_port(),
                            addresses,
                            version,
                            last_seen: now_unix(),
                        });
                    }
                    ServiceEvent::ServiceRemoved(_ty, fullname) => {
                        tracing::info!("peer disappeared: {fullname}");
                        registry.remove_fullname(&fullname);
                    }
                    ServiceEvent::ServiceFound(_ty, _fullname) => {}
                    ServiceEvent::SearchStarted(_) => {}
                    ServiceEvent::SearchStopped(_) => {
                        tracing::warn!("mDNS browse stopped");
                        break;
                    }
                    _ => {}
                },
                Err(err) => {
                    tracing::debug!("mDNS receiver ended: {err}");
                    break;
                }
            }
        }
    });

    // Give the first announcements a moment to be sent before returning.  The
    // daemon is already running; this is not required for correctness.
    std::thread::sleep(Duration::from_millis(50));
    Ok(DiscoveryHandle { daemon, fullname })
}

fn dns_hostname() -> String {
    let raw = hostname().replace('.', "-");
    let cleaned: String = raw
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    format!("{}.local.", cleaned.trim_matches('-'))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn peer(id: &str) -> DiscoveredPeer {
        DiscoveredPeer {
            device_id: id.into(),
            name: format!("PC {id}"),
            fullname: format!("PC {id}._kdefusion._tcp.local."),
            host: "pc.local.".into(),
            port: 47811,
            addresses: vec!["192.168.1.2".into()],
            version: "0.1.0".into(),
            last_seen: 1,
        }
    }

    #[test]
    fn registry_upsert_and_remove() {
        let registry = DiscoveryRegistry::new();
        registry.upsert(peer("a"));
        registry.upsert(peer("b"));
        assert_eq!(registry.len(), 2);
        assert_eq!(registry.get("a").unwrap().name, "PC a");
        registry.remove_fullname("PC a._kdefusion._tcp.local.");
        assert_eq!(registry.len(), 1);
        assert!(registry.get("a").is_none());
    }

    #[test]
    fn dns_hostname_is_valid() {
        let host = dns_hostname();
        assert!(host.ends_with(".local."));
        assert!(!host.contains(' '));
        assert!(!host.contains(".."));
    }
}
