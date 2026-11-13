//! Standalone stdio shell. Application handlers are not wired yet.

#[tokio::main]
async fn main() -> std::io::Result<()> {
    muninn_mcp::serve(
        tokio::io::BufReader::new(tokio::io::stdin()),
        tokio::io::stdout(),
        &mut muninn_mcp::Server::default(),
    )
    .await
}
