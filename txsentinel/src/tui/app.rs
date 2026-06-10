use crate::bundle::TipPercentiles;
use crate::lifecycle::BundleEntry;
use crate::slot_monitor::SlotState;

#[derive(Default, Clone)]
pub struct App {
    pub slot_state: SlotState,
    pub tip_percentiles: TipPercentiles,
    pub slots_until_jito: u64,
    pub active_bundles: Vec<BundleEntry>,
    pub recent_log: Vec<BundleEntry>,
    pub ai_reasoning_lines: Vec<String>,
    pub ai_summary: String,
    pub status_message: String,
    pub should_quit: bool,
    pub submission_count: u32,
    pub network: String,
    pub is_devnet: bool,
    pub ai_scroll: usize,
}

impl App {
    pub fn new(network: String, is_devnet: bool) -> Self {
        Self {
            status_message: "Starting TxSentinel...".to_string(),
            network,
            is_devnet,
            ..Default::default()
        }
    }

    pub fn push_ai_line(&mut self, line: String) {
        self.ai_reasoning_lines.push(line);
        // Keep last 60 lines; auto-scroll to bottom on new content
        if self.ai_reasoning_lines.len() > 60 {
            self.ai_reasoning_lines.remove(0);
        }
        // Auto-scroll to show newest line
        self.ai_scroll = self.ai_reasoning_lines.len().saturating_sub(1);
    }

    pub fn clear_ai_lines(&mut self) {
        self.ai_reasoning_lines.clear();
        self.ai_scroll = 0;
        self.ai_summary.clear();
    }

    pub fn ai_scroll_up(&mut self) {
        self.ai_scroll = self.ai_scroll.saturating_sub(1);
    }

    pub fn ai_scroll_down(&mut self) {
        if !self.ai_reasoning_lines.is_empty() {
            self.ai_scroll = (self.ai_scroll + 1).min(self.ai_reasoning_lines.len() - 1);
        }
    }

    pub fn set_status(&mut self, msg: impl Into<String>) {
        self.status_message = msg.into();
    }
}
