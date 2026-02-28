pub mod identity;
pub mod protocol;
pub mod sandbox;
pub mod store;
pub mod ws_client;
// Future modules:
// pub mod observability;

use crate::Config;

pub async fn run_node(hub_url: &str, _config: &Config) -> anyhow::Result<()> {
    println!("ZeroClaw Node Worker starting...");
    println!("Target hub: {}", hub_url);
    
    // Test identity generation/loading
    let id_path = std::path::PathBuf::from("/var/lib/zeroclaw/node.json");
    // Fallback path if not on unix
    let id_path = if cfg!(unix) {
        id_path
    } else {
        std::path::PathBuf::from("./.zeroclaw_node.json")
    };

    println!("Loading identity from: {:?}", id_path);
    let identity = identity::NodeIdentity::load_or_create(&id_path)?;
    println!("Node Identity OK. ID: {}", identity.node_id);

    let db_path = std::path::PathBuf::from("/var/lib/zeroclaw/jobs.db");
    let db_path = if cfg!(unix) { db_path } else { std::path::PathBuf::from("./zeroclaw_jobs.db") };
    let store = std::sync::Arc::new(crate::worker::store::JobStore::new(&db_path)?);
    println!("SQLite Job Store initialized at {:?}", db_path);
    
    let client = crate::worker::ws_client::WsClient::new(hub_url.to_string(), identity, _config.clone(), store);
    client.connect_and_run().await?;
    
    Ok(())
}
