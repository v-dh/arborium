use mdns_sd::{ServiceDaemon, ServiceInfo};

const SERVICE_TYPE: &str = "_arbor._tcp.local.";

#[derive(Debug, thiserror::Error)]
pub enum MdnsError {
    #[error("failed to create mDNS daemon: {0}")]
    DaemonInit(mdns_sd::Error),
    #[error("failed to register service: {0}")]
    Registration(mdns_sd::Error),
}

/// Holds the mDNS daemon alive. The service is unregistered on drop.
pub struct MdnsRegistration {
    _daemon: ServiceDaemon,
}

/// Register this arbor-httpd instance on the local network via mDNS.
pub fn register_service(
    port: u16,
    tls: bool,
    has_auth: bool,
) -> Result<MdnsRegistration, MdnsError> {
    let daemon = ServiceDaemon::new().map_err(MdnsError::DaemonInit)?;

    let instance_name = hostname::get()
        .ok()
        .and_then(|h: std::ffi::OsString| h.into_string().ok())
        .unwrap_or_else(|| "arbor-httpd".to_owned());

    let properties = [
        (
            "tls",
            if tls {
                "true"
            } else {
                "false"
            },
        ),
        (
            "auth",
            if has_auth {
                "true"
            } else {
                "false"
            },
        ),
        ("version", env!("CARGO_PKG_VERSION")),
    ];

    let service = ServiceInfo::new(
        SERVICE_TYPE,
        &instance_name,
        &format!("{instance_name}.local."),
        "",
        port,
        &properties[..],
    )
    .map_err(MdnsError::Registration)?;

    daemon.register(service).map_err(MdnsError::Registration)?;

    Ok(MdnsRegistration { _daemon: daemon })
}
