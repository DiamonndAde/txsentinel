use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tracing::info;

use super::builder::{Bundle, BundleBuilder};

#[derive(Debug, Deserialize)]
pub struct BundleStatus {
    pub bundle_id: String,
    pub status: String,
    pub landed_slot: Option<u64>,
    pub err: Option<Value>,
}

#[derive(Clone)]
pub struct JitoSubmitter {
    client: reqwest::Client,
    jito_url: String,
}

impl JitoSubmitter {
    pub fn new(jito_url: &str) -> Self {
        Self {
            client: reqwest::Client::new(),
            jito_url: jito_url.to_string(),
        }
    }

    pub async fn get_tip_accounts(&self) -> Result<Vec<String>> {
        let url = format!("{}/api/v1/bundles/tip_accounts", self.jito_url);
        let accounts: Vec<String> = self.client.get(&url).send().await?.json().await?;
        Ok(accounts)
    }

    pub async fn send_bundle(&self, bundle: &Bundle) -> Result<String> {
        let encoded_txs = BundleBuilder::encode_transactions(bundle)?;

        let payload = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "sendBundle",
            "params": [encoded_txs, {"encoding": "base64"}]
        });

        let url = format!("{}/api/v1/bundles", self.jito_url);
        let resp: Value = self
            .client
            .post(&url)
            .json(&payload)
            .send()
            .await?
            .json()
            .await?;

        if let Some(err) = resp.get("error") {
            return Err(anyhow!("Jito error: {err}"));
        }

        let bundle_id = resp["result"]
            .as_str()
            .ok_or_else(|| anyhow!("No bundle ID in response: {resp}"))?
            .to_string();

        info!(bundle_id = %bundle_id, "Bundle submitted to Jito");
        Ok(bundle_id)
    }

    pub async fn get_bundle_status(&self, bundle_id: &str) -> Result<Option<BundleStatus>> {
        let payload = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "getBundleStatuses",
            "params": [[bundle_id]]
        });

        let url = format!("{}/api/v1/bundles", self.jito_url);
        let resp: Value = self
            .client
            .post(&url)
            .json(&payload)
            .send()
            .await?
            .json()
            .await?;

        let statuses = resp["result"]["value"]
            .as_array()
            .cloned()
            .unwrap_or_default();

        if let Some(s) = statuses.first() {
            let status = BundleStatus {
                bundle_id: s["bundle_id"].as_str().unwrap_or("").to_string(),
                status: s["confirmation_status"].as_str().unwrap_or("unknown").to_string(),
                landed_slot: s["slot"].as_u64(),
                err: s.get("err").cloned(),
            };
            return Ok(Some(status));
        }

        Ok(None)
    }
}
