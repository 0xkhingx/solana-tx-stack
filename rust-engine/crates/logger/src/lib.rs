use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::{path::PathBuf, sync::Arc};
use tokio::{fs::OpenOptions, io::AsyncWriteExt, sync::mpsc};

/// Normalized bundle lifecycle record written to JSONL.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LifecycleRecord {
    pub bundle_id: String,
    pub tip_lamports: u64,
    pub submitted_at_slot: Option<u64>,
    pub submitted_at_ts: Option<String>,
    pub processed_at_slot: Option<u64>,
    pub processed_at_ts: Option<String>,
    pub confirmed_at_slot: Option<u64>,
    pub confirmed_at_ts: Option<String>,
    pub finalized_at_slot: Option<u64>,
    pub finalized_at_ts: Option<String>,
    pub failure_reason: Option<serde_json::Value>,
    pub agent_decision: Option<serde_json::Value>,
    pub retry_count: u32,
    pub final_status: String,
}

pub struct LifecycleLogger {
    output_path: Arc<PathBuf>,
}

impl LifecycleLogger {
    pub fn new(output_path: PathBuf) -> Self {
        Self { output_path: Arc::new(output_path) }
    }

    /// Consumes lifecycle events from the engine channel and appends JSONL records.
    pub async fn run(&self, mut rx: mpsc::Receiver<LifecycleRecord>) -> Result<()> {
        let mut file = OpenOptions::new().create(true).append(true).open(&*self.output_path).await?;
        while let Some(record) = rx.recv().await {
            let line = serde_json::to_string(&record)?;
            file.write_all(line.as_bytes()).await?;
            file.write_all(b"\n").await?;
        }
        Ok(())
    }
}
