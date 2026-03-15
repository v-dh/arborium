//! Self-signed TLS certificate generation and HTTPS serving.
//!
//! On first run (or when certs expire), generates a local CA and server
//! certificate so arbor-httpd can serve HTTPS out of the box. Plain HTTP
//! connections on the same port are automatically redirected to HTTPS.

use {
    crate::TlsError,
    rcgen::{BasicConstraints, CertificateParams, DnType, IsCa, KeyPair, KeyUsagePurpose, SanType},
    rustls::ServerConfig,
    std::{
        io::BufReader,
        net::SocketAddr,
        path::{Path, PathBuf},
        sync::Arc,
        time::SystemTime,
    },
    time::OffsetDateTime,
    tokio::{
        io::AsyncWriteExt,
        net::{TcpListener, TcpStream},
    },
};

const LOCALHOST_DOMAIN: &str = "arbor.localhost";

/// Returns the certificate storage directory (`~/.config/arbor/certs/`).
pub fn cert_dir() -> Result<PathBuf, TlsError> {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_owned());
    let dir = PathBuf::from(home).join(".config/arbor/certs");
    std::fs::create_dir_all(&dir).map_err(TlsError::CreateCertDir)?;
    Ok(dir)
}

/// Ensure certificates exist and are fresh. Returns (ca_cert, server_cert, server_key) paths.
pub fn ensure_certs() -> Result<(PathBuf, PathBuf, PathBuf), TlsError> {
    let dir = cert_dir()?;
    let ca_cert_path = dir.join("ca.pem");
    let ca_key_path = dir.join("ca-key.pem");
    let server_cert_path = dir.join("server.pem");
    let server_key_path = dir.join("server-key.pem");

    let need_regen = !ca_cert_path.exists()
        || !server_cert_path.exists()
        || !server_key_path.exists()
        || is_expired(&server_cert_path, 30);

    if need_regen {
        eprintln!("generating TLS certificates in {}", dir.display());
        let (ca_cert_pem, ca_key_pem, server_cert_pem, server_key_pem) = generate_all()?;
        std::fs::write(&ca_cert_path, &ca_cert_pem).map_err(|source| TlsError::Io {
            context: "write ca.pem".to_owned(),
            source,
        })?;
        std::fs::write(&ca_key_path, &ca_key_pem).map_err(|source| TlsError::Io {
            context: "write ca-key.pem".to_owned(),
            source,
        })?;
        std::fs::write(&server_cert_path, &server_cert_pem).map_err(|source| TlsError::Io {
            context: "write server.pem".to_owned(),
            source,
        })?;
        std::fs::write(&server_key_path, &server_key_pem).map_err(|source| TlsError::Io {
            context: "write server-key.pem".to_owned(),
            source,
        })?;
        eprintln!("TLS certificates written to {}", dir.display());
    }

    Ok((ca_cert_path, server_cert_path, server_key_path))
}

fn is_expired(path: &Path, days: u64) -> bool {
    let Ok(meta) = std::fs::metadata(path) else {
        return true;
    };
    let Ok(modified) = meta.modified() else {
        return true;
    };
    let age = SystemTime::now()
        .duration_since(modified)
        .unwrap_or_default();
    age > std::time::Duration::from_secs(days * 24 * 60 * 60)
}

fn required_dns_san_names() -> Vec<String> {
    let mut names = vec![
        LOCALHOST_DOMAIN.to_string(),
        format!("*.{LOCALHOST_DOMAIN}"),
        "localhost".to_string(),
    ];

    if let Some(host) = hostname::get().ok().and_then(|h| h.into_string().ok()) {
        let normalized = host.trim().trim_end_matches('.').to_ascii_lowercase();
        if !normalized.is_empty() && normalized != "localhost" && normalized != LOCALHOST_DOMAIN {
            names.push(normalized.clone());
            if !normalized.contains('.') {
                names.push(format!("{normalized}.local"));
            }
        }
    }

    names.sort_unstable();
    names.dedup();
    names
}

fn generate_all() -> Result<(String, String, String, String), TlsError> {
    let now = OffsetDateTime::now_utc();

    // CA
    let ca_key = KeyPair::generate().map_err(|source| TlsError::CertGeneration {
        context: "generate CA key".to_owned(),
        reason: source.to_string(),
    })?;
    let mut ca_params = CertificateParams::new(Vec::<String>::new()).map_err(|source| {
        TlsError::CertGeneration {
            context: "CA params".to_owned(),
            reason: source.to_string(),
        }
    })?;
    ca_params
        .distinguished_name
        .push(DnType::CommonName, "Arbor Local CA");
    ca_params
        .distinguished_name
        .push(DnType::OrganizationName, "Arbor");
    ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    ca_params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
    ca_params.not_before = now;
    ca_params.not_after = now + time::Duration::days(365 * 10);
    let ca_cert =
        ca_params
            .self_signed(&ca_key)
            .map_err(|source| TlsError::CertGeneration {
                context: "self-sign CA".to_owned(),
                reason: source.to_string(),
            })?;

    // Server cert signed by CA
    let server_key = KeyPair::generate().map_err(|source| TlsError::CertGeneration {
        context: "generate server key".to_owned(),
        reason: source.to_string(),
    })?;
    let mut server_params = CertificateParams::new(vec![LOCALHOST_DOMAIN.to_string()]).map_err(
        |source| TlsError::CertGeneration {
            context: "server params".to_owned(),
            reason: source.to_string(),
        },
    )?;
    server_params
        .distinguished_name
        .push(DnType::CommonName, LOCALHOST_DOMAIN);

    let mut subject_alt_names: Vec<SanType> = required_dns_san_names()
        .into_iter()
        .filter_map(|name| name.as_str().try_into().ok().map(SanType::DnsName))
        .collect();
    subject_alt_names.push(SanType::IpAddress(std::net::IpAddr::V4(
        std::net::Ipv4Addr::LOCALHOST,
    )));
    subject_alt_names.push(SanType::IpAddress(std::net::IpAddr::V6(
        std::net::Ipv6Addr::LOCALHOST,
    )));
    server_params.subject_alt_names = subject_alt_names;
    server_params.not_before = now;
    server_params.not_after = now + time::Duration::days(365);
    let server_cert = server_params.signed_by(&server_key, &ca_cert, &ca_key).map_err(
        |source| TlsError::CertGeneration {
            context: "sign server cert".to_owned(),
            reason: source.to_string(),
        },
    )?;

    Ok((
        ca_cert.pem(),
        ca_key.serialize_pem(),
        server_cert.pem(),
        server_key.serialize_pem(),
    ))
}

/// Load cert + key PEM files into a rustls ServerConfig.
pub fn load_rustls_config(cert_path: &Path, key_path: &Path) -> Result<ServerConfig, TlsError> {
    let _ = rustls::crypto::ring::default_provider().install_default();

    let cert_file = std::fs::File::open(cert_path).map_err(|source| TlsError::Io {
        context: "open server cert".to_owned(),
        source,
    })?;
    let key_file = std::fs::File::open(key_path).map_err(|source| TlsError::Io {
        context: "open server key".to_owned(),
        source,
    })?;

    let certs: Vec<_> = rustls_pemfile::certs(&mut BufReader::new(cert_file))
        .collect::<Result<Vec<_>, _>>()
        .map_err(TlsError::ParseCerts)?;

    let key = rustls_pemfile::private_key(&mut BufReader::new(key_file))
        .map_err(TlsError::ParsePrivateKey)?
        .ok_or(TlsError::NoPrivateKey)?;

    let mut config = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .map_err(|e| TlsError::BuildServerConfig(e.to_string()))?;
    config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
    Ok(config)
}

/// Serve an axum app over TLS on a single port. Plain HTTP connections get a
/// redirect to HTTPS.
pub async fn serve_tls(
    listener: TcpListener,
    tls_config: Arc<ServerConfig>,
    app: axum::Router,
    port: u16,
    bind_host: &str,
) -> Result<(), TlsError> {
    let acceptor = tokio_rustls::TlsAcceptor::from(tls_config);
    let localhost_mode = is_localhost_name(bind_host);
    let bind_host = bind_host.to_string();
    let mut make_service = app.into_make_service_with_connect_info::<SocketAddr>();

    loop {
        let (stream, addr) = match listener.accept().await {
            Ok(conn) => conn,
            Err(e) => {
                if is_accept_error(&e) {
                    continue;
                }
                eprintln!("accept error: {e}");
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                continue;
            },
        };

        // Peek first byte: 0x16 = TLS ClientHello
        let mut peek_buf = [0u8; 1];
        match stream.peek(&mut peek_buf).await {
            Ok(1) if peek_buf[0] == 0x16 => {
                let acceptor = acceptor.clone();
                let service = <_ as tower::Service<SocketAddr>>::call(&mut make_service, addr)
                    .await
                    .unwrap_or_else(|e| match e {});
                tokio::spawn(async move {
                    let tls_stream = match acceptor.accept(stream).await {
                        Ok(s) => s,
                        Err(_) => return,
                    };
                    let io = hyper_util::rt::TokioIo::new(tls_stream);
                    let hyper_service = hyper_util::service::TowerToHyperService::new(service);
                    let _ = hyper_util::server::conn::auto::Builder::new(
                        hyper_util::rt::TokioExecutor::new(),
                    )
                    .serve_connection_with_upgrades(io, hyper_service)
                    .await;
                });
            },
            Ok(_) => {
                // Plain HTTP — send redirect
                let redirect_host = bind_host.clone();
                tokio::spawn(async move {
                    let _ = send_http_redirect(stream, port, &redirect_host, localhost_mode).await;
                });
            },
            Err(_) => {},
        }
    }
}

fn is_localhost_name(name: &str) -> bool {
    matches!(name, "localhost" | "127.0.0.1" | "::1") || name.ends_with(".localhost")
}

fn is_accept_error(e: &std::io::Error) -> bool {
    matches!(
        e.kind(),
        std::io::ErrorKind::ConnectionRefused
            | std::io::ErrorKind::ConnectionAborted
            | std::io::ErrorKind::ConnectionReset
    )
}

async fn send_http_redirect(
    mut stream: TcpStream,
    https_port: u16,
    bind_host: &str,
    localhost_mode: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut buf = vec![0u8; 4096];
    let n =
        tokio::time::timeout(std::time::Duration::from_secs(5), stream.peek(&mut buf)).await??;

    let raw = String::from_utf8_lossy(&buf[..n]);

    let path = raw
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .unwrap_or("/");

    let host = if localhost_mode {
        "localhost".to_string()
    } else {
        raw.lines()
            .find_map(|line| {
                if line.get(..5)?.eq_ignore_ascii_case("host:") {
                    Some(line[5..].trim().to_string())
                } else {
                    None
                }
            })
            .unwrap_or_else(|| bind_host.to_string())
    };

    // Strip port from host if present
    let host_no_port = host.split(':').next().unwrap_or(&host);

    let location = format!("https://{host_no_port}:{https_port}{path}");
    let body =
        format!("<html><body>Redirecting to <a href=\"{location}\">{location}</a></body></html>");
    let response = format!(
        "HTTP/1.1 301 Moved Permanently\r\n\
         Location: {location}\r\n\
         Content-Type: text/html\r\n\
         Content-Length: {}\r\n\
         Connection: close\r\n\
         \r\n\
         {body}",
        body.len()
    );

    stream.write_all(response.as_bytes()).await?;
    stream.shutdown().await?;
    Ok(())
}
