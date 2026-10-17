//! Saved drive telemetry, parsing, history and alerts. Windows collection is
//! blocked independently of this module. A subprocess cannot isolate a kernel
//! driver failure. Unknown telemetry is deliberately not a passing result.
use anyhow::{Context, Result, bail};
use parking_lot::Mutex;
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

pub struct DriveHealth {
    path: PathBuf,
    active: AtomicBool,
    last_attempt: Mutex<f64>,
    error: Mutex<String>,
}
fn now() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}
fn connect(path: &Path) -> Result<Connection> {
    let db = Connection::open(path)?;
    db.busy_timeout(Duration::from_secs(3))?;
    db.execute_batch(
        "PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL; PRAGMA cache_size=-2048;",
    )?;
    Ok(db)
}
impl DriveHealth {
    pub fn open(state: &Path) -> Result<Arc<Self>> {
        let path = state.join("drive-health.sqlite");
        crate::paths::reject_links(&path)?;
        connect(&path)?.execute_batch("CREATE TABLE IF NOT EXISTS health_samples(id INTEGER PRIMARY KEY,drive TEXT NOT NULL,time REAL NOT NULL,data TEXT NOT NULL); CREATE INDEX IF NOT EXISTS health_time ON health_samples(drive,time DESC); CREATE TABLE IF NOT EXISTS health_events(id INTEGER PRIMARY KEY,drive TEXT NOT NULL,time REAL NOT NULL,data TEXT NOT NULL); CREATE TABLE IF NOT EXISTS health_settings(id INTEGER PRIMARY KEY CHECK(id=1),data TEXT NOT NULL); INSERT OR IGNORE INTO health_settings VALUES(1,'{\"interval_minutes\":0,\"temperature_c\":70,\"remaining_life_percent\":10}'); CREATE TABLE IF NOT EXISTS health_latest(drive TEXT PRIMARY KEY,time REAL NOT NULL,data TEXT NOT NULL,present INTEGER NOT NULL DEFAULT 1);")?;
        Ok(Arc::new(Self {
            path,
            active: AtomicBool::new(false),
            last_attempt: Mutex::new(0.),
            error: Mutex::new(String::new()),
        }))
    }
    pub fn settings(&self) -> Result<Value> {
        Ok(serde_json::from_str(&connect(&self.path)?.query_row(
            "SELECT data FROM health_settings WHERE id=1",
            [],
            |r| r.get::<_, String>(0),
        )?)?)
    }
    pub fn configure(&self, body: &Value) -> Result<Value> {
        let interval = body["interval_minutes"]
            .as_u64()
            .context("Choose a refresh interval")?;
        let temp = body["temperature_c"]
            .as_u64()
            .context("Choose a temperature alert")?;
        let life = body["remaining_life_percent"]
            .as_u64()
            .context("Choose a remaining endurance alert")?;
        if ![0, 1, 5, 15, 30, 60].contains(&interval) || !(30..=90).contains(&temp) || life > 50 {
            bail!("Invalid health monitoring settings");
        }
        if interval > 0 {
            if let Some(reason) = crate::health_platform::collection_unavailable_reason() {
                bail!(reason);
            }
        }
        let value =
            json!({"interval_minutes":interval,"temperature_c":temp,"remaining_life_percent":life});
        connect(&self.path)?.execute(
            "UPDATE health_settings SET data=? WHERE id=1",
            [value.to_string()],
        )?;
        Ok(value)
    }
    pub fn status(&self) -> Result<Value> {
        let unavailable = crate::health_platform::collection_unavailable_reason();
        let db = connect(&self.path)?;
        let settings = self.settings()?;
        let mut statement =
            db.prepare("SELECT drive,data,time,present FROM health_latest ORDER BY drive")?;
        let drives = statement
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, f64>(2)?,
                    r.get::<_, bool>(3)?,
                ))
            })?
            .map(|row| {
                let (id, raw, time, present) = row?;
                let mut data: Value = serde_json::from_str(&raw).unwrap_or(Value::Null);
                evaluate(&mut data, &settings);
                data["id"] = json!(id);
                data["sampled_at"] = json!(time);
                data["present"] = json!(present);
                data["stale"] = json!(unavailable.is_some() || now() - time > 900.);
                Ok(data)
            })
            .collect::<Result<Vec<_>>>()?;
        let error = unavailable
            .map(str::to_owned)
            .unwrap_or_else(|| self.error.lock().clone());
        Ok(
            json!({"drives":drives,"settings":settings,"refreshing":self.active.load(Ordering::Acquire),"error":error,"probe_available":unavailable.is_none(),"disabled_reason":unavailable,"read_only":true,"model_calls":0,"host":std::env::consts::OS,"last_attempt":*self.last_attempt.lock()}),
        )
    }
    pub fn detail(&self, id: &str) -> Result<Value> {
        if id.len() != 64 || !id.bytes().all(|c| c.is_ascii_hexdigit()) {
            bail!("Invalid drive identifier");
        }
        let db = connect(&self.path)?;
        let saved: Option<(String, f64, bool)> = db
            .query_row(
                "SELECT data,time,present FROM health_latest WHERE drive=?",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;
        let (raw, time, present) = saved.context("Drive has not been checked")?;
        let mut drive: Value = serde_json::from_str(&raw).unwrap_or(Value::Null);
        evaluate(&mut drive, &self.settings()?);
        drive["id"] = json!(id);
        let unavailable = crate::health_platform::collection_unavailable_reason();
        drive["sampled_at"] = json!(time);
        drive["present"] = json!(present);
        drive["stale"] = json!(unavailable.is_some() || now() - time > 900.);
        let mut stmt=db.prepare("SELECT time,data FROM health_samples WHERE drive=? ORDER BY time DESC,id DESC LIMIT 180")?;
        let mut history=stmt.query_map([id],|r|Ok((r.get::<_,f64>(0)?,r.get::<_,String>(1)?)))?.map(|r|{let(time,raw)=r?;let d:Value=serde_json::from_str(&raw).unwrap_or(Value::Null);Ok(json!({"time":time,"temperature_c":d["temperature_c"],"remaining_percent":d["remaining_percent"],"power_on_hours":d["power_on_hours"],"status":d["status"].as_str().unwrap_or("Unknown")}))}).collect::<Result<Vec<_>>>()?;
        history.reverse();
        let mut stmt=db.prepare("SELECT time,data FROM health_events WHERE drive=? ORDER BY time DESC,id DESC LIMIT 100")?;
        let events = stmt
            .query_map([id], |r| Ok((r.get::<_, f64>(0)?, r.get::<_, String>(1)?)))?
            .map(|r| {
                let (time, raw) = r?;
                let mut d = serde_json::from_str::<Value>(&raw)
                    .ok()
                    .filter(Value::is_object)
                    .unwrap_or_else(|| json!({"status":"Unknown","reasons":["This saved health event is corrupt and cannot be read."],"codes":["unreadable_saved_event"]}));
                d["time"] = json!(time);
                Ok(d)
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(
            json!({"drive":drive,"history":history,"events":events,"probe_available":unavailable.is_none(),"disabled_reason":unavailable,"read_only":true,"model_calls":0}),
        )
    }
    pub fn refresh(self: &Arc<Self>) -> Result<Value> {
        if crate::health_platform::collection_unavailable_reason().is_some() {
            return self.status();
        }
        if self
            .active
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return self.status();
        }
        // Coalesce manual clicks and every client into one probe, at most once / 10s.
        if now() - *self.last_attempt.lock() < 10. {
            self.active.store(false, Ordering::Release);
            return self.status();
        }
        *self.last_attempt.lock() = now();
        self.error.lock().clear();
        let this = self.clone();
        tokio::spawn(async move {
            let start = Instant::now();
            let result = async {
                let mut command = tokio::process::Command::new(std::env::current_exe()?);
                command
                    .arg("health-probe")
                    .kill_on_drop(true)
                    .stdin(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null());
                #[cfg(windows)]
                command.creation_flags(0x08000000);
                let output = tokio::time::timeout(Duration::from_secs(45), command.output())
                    .await
                    .context("Drive check timed out; previous readings retained")??;
                if !output.status.success() || output.stdout.len() > 2 * 1024 * 1024 {
                    bail!("Drive telemetry is unavailable; previous readings retained");
                }
                let data: Value = serde_json::from_slice(&output.stdout)?;
                this.record(&data, start.elapsed().as_millis() as u64)
            }
            .await;
            if let Err(error) = result {
                *this.error.lock() = error.to_string();
            }
            this.active.store(false, Ordering::Release);
        });
        self.status()
    }
    fn record(&self, data: &Value, elapsed: u64) -> Result<()> {
        let drives = data["drives"]
            .as_array()
            .context("Missing drive telemetry")?;
        if drives.len() > 32 {
            bail!("Too many drives in telemetry response");
        }
        for drive in drives {
            if !drive.is_object()
                || !drive["model"]
                    .as_str()
                    .is_some_and(|s| !s.is_empty() && s.len() <= 512)
                || !drive["attributes"]
                    .as_array()
                    .is_some_and(|rows| rows.len() <= 128 && rows.iter().all(Value::is_object))
            {
                bail!("Invalid drive report; saved readings retained");
            }
        }
        let settings = self.settings()?;
        let time = now();
        let mut db = connect(&self.path)?;
        let tx = db.transaction()?;
        tx.execute("UPDATE health_latest SET present=0", [])?;
        for item in drives {
            let mut d = item.clone();
            let identity = format!(
                "{}|{}|{}|{}",
                d["model"],
                d["serial"],
                d["capacity_bytes"],
                if d["serial"].as_str().unwrap_or("").is_empty() {
                    d["path"].as_str().unwrap_or("")
                } else {
                    ""
                }
            );
            let id = hex::encode(Sha256::digest(identity));
            d["id"] = json!(id);
            d["sampled_at"] = json!(time);
            d["elapsed_ms"] = json!(elapsed);
            evaluate(&mut d, &settings);
            let previous: Option<String> = tx
                .query_row("SELECT data FROM health_latest WHERE drive=?", [&id], |r| {
                    r.get(0)
                })
                .optional()?;
            let prev: Value = previous
                .and_then(|v| serde_json::from_str(&v).ok())
                .unwrap_or(Value::Null);
            if (d["status"] != prev["status"] || d["alert_codes"] != prev["alert_codes"])
                && (previous_status(&prev) != "" || d["status"] != "Good")
            {
                tx.execute("INSERT INTO health_events(drive,time,data) VALUES(?,?,?)",params![id,time,json!({"status":d["status"],"reasons":d["reasons"],"previous_status":prev["status"]}).to_string()])?;
            }
            let raw = d.to_string();
            tx.execute(
                "INSERT INTO health_samples(drive,time,data) VALUES(?,?,?)",
                params![id, time, raw],
            )?;
            tx.execute("INSERT INTO health_latest(drive,time,data,present) VALUES(?,?,?,1) ON CONFLICT(drive) DO UPDATE SET time=excluded.time,data=excluded.data,present=1",params![id,time,raw])?;
            tx.execute("DELETE FROM health_samples WHERE drive=? AND id NOT IN (SELECT id FROM health_samples WHERE drive=? ORDER BY time DESC,id DESC LIMIT 512)",params![id,id])?;
        }
        tx.execute(
            "DELETE FROM health_samples WHERE time<?",
            [time - 90. * 86400.],
        )?;
        tx.execute("DELETE FROM health_events WHERE id NOT IN (SELECT id FROM health_events ORDER BY time DESC,id DESC LIMIT 1000)",[])?;
        tx.execute(
            "DELETE FROM health_latest WHERE present=0 AND time<?",
            [time - 90. * 86400.],
        )?;
        tx.commit()?;
        Ok(())
    }
    pub async fn scheduler(self: Arc<Self>) {
        if crate::health_platform::collection_unavailable_reason().is_some() {
            return;
        }
        loop {
            tokio::time::sleep(Duration::from_secs(30)).await;
            let interval = self
                .settings()
                .ok()
                .and_then(|v| v["interval_minutes"].as_u64())
                .unwrap_or(0);
            if interval > 0 && now() - *self.last_attempt.lock() >= interval as f64 * 60. {
                let _ = self.refresh();
            }
        }
    }
}
fn previous_status(d: &Value) -> &str {
    d["status"].as_str().unwrap_or("")
}
pub fn blank(path: &str) -> Value {
    json!({"path":path,"model":"Unknown drive","serial":"","firmware":"","bus":"Unknown","capacity_bytes":null,"rotation_rpm":null,"source":"Unavailable","coverage":"unavailable","smart_passed":null,"temperature_c":null,"remaining_percent":null,"power_on_hours":null,"power_cycles":null,"host_reads_bytes":null,"host_writes_bytes":null,"attributes":[],"limitations":[]})
}
pub fn attribute(id: &str, name: &str, raw: impl ToString, unit: &str) -> Value {
    json!({"id":id,"name":name,"raw":raw.to_string(),"unit":unit,"current":null,"worst":null,"threshold":null})
}
pub fn parse_nvme(data: &[u8], drive: &mut Value) -> Result<()> {
    if data.len() != 512
        || data.iter().all(|b| *b == 0)
        || data.iter().all(|b| *b == 255)
        || data[3] > 100
        || data[4] > 100
    {
        bail!("Invalid NVMe health log");
    }
    let temp = u16::from_le_bytes([data[1], data[2]]);
    drive["temperature_c"] = if (200..=500).contains(&temp) {
        json!(temp as i32 - 273)
    } else {
        Value::Null
    };
    drive["critical_warning"] = json!(data[0]);
    drive["available_spare"] = json!(data[3]);
    drive["spare_threshold"] = json!(data[4]);
    drive["percentage_used"] = json!(data[5]);
    drive["remaining_percent"] = json!(100u8.saturating_sub(data[5]));
    drive["smart_passed"] = json!(data[0] == 0);
    drive["coverage"] = json!("smart");
    let mut attrs = vec![
        attribute("01", "Critical warning", data[0], "bit mask"),
        attribute(
            "02",
            "Composite temperature",
            drive["temperature_c"]
                .as_i64()
                .map(|v| v.to_string())
                .unwrap_or("Unknown".into()),
            "°C",
        ),
        attribute("03", "Available spare", data[3], "%"),
        attribute("04", "Available spare threshold", data[4], "%"),
        attribute("05", "Percentage used", data[5], "%"),
    ];
    for (i, name, key, unit) in [
        (32, "Data units read", "host_reads_bytes", "512,000 bytes"),
        (
            48,
            "Data units written",
            "host_writes_bytes",
            "512,000 bytes",
        ),
        (64, "Host read commands", "host_read_commands", "commands"),
        (80, "Host write commands", "host_write_commands", "commands"),
        (
            96,
            "Controller busy time",
            "controller_busy_minutes",
            "minutes",
        ),
        (112, "Power cycles", "power_cycles", "cycles"),
        (128, "Power on hours", "power_on_hours", "hours"),
        (144, "Unsafe shutdowns", "unsafe_shutdowns", "events"),
        (
            160,
            "Media and data integrity errors",
            "media_errors",
            "errors",
        ),
        (176, "Error log entries", "error_log_entries", "entries"),
    ] {
        let value = u128::from_le_bytes(data[i..i + 16].try_into()?);
        attrs.push(attribute(
            &format!("{:02X}", 6 + (i - 32) / 16),
            name,
            value,
            unit,
        ));
        drive[key] = json!(if i == 32 || i == 48 {
            value.checked_mul(512000).map(|v| v.to_string())
        } else {
            Some(value.to_string())
        });
    }
    for (i, name) in [
        (192, "Warning temperature time"),
        (196, "Critical temperature time"),
    ] {
        attrs.push(attribute(
            &format!("{:02X}", 16 + (i - 192) / 4),
            name,
            u32::from_le_bytes(data[i..i + 4].try_into()?),
            "minutes",
        ));
    }
    for i in 0..8 {
        let n = u16::from_le_bytes([data[200 + i * 2], data[201 + i * 2]]);
        if (200..=500).contains(&n) {
            attrs.push(attribute(
                &format!("T{}", i + 1),
                &format!("Temperature sensor {}", i + 1),
                n as i32 - 273,
                "°C",
            ));
        }
    }
    drive["attributes"] = json!(attrs);
    Ok(())
}
pub fn parse_ata(data: &[u8], thresholds: Option<&[u8]>, drive: &mut Value) -> Result<()> {
    if data.len() != 512
        || data.iter().fold(0u8, |a, b| a.wrapping_add(*b)) != 0
        || data[..2] == [0, 0]
    {
        bail!("Invalid ATA SMART data/checksum");
    }
    let thresholds =
        thresholds.filter(|t| t.len() == 512 && t.iter().fold(0u8, |a, b| a.wrapping_add(*b)) == 0);
    let mut attrs = vec![];
    for row in data[2..362].chunks_exact(12) {
        let id = row[0];
        if id == 0 {
            continue;
        }
        let raw = u64::from_le_bytes([row[5], row[6], row[7], row[8], row[9], row[10], 0, 0]);
        let name = match id {
            1 => "Read error rate",
            3 => "Spin up time",
            4 => "Start/stop count",
            5 => "Reallocated sectors",
            7 => "Seek error rate",
            9 => "Power on hours (vendor units)",
            10 => "Spin retry count",
            12 => "Power cycles",
            184 => "End-to-end errors",
            187 => "Reported uncorrectable errors",
            188 => "Command timeout",
            190 => "Airflow temperature",
            194 => "Temperature",
            196 => "Reallocation events",
            197 => "Current pending sectors",
            198 => "Offline uncorrectable sectors",
            199 => "Interface CRC errors",
            _ => "Vendor-specific attribute",
        };
        let mut a = attribute(&format!("{id:02X}"), name, raw, "");
        a["raw_hex"] = json!(format!("{raw:012X}"));
        a["current"] = json!(row[3]);
        a["worst"] = json!(row[4]);
        a["prefailure"] = json!(row[1] & 1 != 0);
        if let Some(t) = thresholds {
            if let Some(r) = t[2..362].chunks_exact(12).find(|r| r[0] == id) {
                a["threshold"] = json!(r[1]);
            }
        }
        if id == 194 && row[5] > 0 && row[5] < 125 {
            drive["temperature_c"] = json!(row[5]);
        }
        if id == 12 {
            drive["power_cycles"] = json!(raw.to_string());
        }
        // Do not infer vendor-specific SSD wear or hour units from an ID alone.
        if [5, 197, 198].contains(&id) {
            drive[format!("ata_{id}")] = json!(raw);
        }
        attrs.push(a);
    }
    if attrs.is_empty() {
        bail!("Empty ATA SMART table");
    }
    drive["attributes"] = json!(attrs);
    drive["coverage"] = json!("smart");
    Ok(())
}
pub fn evaluate(d: &mut Value, settings: &Value) {
    if !d.is_object() {
        *d = blank("unavailable");
        d["limitations"] =
            json!(["Saved drive data is invalid. No health assessment is available."]);
    }
    let mut severity = if d["smart_passed"] == true { 0 } else { 1 };
    let mut reasons = Vec::<String>::new();
    let mut codes = Vec::<String>::new();
    let mut alert = |level: u8, code: &str, message: String| {
        severity = severity.max(level);
        codes.push(code.into());
        reasons.push(message);
    };
    if d["smart_passed"] == false || d["critical_warning"].as_u64().is_some_and(|n| n != 0) {
        alert(3,"smart_failed","The drive reports a SMART health warning. Back up important files and inspect this drive.".into());
    }
    if d["available_spare"]
        .as_u64()
        .zip(d["spare_threshold"].as_u64())
        .is_some_and(|(a, b)| a < b)
    {
        alert(
            3,
            "spare",
            "Available spare is below the drive's threshold.".into(),
        );
    }
    if let Some(life) = d["remaining_percent"].as_u64() {
        if life <= settings["remaining_life_percent"].as_u64().unwrap_or(10) {
            alert(
                2,
                "endurance",
                format!(
                    "{life}% of rated endurance remains. This is a wear estimate, not a failure date."
                ),
            );
        }
    }
    if let Some(temp) = d["temperature_c"].as_i64() {
        if temp >= settings["temperature_c"].as_i64().unwrap_or(70) {
            alert(
                2,
                "temperature",
                format!("Temperature is {temp} °C, at or above your alert threshold."),
            );
        }
    }
    if d["media_errors"]
        .as_str()
        .and_then(|s| s.parse::<u128>().ok())
        .is_some_and(|n| n > 0)
    {
        alert(
            2,
            "media_errors",
            "The drive has recorded media or data integrity errors.".into(),
        );
    }
    for (key, name) in [
        ("ata_5", "reallocated sectors"),
        ("ata_197", "pending sectors"),
        ("ata_198", "uncorrectable sectors"),
    ] {
        if d[key].as_u64().is_some_and(|n| n > 0) {
            alert(
                2,
                key,
                format!(
                    "SMART reports {name}. Back up important files and review the raw attributes."
                ),
            );
        }
    }
    for a in d["attributes"].as_array().into_iter().flatten() {
        if a["prefailure"] == true
            && a["current"]
                .as_u64()
                .zip(a["threshold"].as_u64())
                .is_some_and(|(a, b)| b > 0 && a <= b)
        {
            alert(
                3,
                "ata_threshold",
                format!(
                    "{} has reached its failure threshold.",
                    a["name"].as_str().unwrap_or("An attribute")
                ),
            );
        }
    }
    if reasons.is_empty() {
        reasons.push(if severity==0{"No health warning reported by the available SMART readings. This does not guarantee the drive cannot fail."}else{"Detailed SMART health is unavailable. A basic device status is not a health test."}.into());
    }
    d["status"] = json!(match severity {
        0 => "Good",
        2 => "Caution",
        3 => "Bad",
        _ => "Unknown",
    });
    d["reasons"] = json!(reasons);
    d["alert_codes"] = json!(codes);
}

#[cfg(test)]
mod tests {
    use super::*;
    fn settings() -> Value {
        json!({"temperature_c":70,"remaining_life_percent":10})
    }
    fn nvme() -> Vec<u8> {
        let mut b = vec![0; 512];
        b[1..3].copy_from_slice(&310u16.to_le_bytes());
        b[3] = 100;
        b[4] = 10;
        b[5] = 4;
        b
    }
    #[test]
    fn nvme_reads_real_units_and_128bit_counters() {
        let mut b = nvme();
        let n = u64::MAX as u128 + 123;
        b[32..48].copy_from_slice(&n.to_le_bytes());
        let mut d = blank("test");
        parse_nvme(&b, &mut d).unwrap();
        evaluate(&mut d, &settings());
        assert_eq!(d["temperature_c"], 37);
        assert_eq!(d["remaining_percent"], 96);
        assert_eq!(d["host_reads_bytes"], (n * 512000).to_string());
        assert_eq!(d["status"], "Good");
    }
    #[test]
    fn missing_and_corrupt_data_never_means_healthy() {
        for b in [vec![0; 512], vec![255; 512], vec![1; 200]] {
            let mut d = blank("test");
            assert!(parse_nvme(&b, &mut d).is_err());
            evaluate(&mut d, &settings());
            assert_eq!(d["status"], "Unknown");
            assert!(d["temperature_c"].is_null());
        }
    }
    #[test]
    fn critical_warnings_override_summary_pass() {
        let mut d = blank("test");
        d["smart_passed"] = json!(true);
        d["critical_warning"] = json!(8);
        evaluate(&mut d, &settings());
        assert_eq!(d["status"], "Bad");
    }
    #[test]
    fn rated_endurance_is_clamped_but_not_a_failure_prediction() {
        let mut b = nvme();
        b[5] = 150;
        let mut d = blank("test");
        parse_nvme(&b, &mut d).unwrap();
        evaluate(&mut d, &settings());
        assert_eq!(d["remaining_percent"], 0);
        assert_eq!(d["status"], "Caution");
    }
    #[test]
    fn spare_media_and_temperature_thresholds() {
        let mut b = nvme();
        b[3] = 3;
        let mut d = blank("test");
        parse_nvme(&b, &mut d).unwrap();
        evaluate(&mut d, &settings());
        assert_eq!(d["status"], "Bad");
        let mut b = nvme();
        b[160] = 1;
        b[1..3].copy_from_slice(&350u16.to_le_bytes());
        parse_nvme(&b, &mut d).unwrap();
        evaluate(&mut d, &settings());
        assert_eq!(d["status"], "Caution");
        assert!(
            d["alert_codes"]
                .as_array()
                .unwrap()
                .contains(&json!("temperature"))
        );
    }
    fn ata() -> Vec<u8> {
        let mut b = vec![0; 512];
        b[0] = 1;
        b[2] = 5;
        b[3] = 1;
        b[5] = 100;
        b[6] = 90;
        b[7] = 2;
        checksum(&mut b);
        b
    }
    fn checksum(b: &mut [u8]) {
        b[511] = 0;
        b[511] = 0u8.wrapping_sub(b.iter().fold(0u8, |a, b| a.wrapping_add(*b)));
    }
    #[test]
    fn ata_checksums_thresholds_and_vendor_units() {
        let mut d = blank("ata");
        let mut b = ata();
        let mut t = vec![0; 512];
        t[0] = 1;
        t[2] = 5;
        t[3] = 100;
        checksum(&mut t);
        parse_ata(&b, Some(&t), &mut d).unwrap();
        evaluate(&mut d, &settings());
        assert_eq!(d["status"], "Bad");
        assert!(d["power_on_hours"].is_null());
        assert_eq!(d["attributes"][0]["raw_hex"], "000000000002");
        b[20] = 7;
        assert!(parse_ata(&b, None, &mut blank("bad")).is_err());
    }
    #[test]
    fn ata_reallocated_sector_is_caution_without_guessing_wear() {
        let mut d = blank("ata");
        parse_ata(&ata(), None, &mut d).unwrap();
        evaluate(&mut d, &settings());
        assert_eq!(d["status"], "Caution");
        assert!(d["remaining_percent"].is_null());
    }
    #[test]
    fn history_alerts_recovery_and_settings_persist() {
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().canonicalize().unwrap();
        let health = DriveHealth::open(&state).unwrap();
        let mut d = blank("drive0");
        d["model"] = json!("fixture");
        d["serial"] = json!("one");
        parse_nvme(&nvme(), &mut d).unwrap();
        health.record(&json!({"drives":[d.clone()]}), 1).unwrap();
        let first = health.status().unwrap();
        let id = first["drives"][0]["id"].as_str().unwrap();
        d["temperature_c"] = json!(80);
        health.record(&json!({"drives":[d.clone()]}), 1).unwrap();
        health.record(&json!({"drives":[d.clone()]}), 1).unwrap();
        assert_eq!(
            health.detail(id).unwrap()["events"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        d["temperature_c"] = json!(40);
        health.record(&json!({"drives":[d.clone()]}), 1).unwrap();
        assert_eq!(
            health.detail(id).unwrap()["events"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
        health.record(&json!({"drives":[]}), 1).unwrap();
        assert_eq!(health.status().unwrap()["drives"][0]["present"], false);
        assert!(
            health
                .configure(
                    &json!({"interval_minutes":2,"temperature_c":70,"remaining_life_percent":10})
                )
                .is_err()
        );
        let interval = if cfg!(windows) { 0 } else { 15 };
        let setting =
            json!({"interval_minutes":interval,"temperature_c":65,"remaining_life_percent":15});
        health.configure(&setting).unwrap();
        assert_eq!(
            DriveHealth::open(&state).unwrap().settings().unwrap(),
            setting
        );
        assert!(health.detail("../other").is_err());
    }

    #[cfg(windows)]
    #[test]
    fn windows_cached_health_readings_are_always_saved_readings() {
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().canonicalize().unwrap();
        let health = DriveHealth::open(&state).unwrap();
        let mut drive = blank("fixture");
        parse_nvme(&nvme(), &mut drive).unwrap();
        health.record(&json!({"drives":[drive]}), 1).unwrap();
        let status = health.status().unwrap();
        assert_eq!(status["drives"][0]["status"], "Good");
        assert_eq!(status["drives"][0]["stale"], true);
        assert_eq!(status["probe_available"], false);
        assert!(!status["error"].as_str().unwrap().is_empty());
        let id = status["drives"][0]["id"].as_str().unwrap();
        let detail = health.detail(id).unwrap();
        assert_eq!(detail["probe_available"], false);
        assert_eq!(detail["drive"]["stale"], true);
        assert_eq!(
            detail["drive"]["sampled_at"],
            status["drives"][0]["sampled_at"]
        );
        assert_eq!(
            health.detail(id).unwrap()["history"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn health_nvme_truncation_and_invalid_percentages_are_rejected() {
        let valid = nvme();
        for length in 0..512 {
            assert!(parse_nvme(&valid[..length], &mut blank("fixture")).is_err());
        }
        let mut extra = valid.clone();
        extra.push(0);
        assert!(parse_nvme(&extra, &mut blank("fixture")).is_err());
        let mut invalid = valid.clone();
        invalid[3] = 101;
        assert!(parse_nvme(&invalid, &mut blank("fixture")).is_err());
        invalid = valid.clone();
        invalid[4] = 255;
        assert!(parse_nvme(&invalid, &mut blank("fixture")).is_err());
    }
    #[test]
    fn health_nvme_unavailable_temperature_and_overflow_remain_null() {
        let mut bytes = nvme();
        bytes[1] = 0;
        bytes[2] = 0;
        bytes[32..48].copy_from_slice(&u128::MAX.to_le_bytes());
        let mut d = blank("fixture");
        parse_nvme(&bytes, &mut d).unwrap();
        assert!(d["temperature_c"].is_null());
        assert!(d["host_reads_bytes"].is_null());
        assert_eq!(d["attributes"][5]["raw"], u128::MAX.to_string());
    }
    #[test]
    fn health_unknown_vendor_units_are_never_guessed() {
        let mut bytes = ata();
        bytes[2] = 0xe7;
        bytes[7] = 99;
        checksum(&mut bytes);
        let mut d = blank("fixture");
        parse_ata(&bytes, None, &mut d).unwrap();
        evaluate(&mut d, &settings());
        assert_eq!(d["status"], "Unknown");
        assert!(d["remaining_percent"].is_null());
        assert!(d["power_on_hours"].is_null());
        assert_eq!(d["attributes"][0]["name"], "Vendor-specific attribute");
    }
    #[test]
    fn health_malformed_report_is_atomic_and_retains_previous_data() {
        let dir = tempfile::tempdir().unwrap();
        let h = DriveHealth::open(&dir.path().canonicalize().unwrap()).unwrap();
        let mut d = blank("fixture://drive");
        parse_nvme(&nvme(), &mut d).unwrap();
        h.record(&json!({"drives":[d]}), 1).unwrap();
        assert!(h.record(&json!({"drives":[null]}), 1).is_err());
        assert!(
            h.record(&json!({"drives":[{"model":"Broken","attributes":42}]}), 1)
                .is_err()
        );
        let status = h.status().unwrap();
        assert_eq!(status["drives"].as_array().unwrap().len(), 1);
        assert_eq!(status["drives"][0]["present"], true);
    }
    #[test]
    fn health_corrupt_saved_shape_becomes_unknown_without_panicking() {
        for mut d in [json!(null), json!(42), json!("bad"), json!([])] {
            evaluate(&mut d, &settings());
            assert_eq!(d["status"], "Unknown");
            assert!(!d["limitations"].as_array().unwrap().is_empty());
        }
    }
    #[test]
    fn health_corrupt_saved_rows_keep_identity_and_history_gaps() {
        let dir = tempfile::tempdir().unwrap();
        let h = DriveHealth::open(&dir.path().canonicalize().unwrap()).unwrap();
        let mut d = blank("fixture://drive");
        parse_nvme(&nvme(), &mut d).unwrap();
        h.record(&json!({"drives":[d]}), 1).unwrap();
        let saved = h.status().unwrap();
        let id = saved["drives"][0]["id"].as_str().unwrap();
        let db = connect(&h.path).unwrap();
        db.execute(
            "INSERT INTO health_events(drive,time,data) VALUES(?,?,?)",
            params![id, now(), "{}"],
        )
        .unwrap();
        for raw in ["[]", "{", "null", "42"] {
            for table in ["health_latest", "health_samples", "health_events"] {
                db.execute(
                    &format!("UPDATE {table} SET data=? WHERE drive=?"),
                    params![raw, id],
                )
                .unwrap();
            }
            let status = h.status().unwrap();
            assert_eq!(status["drives"][0]["id"], id);
            assert_eq!(status["drives"][0]["status"], "Unknown");
            let detail = h.detail(id).unwrap();
            assert_eq!(detail["drive"]["id"], id);
            assert_eq!(detail["drive"]["status"], "Unknown");
            assert_eq!(
                detail["drive"]["sampled_at"],
                saved["drives"][0]["sampled_at"]
            );
            assert!(detail["history"][0]["temperature_c"].is_null());
            assert_eq!(detail["history"][0]["status"], "Unknown");
            assert_eq!(detail["events"][0]["status"], "Unknown");
            assert_eq!(detail["events"][0]["codes"][0], "unreadable_saved_event");
        }
    }
    #[test]
    fn health_history_retention_and_reconnection_are_bounded() {
        let dir = tempfile::tempdir().unwrap();
        let h = DriveHealth::open(&dir.path().canonicalize().unwrap()).unwrap();
        let mut d = blank("fixture://drive");
        parse_nvme(&nvme(), &mut d).unwrap();
        h.record(&json!({"drives":[d.clone()]}), 1).unwrap();
        let before = h.status().unwrap();
        let id = before["drives"][0]["id"].as_str().unwrap();
        let db = connect(&h.path).unwrap();
        for i in 0..520 {
            db.execute(
                "INSERT INTO health_samples(drive,time,data) VALUES(?,?,?)",
                params![id, now() - 1000. + i as f64, d.to_string()],
            )
            .unwrap();
        }
        h.record(&json!({"drives":[]}), 1).unwrap();
        assert_eq!(h.status().unwrap()["drives"][0]["present"], false);
        h.record(&json!({"drives":[d]}), 1).unwrap();
        assert_eq!(h.status().unwrap()["drives"][0]["present"], true);
        assert_eq!(
            db.query_row(
                "SELECT count(*) FROM health_samples WHERE drive=?",
                [id],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            512
        );
        assert_eq!(
            h.detail(id).unwrap()["history"].as_array().unwrap().len(),
            180
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_refresh_and_monitoring_cannot_launch_a_probe() {
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().canonicalize().unwrap();
        let health = DriveHealth::open(&state).unwrap();
        // There is deliberately no Tokio runtime: an attempted spawn would panic.
        let status = health.refresh().unwrap();
        assert_eq!(status["probe_available"], false);
        assert_eq!(status["refreshing"], false);
        assert_eq!(status["last_attempt"].as_f64(), Some(0.));
        assert_eq!(status["error"], status["disabled_reason"]);
        assert!(!status["error"].as_str().unwrap().is_empty());
        assert!(
            health
                .configure(
                    &json!({"interval_minutes":1,"temperature_c":70,"remaining_life_percent":10})
                )
                .is_err()
        );
        assert_eq!(health.settings().unwrap()["interval_minutes"], 0);
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn windows_scheduler_exits_with_existing_monitor_settings() {
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().canonicalize().unwrap();
        let health = DriveHealth::open(&state).unwrap();
        connect(&health.path)
            .unwrap()
            .execute(
                "UPDATE health_settings SET data=? WHERE id=1",
                [
                    json!({"interval_minutes":1,"temperature_c":70,"remaining_life_percent":10})
                        .to_string(),
                ],
            )
            .unwrap();
        tokio::time::timeout(Duration::from_millis(100), health.clone().scheduler())
            .await
            .unwrap();
        assert_eq!(health.status().unwrap()["last_attempt"].as_f64(), Some(0.));
    }
}
