use mdns_sd::{ServiceDaemon, ServiceInfo};
use std::net::IpAddr;
use tracing::{error, info};

const SERVICE_TYPE: &str = "_caboose._tcp.local.";

pub struct MdnsAdvertiser {
    daemon: ServiceDaemon,
}

impl MdnsAdvertiser {
    pub fn new(port: u16, ip: Option<IpAddr>) -> anyhow::Result<Self> {
        let daemon = ServiceDaemon::new()?;

        let hostname = hostname::get()
            .unwrap_or_else(|_| "caboose".into())
            .to_string_lossy()
            .to_string();

        let instance_name = format!("Caboose on {}", hostname);
        let host_name = format!("{}.", hostname);

        let service_info = if let Some(ip) = ip {
            ServiceInfo::new(
                SERVICE_TYPE,
                &instance_name,
                &host_name,
                ip.to_string().as_str(),
                port,
                None,
            )?
        } else {
            ServiceInfo::new(
                SERVICE_TYPE,
                &instance_name,
                &host_name,
                "",
                port,
                None,
            )?
            .enable_addr_auto()
        };

        daemon.register(service_info)?;
        info!(port, "mDNS advertising _caboose._tcp");

        Ok(Self { daemon })
    }
}

impl Drop for MdnsAdvertiser {
    fn drop(&mut self) {
        if let Err(e) = self.daemon.shutdown() {
            error!("mDNS shutdown error: {}", e);
        }
    }
}
