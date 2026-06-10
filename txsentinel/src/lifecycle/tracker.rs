use anyhow::Result;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tracing::info;

use super::{BundleEntry, CommitmentStage, FailureKind, LifecycleLog};

#[derive(Clone)]
pub struct LifecycleTracker {
    entries: Arc<Mutex<HashMap<String, BundleEntry>>>,
    log: LifecycleLog,
}

impl LifecycleTracker {
    pub fn new(log: LifecycleLog) -> Self {
        Self {
            entries: Arc::new(Mutex::new(HashMap::new())),
            log,
        }
    }

    pub fn register(&self, entry: BundleEntry) -> Result<()> {
        let sig = entry.signature.clone();
        self.log.upsert(&entry)?;
        self.entries.lock().unwrap().insert(sig, entry);
        Ok(())
    }

    pub fn advance_processed(&self, signature: &str, slot: u64) -> Result<()> {
        let mut map = self.entries.lock().unwrap();
        if let Some(e) = map.get_mut(signature) {
            if e.stage == CommitmentStage::Submitted {
                e.advance_to_processed(slot);
                info!(sig = signature, slot, "→ Processed");
                self.log.upsert(e)?;
            }
        }
        Ok(())
    }

    pub fn advance_confirmed(&self, signature: &str, slot: u64) -> Result<()> {
        let mut map = self.entries.lock().unwrap();
        if let Some(e) = map.get_mut(signature) {
            if e.stage == CommitmentStage::Processed {
                e.advance_to_confirmed(slot);
                info!(sig = signature, slot, "→ Confirmed");
                self.log.upsert(e)?;
            }
        }
        Ok(())
    }

    pub fn advance_finalized(&self, signature: &str, slot: u64) -> Result<()> {
        let mut map = self.entries.lock().unwrap();
        if let Some(e) = map.get_mut(signature) {
            if e.stage == CommitmentStage::Confirmed {
                e.advance_to_finalized(slot);
                info!(sig = signature, slot, "→ Finalized");
                self.log.upsert(e)?;
            }
        }
        Ok(())
    }

    pub fn mark_failed(&self, signature: &str, reason: &str) -> Result<Option<BundleEntry>> {
        let mut map = self.entries.lock().unwrap();
        if let Some(e) = map.get_mut(signature) {
            let kind = FailureKind::from_str(reason);
            e.mark_failed(kind);
            info!(sig = signature, reason, "→ Failed");
            self.log.upsert(e)?;
            return Ok(Some(e.clone()));
        }
        Ok(None)
    }

    pub fn update_ai_reasoning(&self, signature: &str, reasoning: &str, tip: u64) -> Result<()> {
        let mut map = self.entries.lock().unwrap();
        if let Some(e) = map.get_mut(signature) {
            e.ai_reasoning = Some(reasoning.to_string());
            e.ai_tip_decision = Some(tip);
            self.log.upsert(e)?;
        }
        Ok(())
    }

    pub fn get(&self, signature: &str) -> Option<BundleEntry> {
        self.entries.lock().unwrap().get(signature).cloned()
    }

    pub fn all_active(&self) -> Vec<BundleEntry> {
        self.entries
            .lock()
            .unwrap()
            .values()
            .cloned()
            .collect::<Vec<_>>()
    }

    pub fn recent_from_log(&self) -> Result<Vec<BundleEntry>> {
        self.log.all_entries()
    }
}
