use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use futures::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, env, path::PathBuf, time::Duration};
use tokio::{
    fs,
    sync::{broadcast, mpsc},
    time::sleep,
};
use tracing::{error, info};
use yellowstone_grpc_client::GeyserGrpcClient;
use yellowstone_grpc_proto::geyser::{
    SubscribeRequest, SubscribeRequestFilterAccounts, SubscribeRequestFilterSlots,
};
use solana_sdk::pubkey::Pubkey;

const TIP_ACCOUNTS: [&str; 5] = [
    "96gYZGLnJYVFmbjzopPSU6QiEV5fGqZNyN9nmNhvrZU5",
    "HFqU5x63VTqvQss8hp11i4wVV8bD44PvwucfZ2bU7gRe",
    "Cw8CFyM9FkoMi7K7Crf6HNQqf4uEMzpKw6QNghXLvLkY",
    "ADaUMid9yfUytqMBgopwjb2DTLSokTSzL1zcaozgVFze",
    "DfXygSm4jCyNCybVYYK6DwvWqjKee8pbDmJGcLWNDXjh",
];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SlotEvent {
    pub slot: u64,
    pub ts: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccountUpdateEvent {
    pub pubkey: String,
    pub slot: u64,
    pub ts: DateTime<Utc>,
    pub lamports: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum StreamEvent {
    Slot(SlotEvent),
    AccountUpdate(AccountUpdateEvent),
    Error(String),
}

pub struct YellowstoneSubscriber {
    endpoint: String,
    token: Option<String>,
    backpressure_tx: mpsc::Sender<StreamEvent>,
    broadcast_tx: broadcast::Sender<StreamEvent>,
    repo_root: PathBuf,
}

impl YellowstoneSubscriber {
    pub fn new(
        endpoint: String,
        token: Option<String>,
        backpressure_tx: mpsc::Sender<StreamEvent>,
        broadcast_tx: broadcast::Sender<StreamEvent>,
    ) -> Self {
        let repo_root = env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        Self { endpoint, token, backpressure_tx, broadcast_tx, repo_root }
    }

    pub async fn run(&mut self) -> Result<()> {
        let mut retries = 0u32;
        let max_retries = 5u32;
        loop {
            info!(timestamp = %Utc::now().to_rfc3339(), endpoint = %self.endpoint, "connecting to Yellowstone endpoint");
            match self.connect_and_stream().await {
                Ok(()) => return Ok(()),
                Err(err) => {
                    retries += 1;
                    error!(
                        timestamp = %Utc::now().to_rfc3339(),
                        retry = retries,
                        error = %err,
                        "stream reconnect failed"
                    );
                    if retries >= max_retries {
                        let _ = self.broadcast_tx.send(StreamEvent::Error(format!("max retries exceeded: {err}")));
                        return Err(err);
                    }
                    let delay_ms = 250u64.saturating_mul(2u64.saturating_pow(retries));
                    sleep(Duration::from_millis(delay_ms)).await;
                }
            }
        }
    }

    async fn connect_and_stream(&mut self) -> Result<()> {
        let builder = GeyserGrpcClient::build_from_shared(self.endpoint.clone())
            .context("build Yellowstone client")?;
        let builder = builder
            .x_token(self.token.clone())
            .context("apply Yellowstone x-token")?;
        let mut client = builder
            .connect()
        .await
        .context("connect Yellowstone client")?;

        let mut accounts = HashMap::new();
        for tip in TIP_ACCOUNTS {
            accounts.insert(
                tip.to_string(),
                SubscribeRequestFilterAccounts {
                    account: vec![tip.to_string()],
                    ..Default::default()
                },
            );
        }

        let request = SubscribeRequest {
            slots: HashMap::from([(
                "slot_updates".to_string(),
                SubscribeRequestFilterSlots {
                    filter_by_commitment: Some(true),
                    ..Default::default()
                },
            )]),
            accounts,
            ..Default::default()
        };

        let (mut subscribe_tx, mut stream) = client
            .subscribe()
            .await
            .context("open Yellowstone subscribe stream")?;
        subscribe_tx.send(request).await.context("send Yellowstone subscribe request")?;

        while let Some(update) = stream.next().await {
            let update = update.context("read Yellowstone stream")?;
            match update.update_oneof {
                Some(yellowstone_grpc_proto::geyser::subscribe_update::UpdateOneof::Slot(slot_update)) => {
                    let event = StreamEvent::Slot(SlotEvent {
                        slot: slot_update.slot,
                        ts: Utc::now(),
                    });
                    self.emit_event(event).await?;
                }
                Some(yellowstone_grpc_proto::geyser::subscribe_update::UpdateOneof::Account(account_update)) => {
                    let lamports = account_update
                        .account
                        .as_ref()
                        .map(|account| account.lamports as u64)
                        .unwrap_or_default();
                    let pubkey = account_update
                        .account
                        .as_ref()
                        .and_then(|acct| Pubkey::try_from(acct.pubkey.as_slice()).ok())
                        .map(|p| p.to_string())
                        .unwrap_or_else(|| {
                            account_update
                                .account
                                .as_ref()
                                .map(|acct| hex::encode(&acct.pubkey))
                                .unwrap_or_default()
                        });
                    let event = StreamEvent::AccountUpdate(AccountUpdateEvent {
                        pubkey: pubkey.clone(),
                        slot: account_update.slot,
                        ts: Utc::now(),
                        lamports,
                    });
                    self.write_tip_balance(&pubkey, lamports).await?;
                    self.emit_event(event).await?;
                }
                _ => {}
            }
        }

        Err(anyhow::anyhow!("Yellowstone stream ended unexpectedly"))
    }

    async fn emit_event(&self, event: StreamEvent) -> Result<()> {
        self.backpressure_tx.send(event.clone()).await?;
        let _ = self.broadcast_tx.send(event);
        Ok(())
    }

    async fn write_tip_balance(&self, pubkey: &str, lamports: u64) -> Result<()> {
        let tips_path = self.repo_root.join("logs").join("tips.json");
        let mut tips = if let Ok(raw) = fs::read_to_string(&tips_path).await {
            serde_json::from_str::<serde_json::Value>(&raw).unwrap_or_default()
        } else {
            serde_json::json!({ "accounts": {} })
        };

        if !tips.is_object() {
            tips = serde_json::json!({ "accounts": {} });
        }
        let accounts = tips
            .as_object_mut()
            .and_then(|obj| obj.get_mut("accounts"))
            .and_then(|value| value.as_object_mut())
            .context("tips.json missing accounts object")?;
        accounts.insert(pubkey.to_string(), serde_json::json!({ "lamports": lamports, "updated_at": Utc::now() }));
        fs::write(&tips_path, serde_json::to_vec_pretty(&tips)?).await?;
        Ok(())
    }
}
