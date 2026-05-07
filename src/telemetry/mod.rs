use anyhow::Result;
use tracing_subscriber::prelude::*;
use tracing_subscriber::EnvFilter;

/// Initialise tracing with OpenTelemetry and console output.
///
/// Set `RUST_LOG` to control verbosity (default: `info`).
/// Set `OTEL_EXPORTER_OTLP_ENDPOINT` to ship spans to an OTLP collector.
pub fn init() -> Result<()> {
    let env_filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("datamesh=info,warn"));

    let fmt_layer = tracing_subscriber::fmt::layer()
        .with_target(true)
        .with_thread_ids(false)
        .compact();

    tracing_subscriber::registry()
        .with(env_filter)
        .with(fmt_layer)
        .init();

    Ok(())
}
