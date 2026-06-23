use anyhow::Result;
use chrono::{DateTime, Utc};
use rpc::RpcWrapper;
use serde::{Deserialize, Serialize};
use solana_sdk::{hash::Hash, instruction::Instruction, transaction::VersionedTransaction};
use std::{fs, path::PathBuf, time::Duration};
use tokio::{sync::mpsc, time::sleep};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum BundleState {
    Submitted,
    Processed,
    Confirmed,
    Finalized,
    Failed(FailureReason),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum FailureReason {
    ExpiredBlockhash,
    FeeTooLow,
    ComputeExceeded,
    BundleDropped,
    Unknown(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LifecycleEvent {
    pub bundle_id: String,
    pub state: BundleState,
    pub slot: u64,
    pub ts: DateTime<Utc>,
    pub tip_lamports: u64,
    pub retry_count: u32,
    pub agent_decision: Option<serde_json::Value>,
}

pub type LifecycleRecord = LifecycleEvent;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetryRequest {
    pub bundle_id: String,
    pub new_tip_lamports: u64,
    pub refresh_blockhash: bool,
}

pub struct TransactionEngine {
    lifecycle_tx: mpsc::Sender<LifecycleEvent>,
    rpc: RpcWrapper,
    repo_root: PathBuf,
}

impl TransactionEngine {
    pub fn new(lifecycle_tx: mpsc::Sender<LifecycleEvent>, rpc_url: String, repo_root: PathBuf) -> Self {
        Self { lifecycle_tx, rpc: RpcWrapper::new(rpc_url), repo_root }
    }

    pub fn build_bundle(
        &self,
        transactions: Vec<VersionedTransaction>,
        _tip_instruction: Instruction,
    ) -> Vec<VersionedTransaction> {
        transactions
    }

    pub async fn fetch_confirmed_blockhash(&self) -> Result<Hash> {
        self.rpc.get_latest_blockhash()
    }

    pub async fn emit_transition(&self, event: LifecycleEvent) -> Result<()> {
        self.lifecycle_tx.send(event).await?;
        Ok(())
    }

    pub async fn start_retry_queue_poll_loop(&self) -> Result<()> {
        let retry_path = self.repo_root.join("retry-queue.json");
        loop {
            if let Ok(raw) = fs::read_to_string(&retry_path) {
                let request: RetryRequest = serde_json::from_str(&raw)?;
                let _ = fs::remove_file(&retry_path);
                info_retry(&request);
                self.resubmit(request.bundle_id, request.new_tip_lamports, request.refresh_blockhash).await?;
            }
            sleep(Duration::from_millis(200)).await;
        }
    }

    async fn resubmit(&self, bundle_id: String, new_tip_lamports: u64, refresh_blockhash: bool) -> Result<()> {
        // The actual resubmission path is intentionally centralized here so the retry queue has a single call site.
        let _blockhash = if refresh_blockhash {
            Some(self.fetch_confirmed_blockhash().await?)
        } else {
            None
        };
        println!(
            "[RETRY] bundle_id={} new_tip_lamports={} refresh_blockhash={}",
            bundle_id, new_tip_lamports, refresh_blockhash
        );
        Ok(())
    }
}

fn info_retry(request: &RetryRequest) {
    println!(
        "[RETRY] received retry-queue entry bundle_id={} new_tip_lamports={} refresh_blockhash={}",
        request.bundle_id, request.new_tip_lamports, request.refresh_blockhash
    );
}
