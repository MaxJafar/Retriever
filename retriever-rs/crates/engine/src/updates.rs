//! A local desktop can stop its own idle coordinator for an installer.
//! The shared pairing token is deliberately insufficient for this operation.
use crate::engine::Engine;
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::{fs, io::Write, path::Path, sync::atomic::Ordering};
use subtle::ConstantTimeEq;

pub fn local_update_token(state: &Path) -> Result<String> {
    let path = state.join("update-token");
    if !path.exists() {
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        match options.open(&path) {
            Ok(mut file) => {
                write!(
                    file,
                    "{}{}",
                    uuid::Uuid::new_v4().simple(),
                    uuid::Uuid::new_v4().simple()
                )?;
                file.sync_all()?;
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error.into()),
        }
    }
    let token = fs::read_to_string(path)?;
    ensure!(
        token.len() == 64 && token.bytes().all(|b| b.is_ascii_hexdigit()),
        "Invalid local update token"
    );
    Ok(token)
}
impl Engine {
    pub async fn stop_for_update(&self, token: &str, expected_version: &str) -> Result<Value> {
        ensure!(
            bool::from(token.as_bytes().ct_eq(self.update_token.as_bytes())),
            "Local update authorization is required"
        );
        ensure!(
            expected_version == env!("CARGO_PKG_VERSION"),
            "conflict: The running engine changed; check the update again"
        );
        let _runtime = self
            .runtime_gate
            .try_write()
            .context("conflict: Wait for the current request before installing")?;
        let _mutation = self
            .mutation
            .try_lock()
            .context("conflict: Wait for the current file operation before installing")?;
        ensure!(
            !self.syncing.load(Ordering::SeqCst),
            "conflict: Finish or pause indexing before installing"
        );
        ensure!(
            self.transfers.lock().is_empty(),
            "conflict: Finish file transfers before installing"
        );
        ensure!(
            self.disk.status()?["active"].is_null(),
            "conflict: Finish the Disk Capacity scan before installing"
        );
        ensure!(
            !self.burner.is_active(),
            "conflict: Finish the Disk Burner task before installing"
        );
        let db = self.store.db.lock();
        let busy: i64 = db.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |r| r.get(0))?;
        ensure!(
            busy == 0,
            "conflict: The library checkpoint is busy; retry after the current operation"
        );
        self.shutdown.send_replace(true);
        Ok(
            json!({"stopping":true,"checkpointed":true,"version":env!("CARGO_PKG_VERSION"),"metadata_preserved":true}),
        )
    }
}
