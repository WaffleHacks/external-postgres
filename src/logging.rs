use tracing::Level;
use tracing_error::ErrorLayer;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter, Registry};

/// Initialize the tracing log layer
pub(crate) fn init(default_level: Level) -> eyre::Result<()> {
    color_eyre::install()?;

    Registry::default()
        .with(ErrorLayer::default())
        .with(
            EnvFilter::builder()
                .with_default_directive(default_level.into())
                .from_env_lossy(),
        )
        .with(
            tracing_subscriber::fmt::layer()
                .with_file(cfg!(debug_assertions))
                .with_line_number(cfg!(debug_assertions))
                .with_target(true),
        )
        .init();

    Ok(())
}
