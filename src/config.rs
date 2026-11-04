//! Cluster configuration and environment loading.
//!
//! Traces to: AG-SPEC-2026-V2 Section 2.3, Section 7, [NFR-SEC-001], [NFR-PERF-001]

use std::net::SocketAddr;
use std::path::PathBuf;
use thiserror::Error;

use crate::MAX_FRAME_SIZE_BYTES;

/// Configuration loading errors.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ConfigError {
    #[error("Invalid socket address format in {var}: {val}")]
    InvalidSocketAddr {
        var: &'static str,
        val: String,
    },
    #[error("Invalid integer value in {var}: {val}")]
    InvalidInteger {
        var: &'static str,
        val: String,
    },
}

/// Orchestrator host control plane configuration settings.
#[derive(Debug, Clone)]
pub struct ClusterConfig {
    /// Socket address for gRPC service binding (default: 127.0.0.1:50051).
    pub listen_addr: SocketAddr,

    /// Optional path to X.509 CA certificate for client mTLS validation.
    pub tls_ca_cert_path: Option<PathBuf>,

    /// Optional path to server certificate PEM.
    pub tls_server_cert_path: Option<PathBuf>,

    /// Optional path to server private key PEM.
    pub tls_server_key_path: Option<PathBuf>,

    /// Declarative heartbeat synchronization interval in seconds (default: 2s).
    pub heartbeat_interval_seconds: u32,

    /// Node lease timeout in seconds before worker is marked offline (default: 10s).
    pub lease_timeout_seconds: u32,

    /// Maximum allowable gRPC message frame payload in bytes (default: 16 MB).
    pub max_frame_size_bytes: usize,
}

impl Default for ClusterConfig {
    fn default() -> Self {
        Self {
            listen_addr: "127.0.0.1:50051"
                .parse()
                .expect("Default socket address must be valid"),
            tls_ca_cert_path: None,
            tls_server_cert_path: None,
            tls_server_key_path: None,
            heartbeat_interval_seconds: 2,
            lease_timeout_seconds: 10,
            max_frame_size_bytes: MAX_FRAME_SIZE_BYTES,
        }
    }
}

impl ClusterConfig {
    /// Loads configuration from environment variables with fallback to defaults.
    ///
    /// # Environment Variables:
    /// * `ANDROGRID_LISTEN_ADDR` - Socket address to bind (e.g. "0.0.0.0:50051").
    /// * `ANDROGRID_CA_CERT_PATH` - Path to CA certificate PEM for mTLS.
    /// * `ANDROGRID_SERVER_CERT_PATH` - Path to server certificate PEM.
    /// * `ANDROGRID_SERVER_KEY_PATH` - Path to server private key PEM.
    /// * `ANDROGRID_HEARTBEAT_SECS` - Heartbeat interval in seconds.
    /// * `ANDROGRID_LEASE_TIMEOUT_SECS` - Lease timeout in seconds.
    pub fn from_env() -> Result<Self, ConfigError> {
        let mut config = Self::default();

        if let Ok(val) = std::env::var("ANDROGRID_LISTEN_ADDR") {
            config.listen_addr = val.parse().map_err(|_| ConfigError::InvalidSocketAddr {
                var: "ANDROGRID_LISTEN_ADDR",
                val,
            })?;
        }

        if let Ok(val) = std::env::var("ANDROGRID_CA_CERT_PATH") {
            config.tls_ca_cert_path = Some(PathBuf::from(val));
        }

        if let Ok(val) = std::env::var("ANDROGRID_SERVER_CERT_PATH") {
            config.tls_server_cert_path = Some(PathBuf::from(val));
        }

        if let Ok(val) = std::env::var("ANDROGRID_SERVER_KEY_PATH") {
            config.tls_server_key_path = Some(PathBuf::from(val));
        }

        if let Ok(val) = std::env::var("ANDROGRID_HEARTBEAT_SECS") {
            config.heartbeat_interval_seconds =
                val.parse().map_err(|_| ConfigError::InvalidInteger {
                    var: "ANDROGRID_HEARTBEAT_SECS",
                    val,
                })?;
        }

        if let Ok(val) = std::env::var("ANDROGRID_LEASE_TIMEOUT_SECS") {
            config.lease_timeout_seconds = val.parse().map_err(|_| ConfigError::InvalidInteger {
                var: "ANDROGRID_LEASE_TIMEOUT_SECS",
                val,
            })?;
        }

        Ok(config)
    }

    /// Checks if full mutual TLS credentials are provided in configuration.
    pub fn is_mtls_configured(&self) -> bool {
        self.tls_ca_cert_path.is_some()
            && self.tls_server_cert_path.is_some()
            && self.tls_server_key_path.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config_adheres_to_specification_bounds() {
        let config = ClusterConfig::default();
        assert_eq!(config.listen_addr.port(), 50051);
        assert_eq!(config.heartbeat_interval_seconds, 2);
        assert_eq!(config.lease_timeout_seconds, 10);
        assert_eq!(config.max_frame_size_bytes, 16 * 1024 * 1024);
        assert!(!config.is_mtls_configured());
    }

    #[test]
    fn test_is_mtls_configured_requires_all_three_components() {
        let mut config = ClusterConfig::default();
        assert!(!config.is_mtls_configured());

        config.tls_ca_cert_path = Some(PathBuf::from("/etc/ca.crt"));
        assert!(!config.is_mtls_configured());

        config.tls_server_cert_path = Some(PathBuf::from("/etc/server.crt"));
        assert!(!config.is_mtls_configured());

        config.tls_server_key_path = Some(PathBuf::from("/etc/server.key"));
        assert!(config.is_mtls_configured());
    }
}
