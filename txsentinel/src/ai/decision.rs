use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentDecision {
    /// The tip amount the agent decided on (lamports)
    pub tip_lamports: u64,
    /// Full reasoning trace from DeepSeek R1 <think> block
    pub reasoning: String,
    /// Short summary for TUI display
    pub summary: String,
    /// Which percentile bracket the agent selected
    pub percentile_used: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetryDecision {
    pub should_retry: bool,
    pub new_tip_lamports: u64,
    pub wait_slots: u64,
    pub reasoning: String,
    pub summary: String,
    pub failure_diagnosis: String,
}
