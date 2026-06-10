use anyhow::{anyhow, Result};
use serde_json::{json, Value};
use tracing::info;

use super::decision::{AgentDecision, RetryDecision};
use super::tools::*;
use crate::bundle::TipPercentiles;
use crate::lifecycle::FailureKind;

const SYSTEM_PROMPT: &str = r#"You are TxSentinel's transaction intelligence agent operating on Solana mainnet-beta.

Your role is to make real operational decisions about Jito bundle submissions:
1. How much to tip (in lamports) based on live network conditions
2. Whether to retry a failed transaction, and with what parameters

You have access to live telemetry: tip percentile data, network TPS, leader schedule windows, failure context, and network health signals.

Decision principles:
- Under high TPS (>3000): favor p80-p90 tip range to ensure landing
- Under low TPS (<1500): p50-p60 is usually sufficient
- Blockhash expiry failures: ALWAYS refresh blockhash before retry; also bump tip by 1 percentile bracket
- Fee too low failures: bump tip to p90 minimum
- Bundle failures: check if leader window is favorable; if not, wait for next Jito leader window
- Never exceed p99 unless retrying a critical transaction after 2+ failures
- Balance cost vs landing probability — overpaying wastes SOL, underpaying wastes time

Always write 2-3 sentences explaining your reasoning BEFORE the JSON block. Your reasoning will be displayed live in the TUI dashboard."#;

pub struct AiAgent {
    client: reqwest::Client,
    api_key: String,
    model: String,
    api_base: String,
}

impl AiAgent {
    pub fn new(api_key: &str, model: &str) -> Self {
        Self {
            client: reqwest::Client::new(),
            api_key: api_key.to_string(),
            model: model.to_string(),
            api_base: "https://api.deepseek.com".to_string(),
        }
    }

    /// Decide tip amount for a new bundle submission
    pub async fn decide_tip(
        &self,
        percentiles: &TipPercentiles,
        tps: u64,
        slots_until_jito: u64,
        window_size: u64,
        current_slot: u64,
        avg_proc_to_conf_ms: Option<i64>,
        recent_landing_rate: f64,
    ) -> Result<AgentDecision> {
        let tip_tool = tip_percentiles_tool_result(percentiles, tps);
        let leader_tool = leader_window_tool_result(slots_until_jito, window_size, current_slot);
        let health_tool = network_health_tool_result(avg_proc_to_conf_ms, recent_landing_rate);

        let user_msg = format!(
            "I need to submit a Jito bundle now. Here is the current telemetry:\n\n{}\n\n{}\n\n{}\n\nDecide the optimal tip amount in lamports. Show your step-by-step reasoning: which TPS band applies, which percentile target, and if interpolating between two known percentiles show the calculation (e.g. p90 = p75 + 0.75*(p95-p75)). Then respond with JSON: {{\"tip_lamports\": <number>, \"percentile_used\": \"<p50/p75/p80/p90/p95>\", \"summary\": \"<one sentence>\"}}",
            serde_json::to_string_pretty(&tip_tool)?,
            serde_json::to_string_pretty(&leader_tool)?,
            serde_json::to_string_pretty(&health_tool)?,
        );

        let (reasoning, content) = self.call_reasoner(&user_msg).await?;

        let parsed = extract_json(&content)?;
        let tip = parsed["tip_lamports"]
            .as_u64()
            .ok_or_else(|| anyhow!("No tip_lamports in agent response"))?;

        info!(tip, "AI agent tip decision");

        Ok(AgentDecision {
            tip_lamports: tip,
            reasoning,
            summary: parsed["summary"]
                .as_str()
                .unwrap_or("Agent decided tip")
                .to_string(),
            percentile_used: parsed["percentile_used"]
                .as_str()
                .unwrap_or("unknown")
                .to_string(),
        })
    }

    /// Decide whether and how to retry a failed bundle
    pub async fn decide_retry(
        &self,
        failure_kind: &FailureKind,
        retry_count: u32,
        last_tip: u64,
        slot_age: u64,
        percentiles: &TipPercentiles,
        tps: u64,
        slots_until_jito: u64,
        window_size: u64,
        current_slot: u64,
    ) -> Result<RetryDecision> {
        let failure_tool = failure_context_tool_result(failure_kind, retry_count, last_tip, slot_age);
        let tip_tool = tip_percentiles_tool_result(percentiles, tps);
        let leader_tool = leader_window_tool_result(slots_until_jito, window_size, current_slot);

        let user_msg = format!(
            "A bundle just failed. Here is the context:\n\n{}\n\n{}\n\n{}\n\nDecide whether to retry and with what parameters. Respond with JSON: {{\"should_retry\": <bool>, \"new_tip_lamports\": <number>, \"wait_slots\": <number>, \"failure_diagnosis\": \"<sentence>\", \"summary\": \"<one sentence>\"}}",
            serde_json::to_string_pretty(&failure_tool)?,
            serde_json::to_string_pretty(&tip_tool)?,
            serde_json::to_string_pretty(&leader_tool)?,
        );

        let (reasoning, content) = self.call_reasoner(&user_msg).await?;

        let parsed = extract_json(&content)?;

        Ok(RetryDecision {
            should_retry: parsed["should_retry"].as_bool().unwrap_or(true),
            new_tip_lamports: parsed["new_tip_lamports"].as_u64().unwrap_or(percentiles.p75),
            wait_slots: parsed["wait_slots"].as_u64().unwrap_or(0),
            reasoning,
            summary: parsed["summary"]
                .as_str()
                .unwrap_or("Agent decided retry")
                .to_string(),
            failure_diagnosis: parsed["failure_diagnosis"]
                .as_str()
                .unwrap_or(failure_kind.label())
                .to_string(),
        })
    }

    async fn call_reasoner(&self, user_message: &str) -> Result<(String, String)> {
        let payload = json!({
            "model": self.model,
            "messages": [
                {"role": "system", "content": SYSTEM_PROMPT},
                {"role": "user", "content": user_message}
            ],
            "max_tokens": 800,
            "temperature": 0.0,
        });

        let resp: Value = tokio::time::timeout(
            std::time::Duration::from_secs(45),
            self.client
                .post(format!("{}/chat/completions", self.api_base))
                .header("Authorization", format!("Bearer {}", self.api_key))
                .header("Content-Type", "application/json")
                .json(&payload)
                .send(),
        )
        .await
        .map_err(|_| anyhow!("DeepSeek API timed out after 45s"))??
        .json()
        .await?;

        if let Some(err) = resp.get("error") {
            return Err(anyhow!("DeepSeek error: {err}"));
        }

        let choice = &resp["choices"][0]["message"];
        let content = choice["content"].as_str().unwrap_or("").to_string();

        // R1 (deepseek-reasoner) returns chain-of-thought in reasoning_content.
        // V3 (deepseek-chat) puts everything in content — extract pre-JSON text as reasoning.
        let reasoning_raw = choice["reasoning_content"].as_str().unwrap_or("");
        let reasoning = if !reasoning_raw.is_empty() {
            reasoning_raw.to_string()
        } else {
            // Use any text before the JSON block as the visible reasoning
            content.find('{')
                .map(|i| content[..i].trim().to_string())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| content.clone())
        };

        Ok((reasoning, content))
    }
}

fn extract_json(text: &str) -> Result<Value> {
    // Find JSON block in the response
    if let Some(start) = text.find('{') {
        if let Some(end) = text.rfind('}') {
            let json_str = &text[start..=end];
            return Ok(serde_json::from_str(json_str)?);
        }
    }
    Err(anyhow!("No JSON found in agent response: {text}"))
}
