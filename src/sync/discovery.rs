use anyhow::{Context, Result};
use mdns_sd::{ServiceDaemon, ServiceEvent, ServiceInfo};
use std::collections::HashMap;
use std::net::IpAddr;
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

const SERVICE_TYPE: &str = "_memo-node._tcp.local.";

#[derive(Debug, Clone)]
pub struct DiscoveredPeer {
    pub node_id: String,
    pub address: IpAddr,
    pub grpc_port: u16,
}

pub struct Discovery {
    node_id: String,
    grpc_port: u16,
    mdns: ServiceDaemon,
    peer_tx: mpsc::UnboundedSender<DiscoveredPeer>,
}

impl Discovery {
    pub fn new(
        node_id: String,
        grpc_port: u16,
    ) -> Result<(Self, mpsc::UnboundedReceiver<DiscoveredPeer>)> {
        let mdns = ServiceDaemon::new().context("Failed to create mDNS daemon")?;
        let (peer_tx, peer_rx) = mpsc::unbounded_channel();

        Ok((
            Self {
                node_id,
                grpc_port,
                mdns,
                peer_tx,
            },
            peer_rx,
        ))
    }

    pub fn start(&self) -> Result<()> {
        // Register this node as a service
        self.register_service()?;

        // Browse for other memo-node services
        self.browse_services()?;

        Ok(())
    }

    fn register_service(&self) -> Result<()> {
        let mut properties = HashMap::new();
        properties.insert("node_id".to_string(), self.node_id.clone());
        properties.insert("grpc_port".to_string(), self.grpc_port.to_string());

        let service_info = ServiceInfo::new(
            SERVICE_TYPE,
            &self.node_id,
            &format!("{}.local.", self.node_id),
            (), // Use default IP
            self.grpc_port,
            Some(properties),
        )
        .context("Failed to create service info")?;

        self.mdns
            .register(service_info)
            .context("Failed to register mDNS service")?;

        info!(
            node_id = %self.node_id,
            port = self.grpc_port,
            "Registered mDNS service"
        );

        Ok(())
    }

    fn browse_services(&self) -> Result<()> {
        let receiver = self
            .mdns
            .browse(SERVICE_TYPE)
            .context("Failed to browse mDNS services")?;

        let peer_tx = self.peer_tx.clone();
        let own_node_id = self.node_id.clone();

        // Spawn a task to handle service events
        tokio::spawn(async move {
            while let Ok(event) = receiver.recv_async().await {
                match event {
                    ServiceEvent::ServiceResolved(info) => {
                        if let Some(peer) = Self::parse_service_info(&info, &own_node_id) {
                            info!(
                                node_id = %peer.node_id,
                                address = %peer.address,
                                port = peer.grpc_port,
                                "Discovered peer"
                            );
                            if let Err(e) = peer_tx.send(peer) {
                                error!("Failed to send discovered peer: {}", e);
                            }
                        }
                    }
                    ServiceEvent::ServiceRemoved(_, fullname) => {
                        debug!("Service removed: {}", fullname);
                    }
                    ServiceEvent::SearchStarted(_) => {
                        debug!("mDNS search started");
                    }
                    ServiceEvent::SearchStopped(_) => {
                        warn!("mDNS search stopped");
                    }
                    _ => {}
                }
            }
        });

        Ok(())
    }

    fn parse_service_info(info: &ServiceInfo, own_node_id: &str) -> Option<DiscoveredPeer> {
        let properties = info.get_properties();

        let node_id = properties
            .get("node_id")
            .map(|v| v.val_str().to_string())?;

        // Don't discover ourselves
        if node_id == own_node_id {
            return None;
        }

        let grpc_port = properties
            .get("grpc_port")
            .map(|v| v.val_str())
            .and_then(|s| s.parse::<u16>().ok())?;

        let address = info.get_addresses().iter().next()?.clone();

        Some(DiscoveredPeer {
            node_id,
            address,
            grpc_port,
        })
    }

    pub fn shutdown(&self) -> Result<()> {
        self.mdns
            .shutdown()
            .context("Failed to shutdown mDNS daemon")?;
        Ok(())
    }
}

impl Drop for Discovery {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}
