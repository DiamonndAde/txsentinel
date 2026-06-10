use anyhow::Result;
use serde::Deserialize;
use serde_json;
use std::sync::{Arc, Mutex};

const SOL_TO_LAMPORTS: f64 = 1_000_000_000.0;
// Correct tip floor endpoint (different host from block engine)
const TIP_FLOOR_URL: &str = "https://bundles.jito.wtf/api/v1/bundles/tip_floor";

#[derive(Debug, Clone, Default)]
pub struct TipPercentiles {
    pub p25: u64,
    pub p50: u64,
    pub p75: u64,
    pub p95: u64,
    pub p99: u64,
    pub ema_landed: u64,
}

#[derive(Debug, Deserialize)]
struct TipFloorResponse {
    #[serde(rename = "landed_tips_25th_percentile")]
    p25: Option<f64>,
    #[serde(rename = "landed_tips_50th_percentile")]
    p50: Option<f64>,
    #[serde(rename = "landed_tips_75th_percentile")]
    p75: Option<f64>,
    #[serde(rename = "landed_tips_95th_percentile")]
    p95: Option<f64>,
    #[serde(rename = "landed_tips_99th_percentile")]
    p99: Option<f64>,
    #[serde(rename = "ema_landed_tips_50th_percentile")]
    ema_landed: Option<f64>,
}

#[derive(Clone)]
pub struct TipOracle {
    client: reqwest::Client,
    cached: Arc<Mutex<TipPercentiles>>,
}

impl TipOracle {
    pub fn new(_jito_url: &str) -> Self {
        Self {
            client: reqwest::Client::new(),
            cached: Arc::new(Mutex::new(TipPercentiles::default())),
        }
    }

    pub async fn refresh(&self) -> Result<TipPercentiles> {
        let resp = self.client.get(TIP_FLOOR_URL).send().await?;
        if !resp.status().is_success() {
            tracing::debug!("tip_floor HTTP {}: using cached/defaults", resp.status());
            return Ok(self.get_or_defaults());
        }
        let text = resp.text().await?;
        tracing::debug!("tip_floor raw: {}", &text[..text.len().min(300)]);

        // Response is an array; values are in SOL — multiply by 1e9 to get lamports
        let entry: Option<TipFloorResponse> = if let Ok(arr) = serde_json::from_str::<Vec<TipFloorResponse>>(&text) {
            arr.into_iter().next()
        } else if let Ok(obj) = serde_json::from_str::<TipFloorResponse>(&text) {
            Some(obj)
        } else {
            tracing::warn!("tip_floor parse failed: {}", &text[..text.len().min(200)]);
            None
        };

        if let Some(first) = entry {
            let sol_to_lam = |v: f64| (v * SOL_TO_LAMPORTS).round() as u64;
            let p = TipPercentiles {
                p25: sol_to_lam(first.p25.unwrap_or(0.000_001)),
                p50: sol_to_lam(first.p50.unwrap_or(0.000_005)),
                p75: sol_to_lam(first.p75.unwrap_or(0.000_010)),
                p95: sol_to_lam(first.p95.unwrap_or(0.000_100)),
                p99: sol_to_lam(first.p99.unwrap_or(0.000_200)),
                ema_landed: sol_to_lam(first.ema_landed.unwrap_or(0.000_005)),
            };
            *self.cached.lock().unwrap() = p.clone();
            Ok(p)
        } else {
            Ok(self.get_or_defaults())
        }
    }

    pub fn cached(&self) -> TipPercentiles {
        self.cached.lock().unwrap().clone()
    }

    /// Baseline naive tip: always p50, no reasoning
    pub fn baseline_tip(&self) -> u64 {
        self.cached().p50
    }

    fn get_or_defaults(&self) -> TipPercentiles {
        let cached = self.cached.lock().unwrap().clone();
        if cached.p50 == 0 {
            let defaults = TipPercentiles {
                p25: 2_143,
                p50: 8_040,
                p75: 11_001,
                p95: 100_000,
                p99: 155_000,
                ema_landed: 5_365,
            };
            *self.cached.lock().unwrap() = defaults.clone();
            defaults
        } else {
            cached
        }
    }
}
