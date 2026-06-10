use anyhow::{Context, Result};
use std::path::PathBuf;

#[derive(Clone, Debug)]
pub struct Config {
    pub rpc_url: String,
    pub grpc_endpoint: String,
    pub grpc_x_token: String,
    pub jito_url: String,
    pub deepseek_api_key: String,
    pub deepseek_model: String,
    pub keypair_path: PathBuf,
    pub is_devnet: bool,
    pub network: String,
}

impl Config {
    pub fn from_env() -> Result<Self> {
        dotenvy::dotenv().ok();

        let network = std::env::var("NETWORK").unwrap_or_else(|_| "mainnet-beta".to_string());
        let is_devnet = network == "devnet";

        Ok(Config {
            rpc_url: env("RPC_URL")?,
            grpc_endpoint: env("GRPC_URL")?,
            grpc_x_token: env("GRPC_X_TOKEN")?,
            jito_url: env("JITO_BLOCK_ENGINE_URL")?,
            deepseek_api_key: env("DEEPSEEK_API_KEY")?,
            deepseek_model: std::env::var("DEEPSEEK_MODEL")
                .unwrap_or_else(|_| "deepseek-reasoner".to_string()),
            keypair_path: PathBuf::from(
                std::env::var("KEYPAIR_PATH")
                    .unwrap_or_else(|_| {
                        let home = std::env::var("HOME").unwrap_or_default();
                        format!("{home}/.config/solana/id.json")
                    }),
            ),
            is_devnet,
            network,
        })
    }
}

fn env(key: &str) -> Result<String> {
    std::env::var(key).with_context(|| format!("Missing env var: {key}"))
}
