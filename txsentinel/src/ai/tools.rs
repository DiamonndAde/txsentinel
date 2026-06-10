use serde_json::{json, Value};

use crate::bundle::TipPercentiles;
use crate::lifecycle::FailureKind;

/// Serialize current tip percentiles as a tool result for the AI agent
pub fn tip_percentiles_tool_result(p: &TipPercentiles, tps: u64) -> Value {
    json!({
        "tool": "get_tip_percentiles",
        "result": {
            "p25_lamports": p.p25,
            "p50_lamports": p.p50,
            "p75_lamports": p.p75,
            "p95_lamports": p.p95,
            "p99_lamports": p.p99,
            "ema_landed_p50": p.ema_landed,
            "current_tps": tps,
            "network_load": classify_tps(tps),
        }
    })
}

/// Serialize leader window info as a tool result
pub fn leader_window_tool_result(slots_until_jito: u64, window_size: u64, current_slot: u64) -> Value {
    json!({
        "tool": "get_leader_window",
        "result": {
            "current_slot": current_slot,
            "slots_until_next_jito_leader": slots_until_jito,
            "jito_window_size_slots": window_size,
            "estimated_wait_ms": slots_until_jito * 400,
        }
    })
}

/// Serialize failure context as a tool result
pub fn failure_context_tool_result(
    failure_kind: &FailureKind,
    retry_count: u32,
    last_tip: u64,
    slot_age_at_submission: u64,
) -> Value {
    json!({
        "tool": "get_failure_context",
        "result": {
            "failure_type": failure_kind.label(),
            "explanation": crate::failure::FailureClassifier::explain(failure_kind),
            "retry_attempt": retry_count + 1,
            "previous_tip_lamports": last_tip,
            "blockhash_slot_age_at_submission": slot_age_at_submission,
            "blockhash_validity_slots": 150,
        }
    })
}

/// Serialize network health as a tool result
pub fn network_health_tool_result(
    proc_to_conf_delta_ms: Option<i64>,
    recent_landing_rate: f64,
) -> Value {
    json!({
        "tool": "get_network_health",
        "result": {
            "processed_to_confirmed_delta_ms": proc_to_conf_delta_ms,
            "health": classify_delta(proc_to_conf_delta_ms),
            "recent_landing_rate_pct": recent_landing_rate,
        }
    })
}

fn classify_tps(tps: u64) -> &'static str {
    match tps {
        0..=1500 => "low",
        1501..=3000 => "moderate",
        3001..=5000 => "high",
        _ => "very_high",
    }
}

fn classify_delta(delta_ms: Option<i64>) -> &'static str {
    match delta_ms {
        None => "unknown",
        Some(d) if d < 500 => "healthy",
        Some(d) if d < 1500 => "moderate_lag",
        Some(d) if d < 3000 => "high_lag",
        _ => "critical_lag",
    }
}
