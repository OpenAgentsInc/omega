#[tokio::main]
async fn main() -> agent_client_protocol::Result<()> {
    scv::serve().await
}
