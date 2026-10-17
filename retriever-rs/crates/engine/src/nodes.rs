//! File-only SSH nodes. Sessions carry bounded, identity-checked RPC frames;
//! a cancelled/ambiguous request drops its channel and is never replayed.
use crate::config::{Config, Device};
use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    process::{Child, ChildStdin, ChildStdout},
    sync::Mutex,
};

struct Session {
    child: Child,
    input: ChildStdin,
    output: BufReader<ChildStdout>,
    used: Instant,
}
type Slot = Arc<Mutex<Option<Session>>>;
#[derive(Default)]
pub struct NodePool {
    slots: parking_lot::Mutex<HashMap<String, Slot>>,
}
impl NodePool {
    /// Pipeline only read-only fingerprints; each reply retains its own identity.
    /// Existing version-1 agents already accept consecutive framed requests.
    pub async fn fingerprints(
        &self,
        cfg: &Config,
        d: &Device,
        bodies: Vec<Value>,
    ) -> Result<Vec<std::result::Result<Value, String>>> {
        if bodies.is_empty() || bodies.len() > 8 {
            bail!("Invalid source verification group");
        }
        let key = format!(
            "{}\n{}\n{}\n{}",
            cfg.revision(),
            cfg.ssh_config,
            d.ssh,
            d.agent
        );
        let slot = {
            let mut slots = self.slots.lock();
            if !slots.contains_key(&key) && slots.len() >= 16 {
                slots.retain(|_, slot| Arc::strong_count(slot) > 1);
                if slots.len() >= 16 {
                    bail!("Too many active node connections");
                }
            }
            slots.entry(key).or_default().clone()
        };
        let mut guard = tokio::time::timeout(Duration::from_secs(15), slot.lock())
            .await
            .context("Node is busy; try again shortly")?;
        let mut session = if let Some(mut s) = guard.take() {
            if s.used.elapsed() < Duration::from_secs(60) && s.child.try_wait()?.is_none() {
                s
            } else {
                Self::connect(cfg, d)?
            }
        } else {
            Self::connect(cfg, d)?
        };
        let mut identities = Vec::with_capacity(bodies.len());
        let mut bytes = Vec::new();
        for body in bodies {
            let id = uuid::Uuid::new_v4().to_string();
            let request = retriever_protocol::Request {
                version: 1,
                id: id.clone(),
                action: "fingerprint".into(),
                body,
            };
            let frame = serde_json::to_vec(&request)?;
            if frame.len() >= retriever_protocol::MAX_MESSAGE
                || bytes.len() + frame.len() + 1 > retriever_protocol::MAX_MESSAGE
            {
                bail!("Verification group exceeds protocol budget");
            }
            bytes.extend_from_slice(&frame);
            bytes.push(b'\n');
            identities.push(id);
        }
        let values = tokio::time::timeout(
            Duration::from_secs(
                cfg.limits["ssh_timeout"]
                    .as_u64()
                    .unwrap_or(120)
                    .clamp(1, 120),
            ),
            async {
                session.input.write_all(&bytes).await?;
                session.input.flush().await?;
                fingerprint_replies(&mut session.output, &identities).await
            },
        )
        .await
        .context("Source verification deadline exceeded")??;
        session.used = Instant::now();
        *guard = Some(session);
        Ok(values)
    }
    pub async fn call(&self, cfg: &Config, d: &Device, action: &str, body: Value) -> Result<Value> {
        let mut key = format!(
            "{}\n{}\n{}\n{}",
            cfg.revision(),
            cfg.ssh_config,
            d.ssh,
            d.agent
        );
        // Long copies must not occupy the channel used by browsing or source
        // verification. At most one copy job is admitted by the coordinator.
        if action.starts_with("transfer_") { key.push_str("\ntransfer"); }
        let slot = {
            let mut slots = self.slots.lock();
            if !slots.contains_key(&key) && slots.len() >= 16 {
                slots.retain(|_, slot| Arc::strong_count(slot) > 1);
                if slots.len() >= 16 {
                    bail!("Too many active node connections");
                }
            }
            slots.entry(key).or_default().clone()
        };
        let mut guard = tokio::time::timeout(Duration::from_secs(15), slot.lock())
            .await
            .context("Node is busy; try again shortly")?;
        // Taking ownership makes cancellation kill the process instead of leaving
        // an unread response available for the next request.
        let mut session = if let Some(mut s) = guard.take() {
            if s.used.elapsed() < Duration::from_secs(60) && s.child.try_wait()?.is_none() {
                s
            } else {
                Self::connect(cfg, d)?
            }
        } else {
            Self::connect(cfg, d)?
        };
        let id = uuid::Uuid::new_v4().to_string();
        let request = retriever_protocol::Request {
            version: 1,
            id: id.clone(),
            action: action.into(),
            body,
        };
        let mut bytes = serde_json::to_vec(&request)?;
        if bytes.len() >= retriever_protocol::MAX_MESSAGE {
            bail!("Agent request exceeds budget");
        }
        bytes.push(b'\n');
        let deadline = if matches!(action,"transfer_local_copy"|"transfer_finish") {24*3600} else {cfg.limits["ssh_timeout"].as_u64().unwrap_or(120).clamp(1,120)};
        let response=tokio::time::timeout(Duration::from_secs(deadline),async {
            session.input.write_all(&bytes).await?;session.input.flush().await?;
            let mut collector=retriever_protocol::ResponseCollector::new(&id);
            loop{let mut frame=Vec::new();(&mut session.output).take((retriever_protocol::MAX_MESSAGE+1)as u64).read_until(b'\n',&mut frame).await?;if frame.is_empty(){bail!("SSH node disconnected before replying. Check its saved alias, pinned host key, login and agent path. Inspect any pending file operation before retrying.");}if let Some(r)=collector.push(&frame)? {break Ok::<_,anyhow::Error>(r);}}
        }).await.context("Node deadline exceeded; inspect any file operation before retrying")??;
        if response.version != 1 || response.id != id {
            bail!("Node response identity mismatch");
        }
        // Old agents may exit after one request. They are allowed to answer but
        // are not pooled unless the probe explicitly advertises sessions.
        if action == "probe"
            && response
                .data
                .as_ref()
                .is_some_and(|v| v["node_protocol"] == 1)
            || action != "probe"
        {
            session.used = Instant::now();
            *guard = Some(session);
        }
        if !response.ok {
            bail!(
                "{}",
                response
                    .error
                    .unwrap_or_else(|| "Node request failed".into())
            );
        }
        response.data.context("Node omitted result")
    }
    fn connect(cfg: &Config, d: &Device) -> Result<Session> {
        let mut cmd = tokio::process::Command::new("ssh");
        if !cfg.ssh_config.is_empty() {
            cmd.arg("-F").arg(crate::paths::expand(&cfg.ssh_config));
        }
        #[cfg(windows)]
        cmd.creation_flags(0x08000000);
        cmd.args([
            "-o",
            "BatchMode=yes",
            "-o",
            "StrictHostKeyChecking=yes",
            "-o",
            "ConnectTimeout=8",
            "-o",
            "ServerAliveInterval=15",
            "-o",
            "ServerAliveCountMax=2",
            "-T",
            &d.ssh,
        ]);
        cmd.arg(if d.agent == "retriever-agent" {
            d.agent.clone()
        } else {
            format!("\"{}\"", d.agent)
        })
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true);
        let mut child = cmd.spawn().context("Could not start SSH")?;
        let input = child.stdin.take().unwrap();
        let output = BufReader::new(child.stdout.take().unwrap());
        Ok(Session {
            child,
            input,
            output,
            used: Instant::now(),
        })
    }
}

async fn fingerprint_replies<R: tokio::io::AsyncBufRead + Unpin>(
    reader: &mut R,
    identities: &[String],
) -> Result<Vec<std::result::Result<Value, String>>> {
    let mut values = Vec::with_capacity(identities.len());
    for id in identities {
        // Fingerprints contain only a bounded path, digest and stat fields.
        // They never require the large extraction-fragment extension.
        let mut frame = Vec::new();
        (&mut *reader)
            .take(32769)
            .read_until(b'\n', &mut frame)
            .await?;
        if frame.len() > 32768 || !frame.ends_with(b"\n") {
            bail!("Invalid source verification frame");
        }
        let response: retriever_protocol::Response = serde_json::from_slice(&frame)?;
        if response.version != 1 || response.id != *id || response.fragment.is_some() {
            bail!("Source verification response identity mismatch");
        }
        values.push(if response.ok {
            Ok(response.data.context("Node omitted fingerprint")?)
        } else {
            Err(response
                .error
                .unwrap_or_else(|| "Source verification failed".into()))
        });
    }
    Ok(values)
}

#[cfg(test)]
mod fingerprint_pipeline_tests {
    use super::*;
    fn reply(id: &str, ok: bool) -> String {
        json!({"version":1,"id":id,"ok":ok,"data":if ok{json!({"digest":"unchanged"})}else{Value::Null},"error":if ok{Value::Null}else{json!("unavailable")}}).to_string()+"\n"
    }
    #[tokio::test]
    async fn preserves_each_identity_and_source_error() {
        let bytes = reply("a", true) + &reply("b", false) + &reply("c", true);
        let mut reader = BufReader::new(bytes.as_bytes());
        let result = fingerprint_replies(&mut reader, &["a".into(), "b".into(), "c".into()])
            .await
            .unwrap();
        assert_eq!(result[0].as_ref().unwrap()["digest"], "unchanged");
        assert_eq!(result[1].as_ref().unwrap_err(), "unavailable");
        assert!(result[2].is_ok());
    }
    #[tokio::test]
    async fn rejects_reordered_duplicate_and_truncated_replies() {
        for bytes in [
            reply("b", true) + &reply("a", true),
            reply("a", true) + &reply("a", true),
            reply("a", true),
        ] {
            assert!(
                fingerprint_replies(
                    &mut BufReader::new(bytes.as_bytes()),
                    &["a".into(), "b".into()]
                )
                .await
                .is_err()
            );
        }
    }
    #[tokio::test]
    async fn bounds_untrusted_fingerprint_output() {
        let bytes = "x".repeat(32769);
        assert!(
            fingerprint_replies(&mut BufReader::new(bytes.as_bytes()), &["a".into()])
                .await
                .is_err()
        );
    }
}

pub fn discovered(value: &Value) -> Value {
    let mut nodes = Vec::new();
    for (own, peer) in std::iter::once((true, &value["Self"])).chain(
        value["Peer"]
            .as_object()
            .into_iter()
            .flat_map(|p| p.values().map(|v| (false, v))),
    ) {
        let os = peer["OS"].as_str().unwrap_or("");
        if !["windows", "macOS", "linux"].contains(&os) {
            continue;
        }
        let Some(address) = peer["TailscaleIPs"].as_array().and_then(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .find(|s| s.parse::<std::net::Ipv4Addr>().is_ok())
        }) else {
            continue;
        };
        nodes.push(json!({"name":peer["HostName"],"dns":peer["DNSName"],"address":address,"platform":if os=="macOS"{"macos"}else{os},"online":peer["Online"].as_bool().unwrap_or(false),"local":own,"paired":false}));
    }
    nodes.sort_by_key(|v| v["name"].as_str().unwrap_or("").to_lowercase());
    json!({"nodes":nodes,"discovery_only":true,"ssh_required":true,"health_collection":false})
}
pub async fn discover() -> Result<Value> {
    let binary = if cfg!(windows) {
        std::env::var_os("ProgramFiles")
            .map(std::path::PathBuf::from)
            .map(|p| p.join("Tailscale/tailscale.exe"))
            .filter(|p| p.is_file())
            .unwrap_or_else(|| "tailscale".into())
    } else if cfg!(target_os = "macos")
        && std::path::Path::new("/Applications/Tailscale.app/Contents/MacOS/Tailscale").is_file()
    {
        "/Applications/Tailscale.app/Contents/MacOS/Tailscale".into()
    } else {
        "tailscale".into()
    };
    let mut child = tokio::process::Command::new(binary)
        .args(["status", "--json"])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .context("Tailscale CLI is unavailable; saved SSH aliases still work")?;
    tokio::time::timeout(Duration::from_secs(6), async {
        let mut data = Vec::new();
        child
            .stdout
            .take()
            .unwrap()
            .take(1024 * 1024 + 1)
            .read_to_end(&mut data)
            .await?;
        if data.len() > 1024 * 1024 {
            bail!("Discovery response exceeds budget");
        }
        if !child.wait().await?.success() {
            bail!("Tailscale is not connected");
        }
        Ok(discovered(&serde_json::from_slice(&data)?))
    })
    .await
    .context("Tailscale discovery timed out")?
}
