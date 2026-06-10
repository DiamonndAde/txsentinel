use anyhow::Result;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::broadcast;
use tracing::{info, warn};
use yellowstone_grpc_client::{ClientTlsConfig, GeyserGrpcClient};
use yellowstone_grpc_proto::geyser::{
    subscribe_update::UpdateOneof, CommitmentLevel, SlotStatus, SubscribeRequest,
    SubscribeRequestFilterSlots,
};

#[derive(Debug, Clone)]
pub struct SlotUpdate {
    pub slot: u64,
    pub parent: Option<u64>,
    pub status: String,
}

#[derive(Clone, Default)]
pub struct SlotState {
    pub current_slot: u64,
    pub current_tps: u64,
    pub leader: String,
}

pub struct SlotMonitor {
    endpoint: String,
    x_token: String,
    state: Arc<Mutex<SlotState>>,
    tx: broadcast::Sender<SlotUpdate>,
}

impl SlotMonitor {
    pub fn new(endpoint: &str, x_token: &str) -> Self {
        let (tx, _) = broadcast::channel(512);
        // Ensure endpoint has https:// prefix for gRPC TLS
        let endpoint = if endpoint.starts_with("https://") {
            endpoint.to_string()
        } else {
            format!("https://{endpoint}")
        };
        Self {
            endpoint,
            x_token: x_token.to_string(),
            state: Arc::new(Mutex::new(SlotState::default())),
            tx,
        }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<SlotUpdate> {
        self.tx.subscribe()
    }

    pub fn state(&self) -> SlotState {
        self.state.lock().unwrap().clone()
    }

    pub async fn run(&self) -> Result<()> {
        info!("Slot monitor starting, endpoint={}", self.endpoint);
        loop {
            let result = tokio::time::timeout(
                Duration::from_secs(15),
                self.connect_and_stream(),
            ).await;
            match result {
                Ok(Ok(())) => info!("Slot stream ended cleanly, reconnecting..."),
                Ok(Err(e)) => warn!("Slot stream error: {e:#}, reconnecting in 3s..."),
                Err(_) => warn!("Slot stream connection timed out after 15s, reconnecting..."),
            }
            tokio::time::sleep(tokio::time::Duration::from_secs(3)).await;
        }
    }

    async fn connect_and_stream(&self) -> Result<()> {
        let mut client = GeyserGrpcClient::build_from_shared(self.endpoint.clone())?
            .x_token(Some(self.x_token.as_str()))?
            .tls_config(ClientTlsConfig::new().with_native_roots())?
            .connect()
            .await?;

        info!("Connected to Yellowstone gRPC at {}", self.endpoint);

        let mut slots = HashMap::new();
        slots.insert(
            "slots".to_string(),
            SubscribeRequestFilterSlots {
                filter_by_commitment: Some(false),
                interslot_updates: Some(false),
            },
        );

        let request = SubscribeRequest {
            slots,
            commitment: Some(CommitmentLevel::Confirmed as i32),
            ..Default::default()
        };

        let (_, mut stream) = client.subscribe_with_request(Some(request)).await?;

        let mut last_tick = std::time::Instant::now();
        let mut slot_count = 0u64;

        while let Some(msg) = tokio_stream::StreamExt::next(&mut stream).await {
            match msg {
                Ok(update) => {
                    if let Some(UpdateOneof::Slot(slot_info)) = update.update_oneof {
                        let slot = slot_info.slot;
                        let status = match slot_info.status() {
                            SlotStatus::SlotProcessed => "processed",
                            SlotStatus::SlotConfirmed => "confirmed",
                            SlotStatus::SlotFinalized => "finalized",
                            _ => "unknown",
                        };

                        slot_count += 1;
                        let elapsed = last_tick.elapsed().as_secs_f64();
                        if elapsed >= 1.0 {
                            let slots_per_sec = slot_count as f64 / elapsed;
                            let approx_tps = (slots_per_sec * 2_500.0) as u64;
                            {
                                let mut s = self.state.lock().unwrap();
                                s.current_slot = slot;
                                s.current_tps = approx_tps;
                            }
                            slot_count = 0;
                            last_tick = std::time::Instant::now();
                        } else {
                            self.state.lock().unwrap().current_slot = slot;
                        }

                        let _ = self.tx.send(SlotUpdate {
                            slot,
                            parent: slot_info.parent,
                            status: status.to_string(),
                        });
                    }
                }
                Err(e) => return Err(e.into()),
            }
        }

        Ok(())
    }
}
