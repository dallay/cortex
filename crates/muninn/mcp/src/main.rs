//! Standalone stdio shell. Application handlers are not wired yet.

fn main() -> std::io::Result<()> {
    muninn_mcp::serve(
        std::io::stdin().lock(),
        std::io::stdout().lock(),
        &mut muninn_mcp::Server::default(),
    )
}
