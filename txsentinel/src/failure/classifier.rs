use crate::lifecycle::FailureKind;
use serde_json::Value;

pub struct FailureClassifier;

impl FailureClassifier {
    /// Classify a failure from a Jito bundle status or RPC error response
    pub fn classify(err: &Value) -> FailureKind {
        let msg = err.to_string().to_lowercase();
        Self::classify_str(&msg)
    }

    pub fn classify_str(msg: &str) -> FailureKind {
        let lower = msg.to_lowercase();
        // Check compute budget first — RPC responses often contain "blockhash" as a JSON
        // field name (e.g. "replacementBlockhash") which would false-trigger the blockhash check
        if lower.contains("computationalbudgetexceeded")
            || lower.contains("computational budget exceeded")
            || lower.contains("computeexceeded")
        {
            FailureKind::ComputeExceeded
        } else if lower.contains("blockhashnotfound")
            || lower.contains("blockhash not found")
            || lower.contains("block hash not found")
            || lower.contains("blockhash expired")
            || lower.contains("\"expiredblockhash\"")
        {
            FailureKind::ExpiredBlockhash
        } else if lower.contains("insufficientfundsfor")
            || lower.contains("insufficient lamports")
            || lower.contains("insufficient funds")
            || lower.contains("prioritization fee")
        {
            FailureKind::FeeTooLow
        } else if lower.contains("bundle") || lower.contains("dropped") {
            FailureKind::BundleFailure
        } else if lower.contains("skip") || lower.contains("leader") {
            FailureKind::LeaderSkipped
        } else {
            FailureKind::Unknown(msg.to_string())
        }
    }

    /// Human-readable explanation of why a failure occurred
    pub fn explain(kind: &FailureKind) -> &'static str {
        match kind {
            FailureKind::ExpiredBlockhash => {
                "Blockhash exceeded its 150-slot validity window before the transaction landed."
            }
            FailureKind::FeeTooLow => {
                "Priority fee was below the network floor; transaction was deprioritized or dropped."
            }
            FailureKind::ComputeExceeded => {
                "Transaction exceeded its compute budget allocation."
            }
            FailureKind::BundleFailure => {
                "Bundle was rejected by Jito block engine — leader may have skipped or tip was insufficient."
            }
            FailureKind::LeaderSkipped => {
                "The Jito leader skipped their slot; bundle was not included in any block."
            }
            FailureKind::Unknown(_) => {
                "Unknown failure — inspect raw error for details."
            }
        }
    }
}
