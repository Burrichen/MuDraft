//! The one place outbound HTTP clients are configured.
//!
//! reqwest verifies certificates through the platform. On Android that path needs a Java-side
//! verifier set up at app start and otherwise panics on the first request, so Android uses the
//! bundled Mozilla root certificates instead (updated with each app release). Every other
//! platform keeps reqwest's default, the operating system's own certificate store.

use crate::metadata::http::USER_AGENT;

/// A client builder with MuDraft's user agent and the platform-appropriate TLS setup.
pub fn client_builder() -> reqwest::ClientBuilder {
    let builder = reqwest::Client::builder().user_agent(USER_AGENT);
    #[cfg(target_os = "android")]
    let builder = builder.tls_backend_preconfigured(android_tls());
    builder
}

#[cfg(target_os = "android")]
fn android_tls() -> rustls::ClientConfig {
    use std::sync::Arc;

    let roots = rustls::RootCertStore {
        roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
    };
    let mut config = rustls::ClientConfig::builder_with_provider(Arc::new(
        rustls::crypto::aws_lc_rs::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .expect("the default TLS versions are supported")
    .with_root_certificates(roots)
    .with_no_client_auth();
    config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
    config
}
