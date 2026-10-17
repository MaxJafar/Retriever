//! Platform adapters expose only fixed, read-only health queries. Device paths
//! originate from OS enumeration; the HTTP client cannot supply a path/command.
#[cfg(target_os = "macos")]
use crate::health::blank;
#[cfg(target_os = "macos")]
use anyhow::Context;
use anyhow::{Result, bail};
use serde_json::{Value, json};

/// Keep Windows collection closed until the storage query path is validated.
/// The 2026-10-14 dump caught retriever.exe completing a protocol-specific NVMe
/// query in storport with kernel heap corruption (bugcheck 0x139, parameter 0x1d).
/// A child process and a timeout cannot contain a kernel-mode failure.
pub fn collection_unavailable_reason() -> Option<&'static str> {
    cfg!(windows).then_some(
        "Drive health checks are temporarily disabled on Windows after a system crash. Saved readings remain available.",
    )
}

pub fn collect() -> Result<Value> {
    if let Some(reason) = collection_unavailable_reason() {
        bail!(reason);
    }
    Ok(json!({"drives":platform::collect()?,"read_only":true,"model_calls":0}))
}

#[cfg(all(test, windows))]
mod safety_tests {
    #[test]
    fn direct_probe_is_blocked_before_opening_devices() {
        let reason = super::collection_unavailable_reason().unwrap();
        assert_eq!(super::collect().unwrap_err().to_string(), reason);
        assert_eq!(super::platform::collect().unwrap_err().to_string(), reason);
    }
}

#[cfg(windows)]
mod platform {
    use super::*;

    // No raw Windows device handles or IOCTLs remain in the active adapter.
    // Retain this independent refusal if an outer caller is changed later.
    pub fn collect() -> Result<Vec<Value>> {
        bail!(collection_unavailable_reason().expect("Windows health collection is disabled"));
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use super::*;
    use std::{
        io::Write,
        process::{Command, Stdio},
    };
    fn plist(args: &[&str]) -> Result<Value> {
        let output = Command::new("/usr/sbin/diskutil").args(args).output()?;
        if !output.status.success() || output.stdout.len() > 1024 * 1024 {
            bail!("macOS did not provide drive telemetry");
        }
        let mut child = Command::new("/usr/bin/plutil")
            .args(["-convert", "json", "-o", "-", "--", "-"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;
        child
            .stdin
            .take()
            .context("plist input unavailable")?
            .write_all(&output.stdout)?;
        let output = child.wait_with_output()?;
        if !output.status.success() {
            bail!("Invalid macOS drive information");
        }
        Ok(serde_json::from_slice(&output.stdout)?)
    }
    pub fn collect() -> Result<Vec<Value>> {
        let list = plist(&["list", "-plist", "physical"])?;
        let mut drives = vec![];
        for disk in list["AllDisksAndPartitions"]
            .as_array()
            .into_iter()
            .flatten()
            .take(32)
        {
            let name = disk["DeviceIdentifier"].as_str().unwrap_or("");
            if !name.starts_with("disk")
                || !name[4..].bytes().all(|b| b.is_ascii_digit())
                || name.len() > 16
            {
                continue;
            }
            let mut drive = blank(&format!("/dev/{name}"));
            let mut limitations = vec![];
            match plist(&["info", "-plist", name]) {
                Ok(info) => {
                    drive["model"] = info["MediaName"].clone();
                    drive["bus"] = info["BusProtocol"].clone();
                    drive["capacity_bytes"] = info["TotalSize"].clone();
                    drive["serial"] = info.get("SerialNumber").cloned().unwrap_or(json!(""));
                    drive["firmware"] = info.get("FirmwareRevision").cloned().unwrap_or(json!(""));
                    let status = info["SMARTStatus"].as_str().unwrap_or("");
                    if status == "Verified" || status == "Failing" {
                        drive["smart_passed"] = json!(status == "Verified");
                        drive["coverage"] = json!("summary");
                        drive["source"] = json!("macOS SMART status");
                    }
                    let smart = &info["SMARTDeviceSpecificKeysMayVaryNotGuaranteed"];
                    if smart.is_object() {
                        // Apple's optional NVMe dictionary uses Kelvin and split 128-bit counters.
                        // Keep missing values null instead of synthesizing an all-zero log.
                        drive["source"] = json!("macOS NVMe SMART telemetry");
                        drive["coverage"] = json!("smart");
                        let mut attrs = vec![];
                        for (key, target, label, unit) in [
                            (
                                "TEMPERATURE",
                                "temperature_c",
                                "Composite temperature",
                                "°C",
                            ),
                            ("AVAILABLE_SPARE", "available_spare", "Available spare", "%"),
                            (
                                "AVAILABLE_SPARE_THRESHOLD",
                                "spare_threshold",
                                "Available spare threshold",
                                "%",
                            ),
                            ("PERCENTAGE_USED", "percentage_used", "Percentage used", "%"),
                            (
                                "CRITICAL_WARNING",
                                "critical_warning",
                                "Critical warning",
                                "bit mask",
                            ),
                        ] {
                            if let Some(n) = smart[key].as_u64() {
                                let value = if key == "TEMPERATURE" {
                                    if !(200..=500).contains(&n) {
                                        continue;
                                    }
                                    n as i64 - 273
                                } else {
                                    n as i64
                                };
                                drive[target] = json!(value);
                                attrs.push(crate::health::attribute(key, label, value, unit));
                            }
                        }
                        if let Some(used) = drive["percentage_used"].as_u64() {
                            drive["remaining_percent"] = json!(100u64.saturating_sub(used));
                        }
                        for (key, target, label, unit) in [
                            (
                                "DATA_UNITS_READ",
                                "host_reads_bytes",
                                "Data units read",
                                "512,000 bytes",
                            ),
                            (
                                "DATA_UNITS_WRITTEN",
                                "host_writes_bytes",
                                "Data units written",
                                "512,000 bytes",
                            ),
                            (
                                "HOST_READ_COMMANDS",
                                "host_read_commands",
                                "Host read commands",
                                "commands",
                            ),
                            (
                                "HOST_WRITE_COMMANDS",
                                "host_write_commands",
                                "Host write commands",
                                "commands",
                            ),
                            (
                                "CONTROLLER_BUSY_TIME",
                                "controller_busy_minutes",
                                "Controller busy time",
                                "minutes",
                            ),
                            ("POWER_CYCLES", "power_cycles", "Power cycles", "cycles"),
                            (
                                "POWER_ON_HOURS",
                                "power_on_hours",
                                "Power on hours",
                                "hours",
                            ),
                            (
                                "UNSAFE_SHUTDOWNS",
                                "unsafe_shutdowns",
                                "Unsafe shutdowns",
                                "events",
                            ),
                            (
                                "MEDIA_ERRORS",
                                "media_errors",
                                "Media and data integrity errors",
                                "errors",
                            ),
                            (
                                "NUM_ERROR_INFO_LOG_ENTRIES",
                                "error_log_entries",
                                "Error log entries",
                                "entries",
                            ),
                        ] {
                            if let Some(low) = smart[format!("{key}_0")].as_u64() {
                                if let Some(high) = smart[format!("{key}_1")].as_u64() {
                                    let n = (high as u128) << 64 | low as u128;
                                    attrs.push(crate::health::attribute(key, label, n, unit));
                                    drive[target] = json!(if target.ends_with("_bytes") {
                                        n.checked_mul(512000).map(|v| v.to_string())
                                    } else {
                                        Some(n.to_string())
                                    });
                                }
                            }
                        }
                        drive["attributes"] = json!(attrs);
                    } else {
                        limitations.push("macOS exposes no raw SMART table for this drive. USB bridges and older macOS versions may supply only a summary or no health data.");
                    }
                }
                Err(_) => limitations.push("macOS could not read this drive's information."),
            }
            drive["limitations"] = json!(limitations);
            drives.push(drive);
        }
        Ok(drives)
    }
}
#[cfg(not(any(windows, target_os = "macos")))]
mod platform {
    use super::*;
    pub fn collect() -> Result<Vec<Value>> {
        bail!("Native drive health is supported on Windows and macOS")
    }
}
