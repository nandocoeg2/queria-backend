use tracing_subscriber::{EnvFilter, fmt};

pub fn init_json_tracing(service_name: &'static str, default_level: &str) {
    let env_filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(default_level));

    let _ = fmt()
        .json()
        .with_env_filter(env_filter)
        .with_target(true)
        .with_current_span(true)
        .with_span_list(true)
        .try_init();

    tracing::info!(service = service_name, "tracing initialized");
}
