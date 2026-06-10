use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum CommitmentStage {
    Submitted,
    Processed,
    Confirmed,
    Finalized,
    Failed(FailureKind),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum FailureKind {
    ExpiredBlockhash,
    FeeTooLow,
    ComputeExceeded,
    BundleFailure,
    LeaderSkipped,
    Unknown(String),
}

impl FailureKind {
    pub fn from_str(s: &str) -> Self {
        let lower = s.to_lowercase();
        if lower.contains("blockhash") || lower.contains("block hash") {
            Self::ExpiredBlockhash
        } else if lower.contains("insufficient") || lower.contains("fee") {
            Self::FeeTooLow
        } else if lower.contains("compute") || lower.contains("budget") {
            Self::ComputeExceeded
        } else if lower.contains("bundle") {
            Self::BundleFailure
        } else {
            Self::Unknown(s.to_string())
        }
    }

    pub fn label(&self) -> &str {
        match self {
            Self::ExpiredBlockhash => "BlockhashExpired",
            Self::FeeTooLow => "FeeTooLow",
            Self::ComputeExceeded => "ComputeExceeded",
            Self::BundleFailure => "BundleFailure",
            Self::LeaderSkipped => "LeaderSkipped",
            Self::Unknown(_) => "Unknown",
        }
    }
}

impl std::fmt::Display for CommitmentStage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Submitted => write!(f, "SUBMITTED"),
            Self::Processed => write!(f, "PROCESSED"),
            Self::Confirmed => write!(f, "CONFIRMED"),
            Self::Finalized => write!(f, "FINALIZED"),
            Self::Failed(k) => write!(f, "FAILED({})", k.label()),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BundleEntry {
    pub id: String,
    pub bundle_id: Option<String>,
    pub signature: String,
    pub tip_lamports: u64,
    pub submitted_slot: u64,
    pub stage: CommitmentStage,

    pub submitted_at: DateTime<Utc>,
    pub processed_at: Option<DateTime<Utc>>,
    pub confirmed_at: Option<DateTime<Utc>>,
    pub finalized_at: Option<DateTime<Utc>>,
    pub failed_at: Option<DateTime<Utc>>,

    pub processed_slot: Option<u64>,
    pub confirmed_slot: Option<u64>,
    pub finalized_slot: Option<u64>,

    // Latency deltas in ms
    pub submitted_to_processed_ms: Option<i64>,
    pub processed_to_confirmed_ms: Option<i64>,
    pub confirmed_to_finalized_ms: Option<i64>,

    // AI agent decision for this bundle
    pub ai_reasoning: Option<String>,
    pub ai_tip_decision: Option<u64>,

    // Counterfactual: what naive baseline would have tipped
    pub baseline_tip: Option<u64>,

    pub retry_count: u32,
    pub injected_fault: Option<String>,
}

impl BundleEntry {
    pub fn new(signature: String, tip_lamports: u64, submitted_slot: u64) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            bundle_id: None,
            signature,
            tip_lamports,
            submitted_slot,
            stage: CommitmentStage::Submitted,
            submitted_at: Utc::now(),
            processed_at: None,
            confirmed_at: None,
            finalized_at: None,
            failed_at: None,
            processed_slot: None,
            confirmed_slot: None,
            finalized_slot: None,
            submitted_to_processed_ms: None,
            processed_to_confirmed_ms: None,
            confirmed_to_finalized_ms: None,
            ai_reasoning: None,
            ai_tip_decision: None,
            baseline_tip: None,
            retry_count: 0,
            injected_fault: None,
        }
    }

    pub fn advance_to_processed(&mut self, slot: u64) {
        self.stage = CommitmentStage::Processed;
        self.processed_at = Some(Utc::now());
        self.processed_slot = Some(slot);
        if let Some(sub) = self.submitted_at.timestamp_millis().checked_sub(0) {
            let _ = sub;
        }
        self.submitted_to_processed_ms = Some(
            Utc::now()
                .signed_duration_since(self.submitted_at)
                .num_milliseconds(),
        );
    }

    pub fn advance_to_confirmed(&mut self, slot: u64) {
        self.stage = CommitmentStage::Confirmed;
        self.confirmed_at = Some(Utc::now());
        self.confirmed_slot = Some(slot);
        if let Some(processed_at) = self.processed_at {
            self.processed_to_confirmed_ms = Some(
                Utc::now()
                    .signed_duration_since(processed_at)
                    .num_milliseconds(),
            );
        }
    }

    pub fn advance_to_finalized(&mut self, slot: u64) {
        self.stage = CommitmentStage::Finalized;
        self.finalized_at = Some(Utc::now());
        self.finalized_slot = Some(slot);
        if let Some(confirmed_at) = self.confirmed_at {
            self.confirmed_to_finalized_ms = Some(
                Utc::now()
                    .signed_duration_since(confirmed_at)
                    .num_milliseconds(),
            );
        }
    }

    pub fn mark_failed(&mut self, kind: FailureKind) {
        self.stage = CommitmentStage::Failed(kind);
        self.failed_at = Some(Utc::now());
    }

    pub fn total_latency_ms(&self) -> Option<i64> {
        let end = self.finalized_at.or(self.failed_at)?;
        Some(end.signed_duration_since(self.submitted_at).num_milliseconds())
    }
}
