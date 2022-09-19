use std::{
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

use anyhow::{Context as AnyhowContext, Result};
use clap::Parser;
use pin_project_lite::pin_project;
use rustls::ClientConfig;
use tokio::{
    io::{AsyncRead, AsyncWrite, ReadBuf},
    net::TcpStream,
};
use tokio_rustls::{client::TlsStream, TlsConnector};
use tracing::debug;

/// Transport CLI config.
#[derive(Debug, Parser)]
pub struct TransportCLIConfig {
    /// Use TLS.
    #[clap(short, long)]
    tls: bool,

    /// Host and port.
    #[clap()]
    addr: String,
}

pin_project! {
    #[project = TransportProj]
    #[derive(Debug)]
    pub enum Transport {
        Plain{
            #[pin]
            inner: TcpStream,
        },

        Tls{
            #[pin]
            inner: TlsStream<TcpStream>,
        },
    }
}

impl AsyncRead for Transport {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        match self.project() {
            TransportProj::Plain { inner } => inner.poll_read(cx, buf),

            TransportProj::Tls { inner } => inner.poll_read(cx, buf),
        }
    }
}

impl AsyncWrite for Transport {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        match self.project() {
            TransportProj::Plain { inner } => inner.poll_write(cx, buf),

            TransportProj::Tls { inner } => inner.poll_write(cx, buf),
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        match self.project() {
            TransportProj::Plain { inner } => inner.poll_flush(cx),

            TransportProj::Tls { inner } => inner.poll_flush(cx),
        }
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        match self.project() {
            TransportProj::Plain { inner } => inner.poll_shutdown(cx),

            TransportProj::Tls { inner } => inner.poll_shutdown(cx),
        }
    }
}

pub async fn setup_transport(cfg: TransportCLIConfig) -> Result<Transport> {
    let tcp_stream = TcpStream::connect(&cfg.addr).await.context("TCP connect")?;
    tcp_stream.set_nodelay(true).context("set TCP_NODELAY")?;
    debug!(addr = cfg.addr.as_str(), "TCP connected");

    if cfg.tls {
        // Strip port if any
        let host = cfg.addr.split(':').next().context("invalid host-port")?;
        let server_name = rustls::ServerName::try_from(host).context("hostname parsing")?;

        let mut root_store = rustls::RootCertStore::empty();
        root_store.add_server_trust_anchors(webpki_roots::TLS_SERVER_ROOTS.0.iter().map(|ta| {
            rustls::OwnedTrustAnchor::from_subject_spki_name_constraints(
                ta.subject,
                ta.spki,
                ta.name_constraints,
            )
        }));

        let mut config = ClientConfig::builder()
            .with_safe_defaults()
            .with_root_certificates(root_store)
            .with_no_client_auth();
        config.alpn_protocols = vec![b"h2".to_vec()];

        let connector = TlsConnector::from(Arc::new(config));
        let tls_stream = connector
            .connect(server_name, tcp_stream)
            .await
            .context("TLS connect")?;
        debug!(addr = cfg.addr.as_str(), "TLS connected");

        Ok(Transport::Tls { inner: tls_stream })
    } else {
        Ok(Transport::Plain { inner: tcp_stream })
    }
}
