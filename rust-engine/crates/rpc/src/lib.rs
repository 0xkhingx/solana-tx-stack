use anyhow::Result;
use solana_client::rpc_client::RpcClient;
use solana_commitment_config::CommitmentConfig;
use solana_hash::Hash;
use solana_signature::Signature;
use std::time::Duration;

/// Thin RPC wrapper with confirmed-default semantics and simple retry policy.
pub struct RpcWrapper {
    client: RpcClient,
}

impl RpcWrapper {
    pub fn new(url: String) -> Self {
        Self {
            client: RpcClient::new_with_timeout_and_commitment(
                url,
                Duration::from_secs(2),
                CommitmentConfig::confirmed(),
            ),
        }
    }

    pub fn get_latest_blockhash(&self) -> Result<Hash> {
        retry_3(|| Ok(self.client.get_latest_blockhash()?))
    }

    pub fn get_signature_statuses(&self, signatures: &[Signature]) -> Result<Vec<Option<solana_client::rpc_response::RpcSignatureResult>>> {
        retry_3(|| Ok(self.client.get_signature_statuses(signatures)?.value))
    }

    pub fn get_slot(&self) -> Result<u64> {
        retry_3(|| Ok(self.client.get_slot_with_commitment(CommitmentConfig::confirmed())?))
    }
}

fn retry_3<T, F>(mut f: F) -> Result<T>
where
    F: FnMut() -> Result<T>,
{
    let mut last_err = None;
    for attempt in 0..3 {
        match f() {
            Ok(value) => return Ok(value),
            Err(err) => {
                last_err = Some(err);
                if attempt < 2 {
                    std::thread::sleep(Duration::from_millis(500));
                }
            }
        }
    }
    Err(last_err.unwrap())
}
