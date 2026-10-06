//! The HTTP client of the downloads, which trusts the system's certificates (the OS's own verifier on Windows and
//! macOS), so it also works behind a proxy whose certificate the system trusts

use std::time::Duration;
use ureq::tls::{RootCerts, TlsConfig};

pub fn agent(timeout: Duration) -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(timeout))
        .user_agent(concat!("osdhub-manager/", env!("CARGO_PKG_VERSION")))
        .tls_config(
            TlsConfig::builder()
                .root_certs(RootCerts::PlatformVerifier)
                .build(),
        )
        .build()
        .into()
}
