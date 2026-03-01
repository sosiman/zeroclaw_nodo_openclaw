use anyhow::Result;
use futures_util::{SinkExt, StreamExt};
use serde_json::json;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use std::sync::Arc;
use tokio::sync::{mpsc, Semaphore};

use crate::worker::identity::NodeIdentity;
use crate::worker::protocol::PROTOCOL_VERSION;
use crate::worker::sandbox::{SandboxManager, ExecutionConfig};
use crate::Config;

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

pub struct WsClient {
    hub_url: String,
    identity: tokio::sync::Mutex<NodeIdentity>,
    config: Config,
    sandbox: Arc<SandboxManager>,
    store: Arc<crate::worker::store::JobStore>,
    
    // Observability metrics
    jobs_ok: Arc<AtomicU64>,
    jobs_fail: Arc<AtomicU64>,
    startup_time: SystemTime,

    // Maintenance state
    is_draining: Arc<AtomicBool>,  // no new jobs, finish current ones
    is_disabled: Arc<AtomicBool>,  // hard stop, disconnect from hub
}

impl WsClient {
    pub fn new(hub_url: String, identity: NodeIdentity, config: Config, store: Arc<crate::worker::store::JobStore>) -> Self {
        Self {
            hub_url,
            identity: tokio::sync::Mutex::new(identity),
            config,
            sandbox: Arc::new(SandboxManager::new(ExecutionConfig::default())),
            store,
            jobs_ok: Arc::new(AtomicU64::new(0)),
            jobs_fail: Arc::new(AtomicU64::new(0)),
            startup_time: SystemTime::now(),
            is_draining: Arc::new(AtomicBool::new(false)),
            is_disabled: Arc::new(AtomicBool::new(false)),
        }
    }

    pub async fn connect_and_run(&self) -> Result<()> {
        let mut retry_count = 0;
        let max_backoff_sec = 30;
        let base_backoff_sec = 2;

        loop {
            match self.connect_inner().await {
                Ok(_) => {
                    println!("WebSocket disconnected cleanly. Reconnecting in 15 seconds to allow pairing...");
                    tokio::time::sleep(Duration::from_secs(15)).await;
                    retry_count = 0; // Reset on clean disconnect if we were connected for a while
                }
                Err(e) => {
                    println!("WebSocket connection dropped: {}. Retrying...", e);
                    
                    let backoff = (base_backoff_sec * 2_u64.pow(retry_count.min(6) as u32)).min(max_backoff_sec);
                    let jitter = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as u64 % 2000;
                    let wait_time = Duration::from_secs(backoff) + Duration::from_millis(jitter);
                    
                    println!("Waiting {:?} before next attempt...", wait_time);
                    tokio::time::sleep(wait_time).await;
                    retry_count += 1;
                }
            }
        }
    }

    async fn connect_inner(&self) -> Result<()> {
        let (mut ws_stream, _) = connect_async(&self.hub_url).await?;
        println!("Connected to {}", self.hub_url);
        let (tx, mut rx) = mpsc::unbounded_channel::<Message>();
        let semaphore = Arc::new(Semaphore::new(self.sandbox.get_limits().max_concurrent_jobs));


        let mut ping_interval = tokio::time::interval(Duration::from_secs(30));

        loop {
            tokio::select! {
                msg = ws_stream.next() => {
                    match msg {
                        Some(Ok(Message::Text(text))) => {
                    self.handle_message(&text, tx.clone(), semaphore.clone()).await?;
                        }
                        Some(Ok(Message::Ping(data))) => {
                            ws_stream.send(Message::Pong(data)).await?;
                        }
                        Some(Ok(Message::Close(_))) => {
                            println!("Received close message");
                            break;
                        }
                        Some(Err(e)) => {
                            return Err(e.into());
                        }
                        None => {
                            println!("WebSocket stream ended");
                            break;
                        }
                        _ => {}
                    }
                }
                Some(msg) = rx.recv() => {
                    ws_stream.send(msg).await?;
                }
                _ = ping_interval.tick() => {
                    // Send a standard WebSocket ping heartbeat
                    ws_stream.send(Message::Ping(vec![].into())).await?;
                }
            }
        }

        Ok(())
    }

    async fn handle_message(
        &self,
        text: &str,
        tx: mpsc::UnboundedSender<Message>,
        semaphore: Arc<Semaphore>,
    ) -> Result<()> {
        let v: serde_json::Value = serde_json::from_str(text)?;

        // Handle the initial connect.challenge
        if v["type"] == "event" && v["event"] == "connect.challenge" {
            println!("Received connect.challenge!");
            let nonce = v["payload"]["nonce"].as_str().unwrap_or("");
            
            // Build ConnectParams
            let identity = self.identity.lock().await;

            let signed_at_ms = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_millis() as u64;

            let auth_token = std::env::var("OPENCLAW_GATEWAY_TOKEN").unwrap_or_default();

            // Generate signature for "v2|device_id|client_id|mode|role|scopes|signed_at|token|nonce"
            let payload_string = format!(
                "v2|{}|node-host|node|node||{}|{}|{}",
                identity.node_id,
                signed_at_ms,
                auth_token,
                nonce
            );

            let signature = identity.sign_payload(payload_string.as_bytes());

            let connect_req = json!({
                "type": "req",
                "method": "connect",
                "id": uuid::Uuid::new_v4().to_string(),
                "params": {
                    "minProtocol": PROTOCOL_VERSION,
                    "maxProtocol": PROTOCOL_VERSION,
                    "auth": {
                        "token": auth_token
                    },
                    "client": {
                        "id": "node-host",
                        "version": env!("CARGO_PKG_VERSION"),
                        "platform": std::env::consts::OS,
                        "mode": "node"
                    },
                    "role": "node",
                    "device": {
                        "id": identity.node_id,
                        "publicKey": identity.public_key_base64(),
                        "signature": signature,
                        "signedAt": signed_at_ms,
                        "nonce": nonce
                    },
                    "caps": ["can_run", "can_invoke", "sandbox_profile_default"]
                }
            });
            drop(identity); // Release the lock

            let _ = tx.send(Message::Text(connect_req.to_string().into()));
            println!("Sent connect response with signature");
        } else if v["type"] == "req" {
            let method = v["method"].as_str().unwrap_or("");
            let req_id = v["id"].as_str().unwrap_or("").to_string();

            if method == "nodes.run" {
                println!("Handling nodes.run: {}", text);

                // Refuse new work when draining or disabled
                if self.is_draining.load(Ordering::Relaxed) || self.is_disabled.load(Ordering::Relaxed) {
                    let reason = if self.is_disabled.load(Ordering::Relaxed) { "node_disabled" } else { "node_draining" };
                    let err_res = json!({
                        "type": "res",
                        "id": req_id,
                        "ok": false,
                        "error": {
                            "code": reason,
                            "message": "Node is not accepting new jobs right now"
                        }
                    });
                    let _ = tx.send(Message::Text(err_res.to_string().into()));
                    return Ok(());
                }

                let command = v["params"]["command"].as_str().unwrap_or("");
                let args: Vec<String> = v["params"]["args"]
                    .as_array()
                    .map(|a| a.iter().filter_map(|i| i.as_str().map(String::from)).collect())
                    .unwrap_or_default();

                let command = command.to_string();
                let sandbox = Arc::clone(&self.sandbox);
                let tx_clone = tx.clone();
                let req_id_clone = req_id.clone();
                let store = Arc::clone(&self.store);
                let jobs_ok = Arc::clone(&self.jobs_ok);
                let jobs_fail = Arc::clone(&self.jobs_fail);

                if let Ok(Some(existing_job)) = store.get_job(&req_id) {
                    if existing_job.status == "success" || existing_job.status == "error" {
                        let res = serde_json::json!({
                            "type": "res",
                            "id": req_id,
                            "ok": existing_job.status == "success",
                            "payload": {
                                "status": existing_job.status,
                                "stdout": existing_job.stdout.unwrap_or_default(),
                                "stderr": existing_job.stderr.unwrap_or_default(),
                                "exitCode": existing_job.exit_code.unwrap_or(-1)
                            }
                        });
                        let _ = tx.send(Message::Text(res.to_string().into()));
                        return Ok(());
                    } else {
                        // Job is already running, skip re-execution
                        return Ok(());
                    }
                }

                let started_at = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
                let _ = store.insert_job(&req_id, &command, &serde_json::to_string(&args).unwrap_or_default(), started_at);
                let store_clone = Arc::clone(&store);

                
                tokio::spawn(async move {
                    if let Err(e) = sandbox.validate_command(&command) {
                    let ended_at = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
                    let _ = store_clone.update_job_error(&req_id_clone, ended_at, &e.to_string());
                    let err_res = json!({
                        "type": "res",
                        "id": req_id_clone,
                        "ok": false,
                        "error": {
                            "code": "sandbox_error",
                            "message": e.to_string()
                        }
                    });
                    let _ = tx_clone.send(Message::Text(err_res.to_string().into()));
                    return;
                }

                let _permit = match semaphore.acquire().await {
                    Ok(p) => p,
                    Err(_) => return,
                };

                use std::process::Stdio;
                use tokio::io::AsyncReadExt;

                let (payload, exit_code, stdout_len, stderr_len) = match tokio::process::Command::new(command)
                    .args(&args)
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped())
                    .spawn()
                {
                    Ok(mut child) => {
                        let limits = sandbox.get_limits();
                        let max_bytes = limits.max_output_bytes_total;
                        let max_duration = std::time::Duration::from_secs(limits.max_job_duration_sec);

                        let mut stdout = child.stdout.take().unwrap();
                        let mut stderr = child.stderr.take().unwrap();

                        let mut stdout_buf = Vec::new();
                        let mut stderr_buf = Vec::new();

                        let read_task = async {
                            let mut out_chunk = [0; 4096];
                            let mut err_chunk = [0; 4096];
                            
                            loop {
                                tokio::select! {
                                    Ok(n) = stdout.read(&mut out_chunk) => {
                                        if n == 0 { break; }
                                        if stdout_buf.len() + n > max_bytes {
                                            stdout_buf.extend_from_slice(&out_chunk[..max_bytes.saturating_sub(stdout_buf.len())]);
                                            stdout_buf.extend_from_slice(b"\n...[TRUNCATED BY SANDBOX LIMIT]...");
                                            break;
                                        }
                                        stdout_buf.extend_from_slice(&out_chunk[..n]);
                                    }
                                    Ok(n) = stderr.read(&mut err_chunk) => {
                                        if n == 0 { break; }
                                        if stderr_buf.len() + n > max_bytes {
                                            stderr_buf.extend_from_slice(&err_chunk[..max_bytes.saturating_sub(stderr_buf.len())]);
                                            stderr_buf.extend_from_slice(b"\n...[TRUNCATED BY SANDBOX LIMIT]...");
                                            break;
                                        }
                                        stderr_buf.extend_from_slice(&err_chunk[..n]);
                                    }
                                    else => break,
                                }
                            }
                        };

                        match tokio::time::timeout(max_duration, async { tokio::join!(child.wait(), read_task) }).await {
                            Ok((Ok(status), _)) => {
                                let stdout_str = String::from_utf8_lossy(&stdout_buf).into_owned();
                                let stderr_str = String::from_utf8_lossy(&stderr_buf).into_owned();
                                let exit_code = status.code().unwrap_or(-1);
                                (json!({
                                    "status": "success",
                                    "stdout": stdout_str,
                                    "stderr": stderr_str,
                                    "exitCode": exit_code
                                }), exit_code, stdout_buf.len(), stderr_buf.len())
                            }
                            Ok((Err(e), _)) => {
                                let err_msg = format!("Process execution failed: {}", e);
                                (-1, stdout_buf.len(), stderr_buf.len());
                                (json!({
                                    "status": "error",
                                    "error": err_msg
                                }), -1, stdout_buf.len(), stderr_buf.len())
                            }
                            Err(_) => {
                                let _ = child.kill().await;
                                let err_msg = "Job timed out and was killed by the sandbox.";
                                (json!({
                                    "status": "error",
                                    "error": err_msg
                                }), -1, stdout_buf.len(), stderr_buf.len())
                            }
                        }
                    },
                    Err(e) => (json!({
                        "status": "error",
                        "error": e.to_string()
                    }), -1, 0, 0),
                };

                if exit_code == 0 {
                    jobs_ok.fetch_add(1, Ordering::Relaxed);
                } else {
                    jobs_fail.fetch_add(1, Ordering::Relaxed);
                }

                // Update the store
                let store_status = if exit_code == 0 { "success" } else { "error" };
                if let Err(e) = store_clone.complete_job(
                    &req_id_clone,
                    store_status,
                    exit_code,
                    stdout_len as i64,
                    stderr_len as i64,
                ) {
                    eprintln!("Failed to complete job in job store: {}", e);
                }

                let ok = payload.get("status").and_then(|s| s.as_str()) == Some("success");
                let res = json!({
                    "type": "res",
                    "id": req_id_clone,
                    "ok": ok,
                    "payload": payload
                });

                let _ = tx_clone.send(Message::Text(res.to_string().into()));
                });
            } else if method == "nodes.rotate_key" {
                let mut identity = self.identity.lock().await;
                let id_path = std::path::PathBuf::from("/var/lib/zeroclaw/node.json");
                let id_path = if cfg!(unix) { id_path } else { std::path::PathBuf::from("./.zeroclaw_node.json") };
                
                let ok = match identity.rotate_key(&id_path) {
                    Ok(_) => true,
                    Err(e) => {
                        println!("Failed to rotate key: {}", e);
                        false
                    }
                };
                drop(identity);

                let res = json!({
                    "type": "res",
                    "id": req_id,
                    "ok": ok,
                    "payload": {
                        "message": if ok { "Key rotated successfully" } else { "Failed to rotate key" }
                    }
                });

                let _ = tx.send(Message::Text(res.to_string().into()));
            } else if method == "nodes.invoke" {
                println!("Handling nodes.invoke: {}", text);
                
                let res = json!({
                    "type": "res",
                    "id": req_id,
                    "ok": true,
                    "payload": {
                        "status": "success",
                        "result": "Invoked successfully"
                    }
                });
                let _ = tx.send(Message::Text(res.to_string().into()));
            } else if method == "nodes.stats" {
                let uptime_sec = self.startup_time.elapsed().unwrap_or(std::time::Duration::from_secs(0)).as_secs();
                let jobs_ok = self.jobs_ok.load(Ordering::Relaxed);
                let jobs_fail = self.jobs_fail.load(Ordering::Relaxed);

                let res = json!({
                    "id": req_id,
                    "type": "res",
                    "ok": true,
                    "payload": {
                        "uptime_sec": uptime_sec,
                        "jobs_ok": jobs_ok,
                        "jobs_fail": jobs_fail,
                        "draining": self.is_draining.load(Ordering::Relaxed),
                        "disabled": self.is_disabled.load(Ordering::Relaxed),
                        "version": env!("CARGO_PKG_VERSION")
                    }
                });

                let _ = tx.send(Message::Text(res.to_string().into()));
            } else if method == "nodes.drain" {
                // Start draining: stop accepting new jobs, finish active ones
                self.is_draining.store(true, Ordering::Relaxed);
                println!("Node entering DRAIN mode — no new jobs will be accepted");

                let res = json!({
                    "id": req_id,
                    "type": "res",
                    "ok": true,
                    "payload": { "message": "Node is now draining", "draining": true }
                });
                let _ = tx.send(Message::Text(res.to_string().into()));

            } else if method == "nodes.disable" {
                // Hard disable: ACK then signal the connect_inner loop to break
                self.is_disabled.store(true, Ordering::Relaxed);
                println!("Node DISABLED by Hub — disconnecting");

                let res = json!({
                    "id": req_id,
                    "type": "res",
                    "ok": true,
                    "payload": { "message": "Node is disabled and disconnecting" }
                });
                let _ = tx.send(Message::Text(res.to_string().into()));
                // Return Err to break connect_inner and stop reconnect loop
                return Err(anyhow::anyhow!("node_disabled"));

            } else {
                println!("Unhandled req method: {}", method);
            }
        } else if v["type"] == "res" {
            println!("Received response: {}", text);
        } else {
            println!("Unknown method: {}", text);
        }

        Ok(())
    }
    
    fn increment_metric(metric: &AtomicU64) {
        metric.fetch_add(1, Ordering::Relaxed);
    }
}
