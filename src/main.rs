use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use clap::Parser;
use futures::FutureExt;
use h2::{client, Ping};
use logging::{setup_logging, LoggingCLIConfig};
use tokio_util::sync::CancellationToken;
use tracing::{debug, info};
use transport::TransportCLIConfig;

use crate::transport::setup_transport;

mod logging;
mod transport;

/// CLI args.
#[derive(Debug, Parser)]
struct Args {
    /// Logging config.
    #[clap(flatten)]
    logging_cfg: LoggingCLIConfig,

    /// Transport config.
    #[clap(flatten)]
    transport_cfg: TransportCLIConfig,

    /// Count.
    #[clap(short, long)]
    count: Option<usize>,

    /// Interval between pings.
    #[clap(
        short,
        long,
        default_value="100ms",
        value_parser=humantime::parse_duration,
    )]
    interval: Duration,
}

/// Main entry point.
#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    setup_logging(args.logging_cfg)?;

    let transport = setup_transport(args.transport_cfg).await?;

    let (_send_request, mut connection) =
        client::handshake(transport).await.context("handshake")?;
    debug!("handshake complete",);

    // set up connection driver
    let mut ping_pong = connection.ping_pong().context("ping pong available")?;
    let cancel = CancellationToken::new();
    let cancel_captured = cancel.clone();
    let mut driver_handle = tokio::spawn(async move {
        let driver = async { connection.await.context("connection driver") };

        tokio::select! {
            _ = cancel_captured.cancelled() => Ok(()),
            r = driver => r,
        }
    })
    .fuse();

    // set up ping-pong loop
    let count = args.count.unwrap_or(usize::MAX);
    let looper = async {
        for _ in 0..count {
            let t_start = Instant::now();
            ping_pong.ping(Ping::opaque()).await.context("ping pong")?;
            info!(
                d=?t_start.elapsed(),
                "pong",
            );

            tokio::time::sleep(args.interval).await;
        }

        Ok(()) as Result<()>
    }
    .fuse();

    // run main construct
    tokio::pin!(looper);
    futures::select! {
        _ = looper => {},
        e = driver_handle => {
            return Err(e.unwrap_err().into());
        }
    }

    // shutdown
    cancel.cancel();
    driver_handle.await??;
    debug!("shutdown complete");

    Ok(())
}
