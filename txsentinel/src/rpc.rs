use anyhow::{anyhow, Result};
use base64::Engine as _;
use serde_json::{json, Value};
use solana_sdk::hash::Hash;
use solana_sdk::transaction::Transaction;
use std::str::FromStr;

/// Minimal Solana JSON-RPC client using reqwest directly.
/// Avoids the heavy solana-client dep tree and its crypto version conflicts.
#[derive(Clone)]
pub struct SolanaRpc {
    client: reqwest::Client,
    url: String,
}

impl SolanaRpc {
    pub fn new(url: &str) -> Self {
        Self {
            client: reqwest::Client::new(),
            url: url.to_string(),
        }
    }

    async fn call(&self, method: &str, params: Value) -> Result<Value> {
        let body = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": method,
            "params": params,
        });

        let resp: Value = self
            .client
            .post(&self.url)
            .json(&body)
            .send()
            .await?
            .json()
            .await?;

        if let Some(err) = resp.get("error") {
            return Err(anyhow!("RPC error ({method}): {err}"));
        }

        Ok(resp["result"].clone())
    }

    pub async fn get_latest_blockhash(&self) -> Result<Hash> {
        let result = self
            .call(
                "getLatestBlockhash",
                json!([{"commitment": "confirmed"}]),
            )
            .await?;

        let hash_str = result["value"]["blockhash"]
            .as_str()
            .ok_or_else(|| anyhow!("No blockhash in response"))?;

        Hash::from_str(hash_str).map_err(|e| anyhow!("Invalid blockhash: {e}"))
    }

    pub async fn get_slot(&self) -> Result<u64> {
        let result = self
            .call("getSlot", json!([{"commitment": "confirmed"}]))
            .await?;

        result
            .as_u64()
            .ok_or_else(|| anyhow!("Invalid slot response"))
    }

    pub async fn get_transaction_status(&self, signature: &str) -> Result<Option<String>> {
        let result = self
            .call(
                "getSignatureStatuses",
                json!([[signature], {"searchTransactionHistory": true}]),
            )
            .await?;

        if let Some(status) = result["value"][0].as_object() {
            if let Some(err) = status.get("err") {
                if !err.is_null() {
                    return Ok(Some(format!("failed:{err}")));
                }
            }
            let confirmation = status
                .get("confirmationStatus")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
                .to_string();
            return Ok(Some(confirmation));
        }

        Ok(None)
    }

    pub async fn get_balance(&self, pubkey: &str) -> Result<u64> {
        let result = self
            .call("getBalance", json!([pubkey, {"commitment": "confirmed"}]))
            .await?;

        result["value"]
            .as_u64()
            .ok_or_else(|| anyhow!("Invalid balance response"))
    }

    /// Submit a transaction directly via sendTransaction (devnet fallback — no Jito).
    pub async fn send_transaction(&self, tx: &Transaction) -> Result<String> {
        let encoded = bincode::serialize(tx).map_err(|e| anyhow!("serialize tx: {e}"))?;
        let b64 = base64::engine::general_purpose::STANDARD.encode(&encoded);

        let result = self
            .call(
                "sendTransaction",
                json!([b64, {
                    "encoding": "base64",
                    "skipPreflight": false,
                    "preflightCommitment": "confirmed",
                    "maxRetries": 3
                }]),
            )
            .await?;

        result
            .as_str()
            .map(|s| s.to_string())
            .ok_or_else(|| anyhow!("Invalid sendTransaction response: {:?}", result))
    }
}
