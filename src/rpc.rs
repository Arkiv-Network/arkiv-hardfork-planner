use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;

use serde_json::{json, Value};
use tokio::time::sleep;

use crate::quantity::parse_quantity;
use crate::store::ScheduleStore;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RpcStatusSnapshot {
    pub verified: bool,
    pub chain_id: Option<u64>,
    pub error: Option<String>,
}

#[derive(Debug, Default)]
pub struct RpcStatus {
    inner: Mutex<RpcStatusSnapshot>,
}

impl RpcStatus {
    pub fn mark_verified(&self, chain_id: u64) {
        let mut inner = self.inner.lock().expect("rpc status lock poisoned");
        inner.verified = true;
        inner.chain_id = Some(chain_id);
        inner.error = None;
    }

    pub fn mark_error<S>(&self, message: S)
    where
        S: Into<String>,
    {
        let mut inner = self.inner.lock().expect("rpc status lock poisoned");
        inner.error = Some(message.into());
    }

    pub fn snapshot(&self) -> RpcStatusSnapshot {
        self.inner.lock().expect("rpc status lock poisoned").clone()
    }
}

pub async fn poll_loop(
    store: Arc<ScheduleStore>,
    status: Arc<RpcStatus>,
    client: reqwest::Client,
    rpc_url: String,
    timeout: Duration,
    interval: Duration,
) {
    let expected_chain_id = store.snapshot().chain_id;

    loop {
        if !status.snapshot().verified {
            match fetch_chain_id(&client, &rpc_url, timeout).await {
                Ok(actual_chain_id) if actual_chain_id == expected_chain_id => {
                    status.mark_verified(actual_chain_id);
                    println!(
                        "{}",
                        json!({
                            "message": "rpc chain id verified",
                            "chainId": actual_chain_id,
                            "rpcUrl": rpc_url,
                        })
                    );
                }
                Ok(actual_chain_id) => {
                    let message = format!(
                        "chain id mismatch; expected {expected_chain_id}, got {actual_chain_id}"
                    );
                    status.mark_error(&message);
                    eprintln!(
                        "{}",
                        json!({
                            "message": "rpc chain id mismatch",
                            "expected": expected_chain_id,
                            "actual": actual_chain_id,
                            "rpcUrl": rpc_url,
                        })
                    );
                    sleep(interval).await;
                    continue;
                }
                Err(message) => {
                    status.mark_error(format!("chain id check failed: {message}"));
                    eprintln!(
                        "{}",
                        json!({
                            "message": "rpc chain id check failed",
                            "rpcUrl": rpc_url,
                            "error": message,
                        })
                    );
                    sleep(interval).await;
                    continue;
                }
            }
        }

        match fetch_block_number(&client, &rpc_url, timeout).await {
            Ok(block) => {
                if store.set_current_block(block) {
                    let snapshot = store.snapshot();
                    println!(
                        "{}",
                        json!({
                            "message": "current block updated",
                            "block": block.to_string(),
                            "version": snapshot.version,
                            "chainId": snapshot.chain_id,
                            "hash": snapshot.hash,
                        })
                    );
                }
            }
            Err(message) => {
                status.mark_error(format!("block poll failed: {message}"));
                eprintln!(
                    "{}",
                    json!({ "message": "rpc poll failed", "error": message })
                );
            }
        }
        sleep(interval).await;
    }
}

pub async fn fetch_block_number(
    client: &reqwest::Client,
    rpc_url: &str,
    timeout: Duration,
) -> Result<u64, String> {
    rpc_quantity(client, rpc_url, timeout, "eth_blockNumber").await
}

pub async fn fetch_chain_id(
    client: &reqwest::Client,
    rpc_url: &str,
    timeout: Duration,
) -> Result<u64, String> {
    rpc_quantity(client, rpc_url, timeout, "eth_chainId").await
}

async fn rpc_quantity(
    client: &reqwest::Client,
    rpc_url: &str,
    timeout: Duration,
    method: &str,
) -> Result<u64, String> {
    let request_body = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": method,
        "params": [],
    });

    let response = tokio::time::timeout(timeout, client.post(rpc_url).json(&request_body).send())
        .await
        .map_err(|_| "rpc request timed out".to_string())?
        .map_err(|error| format!("rpc request failed: {error}"))?;

    let status = response.status();
    let value: Value = response
        .json()
        .await
        .map_err(|error| format!("rpc decode failed: {error}"))?;

    if !status.is_success() {
        return Err(format!("rpc http status {status}"));
    }

    if let Some(error_value) = value.get("error").filter(|v| !v.is_null()) {
        return Err(format!("rpc error: {error_value}"));
    }

    let result = value
        .get("result")
        .and_then(Value::as_str)
        .ok_or_else(|| "rpc response missing string result".to_string())?;

    parse_quantity(result).ok_or_else(|| format!("rpc returned non-quantity result: {result}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ScheduleDocument, ScheduleEntry};

    fn entry() -> ScheduleEntry {
        ScheduleEntry {
            activation_block: 0,
            min_base_fee_per_gas: "440000000".to_string(),
            elasticity_multiplier: 2,
            base_fee_max_change_denominator: 8,
            max_block_gas_limit: "30000000".to_string(),
        }
    }

    #[test]
    fn parse_quantity_reads_eth_block_number_result() {
        assert_eq!(parse_quantity("0x10"), Some(16));
        assert_eq!(parse_quantity("0x1e8480"), Some(2_000_000));
    }

    #[test]
    fn document_with_current_block_serializes_for_gating() {
        let doc = ScheduleDocument {
            chain_id: 42069,
            version: 1,
            current_block: Some(2_000_000),
            schedule: vec![entry()],
        };
        let serialized = crate::model::canonicalize(&doc);
        assert!(serialized.contains("\"currentBlock\": 2000000"));
    }

    #[test]
    fn rpc_status_tracks_verification_and_errors() {
        let status = RpcStatus::default();
        assert_eq!(status.snapshot(), RpcStatusSnapshot::default());

        status.mark_error("not ready");
        assert_eq!(status.snapshot().error.as_deref(), Some("not ready"));

        status.mark_verified(42069);
        let snapshot = status.snapshot();
        assert!(snapshot.verified);
        assert_eq!(snapshot.chain_id, Some(42069));
        assert_eq!(snapshot.error, None);
    }
}
