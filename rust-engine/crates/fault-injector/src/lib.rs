use anyhow::Result;
use rpc::RpcWrapper;
use solana_sdk::hash::Hash;
use tokio::time::{sleep, Duration};

#[derive(Debug, Clone)]
pub enum FaultType {
    ExpiredBlockhashFault,
    LowTipFault,
}

#[derive(Debug, Clone)]
pub enum FaultOutcome {
    StaleBlockhash(Hash),
    TipLamports(u64),
}

pub struct FaultInjector {
    rpc: RpcWrapper,
}

impl FaultInjector {
    pub fn new(rpc_url: String) -> Self {
        Self { rpc: RpcWrapper::new(rpc_url) }
    }

    /// Returns the concrete input the caller should use to provoke the requested failure.
    pub async fn inject(&self, fault_type: FaultType) -> Result<FaultOutcome> {
        match fault_type {
            FaultType::ExpiredBlockhashFault => {
                let stale = self.rpc.get_latest_blockhash()?;
                // A 40s delay is a simple, async-safe way to ensure the fetched blockhash ages out.
                sleep(Duration::from_secs(40)).await;
                Ok(FaultOutcome::StaleBlockhash(stale))
            }
            FaultType::LowTipFault => Ok(FaultOutcome::TipLamports(1)),
        }
    }
}
