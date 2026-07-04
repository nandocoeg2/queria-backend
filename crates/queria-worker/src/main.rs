mod jobs;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    queria_observability::init_json_tracing("queria-worker", "info");
    jobs::run_once().await;
    Ok(())
}
