use anyhow::Result;
use rusqlite::{Connection, params};
use std::sync::{Arc, Mutex};

use super::types::BundleEntry;

#[derive(Clone)]
pub struct LifecycleLog {
    conn: Arc<Mutex<Connection>>,
}

impl LifecycleLog {
    pub fn open(path: &str) -> Result<Self> {
        let conn = Connection::open(path)?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS bundles (
                id              TEXT PRIMARY KEY,
                bundle_id       TEXT,
                signature       TEXT NOT NULL,
                tip_lamports    INTEGER NOT NULL,
                submitted_slot  INTEGER NOT NULL,
                stage           TEXT NOT NULL,
                submitted_at    TEXT NOT NULL,
                processed_at    TEXT,
                confirmed_at    TEXT,
                finalized_at    TEXT,
                failed_at       TEXT,
                processed_slot  INTEGER,
                confirmed_slot  INTEGER,
                finalized_slot  INTEGER,
                sub_to_proc_ms  INTEGER,
                proc_to_conf_ms INTEGER,
                conf_to_fin_ms  INTEGER,
                ai_reasoning    TEXT,
                ai_tip_decision INTEGER,
                baseline_tip    INTEGER,
                retry_count     INTEGER NOT NULL DEFAULT 0,
                injected_fault  TEXT,
                data_json       TEXT NOT NULL
            );",
        )?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    pub fn upsert(&self, entry: &BundleEntry) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        let json = serde_json::to_string(entry)?;
        conn.execute(
            "INSERT INTO bundles (
                id, bundle_id, signature, tip_lamports, submitted_slot, stage,
                submitted_at, processed_at, confirmed_at, finalized_at, failed_at,
                processed_slot, confirmed_slot, finalized_slot,
                sub_to_proc_ms, proc_to_conf_ms, conf_to_fin_ms,
                ai_reasoning, ai_tip_decision, baseline_tip,
                retry_count, injected_fault, data_json
            ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21,?22,?23)
            ON CONFLICT(id) DO UPDATE SET
                bundle_id=excluded.bundle_id, stage=excluded.stage,
                processed_at=excluded.processed_at, confirmed_at=excluded.confirmed_at,
                finalized_at=excluded.finalized_at, failed_at=excluded.failed_at,
                processed_slot=excluded.processed_slot, confirmed_slot=excluded.confirmed_slot,
                finalized_slot=excluded.finalized_slot,
                sub_to_proc_ms=excluded.sub_to_proc_ms,
                proc_to_conf_ms=excluded.proc_to_conf_ms,
                conf_to_fin_ms=excluded.conf_to_fin_ms,
                ai_reasoning=excluded.ai_reasoning,
                ai_tip_decision=excluded.ai_tip_decision,
                baseline_tip=excluded.baseline_tip,
                retry_count=excluded.retry_count,
                data_json=excluded.data_json",
            params![
                entry.id,
                entry.bundle_id,
                entry.signature,
                entry.tip_lamports as i64,
                entry.submitted_slot as i64,
                entry.stage.to_string(),
                entry.submitted_at.to_rfc3339(),
                entry.processed_at.map(|t| t.to_rfc3339()),
                entry.confirmed_at.map(|t| t.to_rfc3339()),
                entry.finalized_at.map(|t| t.to_rfc3339()),
                entry.failed_at.map(|t| t.to_rfc3339()),
                entry.processed_slot.map(|s| s as i64),
                entry.confirmed_slot.map(|s| s as i64),
                entry.finalized_slot.map(|s| s as i64),
                entry.submitted_to_processed_ms,
                entry.processed_to_confirmed_ms,
                entry.confirmed_to_finalized_ms,
                entry.ai_reasoning.as_deref(),
                entry.ai_tip_decision.map(|t| t as i64),
                entry.baseline_tip.map(|t| t as i64),
                entry.retry_count as i64,
                entry.injected_fault.as_deref(),
                json,
            ],
        )?;
        Ok(())
    }

    pub fn all_entries(&self) -> Result<Vec<BundleEntry>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT data_json FROM bundles ORDER BY submitted_at DESC LIMIT 50")?;
        let entries = stmt
            .query_map([], |row| row.get::<_, String>(0))?
            .filter_map(|r| r.ok())
            .filter_map(|json| serde_json::from_str(&json).ok())
            .collect();
        Ok(entries)
    }
}
