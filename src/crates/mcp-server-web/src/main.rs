use anyhow::Result;
use mcp_server_web::{config::Config, server::WebServer};
use rmcp::{ServiceExt, transport::stdio};
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<()> {
    // Stdout belongs to the MCP transport, so logs go to stderr.
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .init();

    let server = WebServer::new(Config::from_env()?)?;
    let service = server.serve(stdio()).await.inspect_err(|error| {
        tracing::error!("serving error: {error:?}");
    })?;

    service.waiting().await?;
    Ok(())
}
