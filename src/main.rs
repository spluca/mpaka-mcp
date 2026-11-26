use mpaka_mcp::{McpConfig, serve_stdio};

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter("info")
        .with_writer(std::io::stderr)
        .init();

    let result = async {
        let config = McpConfig::from_env()
            .map_err(|error| format!("invalid mPaka MCP configuration: {error:?}"))?;
        serve_stdio(config)
            .await
            .map_err(|error| format!("mPaka MCP server failed: {error}"))
    }
    .await;
    if let Err(error) = result {
        tracing::error!(%error, "mPaka MCP server stopped");
        std::process::exit(1);
    }
}
