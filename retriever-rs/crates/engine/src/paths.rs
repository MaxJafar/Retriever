use crate::config::Device;
use anyhow::{Context, Result, bail};
use regex::Regex;
use std::{
    fs,
    path::{Component, Path, PathBuf},
    sync::LazyLock,
};

static SECRETS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)(^|[._ /\\-])(credentials?|passwords?|secrets?|tokens?|model[_-]api[_-]keys|private[_-]?key|id_rsa|id_ed25519|authorized_keys|\.env)([._ /\\-]|$)").unwrap()
});
static MARKERS: LazyLock<regex::bytes::Regex> = LazyLock::new(|| {
    regex::bytes::Regex::new(r#"(?i)-----BEGIN (?:[A-Z ]*PRIVATE KEY|OPENSSH PRIVATE KEY)-----|(?:api[_-]?key|access[_-]?token|password|client[_-]?secret)\s*[:=]\s*["']?[A-Za-z0-9/+_.-]{8,}"#).unwrap()
});
pub fn secret_content(bytes: &[u8]) -> bool {
    MARKERS.is_match(bytes)
}
pub fn absolute(path: &str) -> bool {
    !path.contains('\0')
        && !path.starts_with("\\\\.\\")
        && (Path::new(path).is_absolute()
            || path.starts_with('/')
            || path.starts_with("~/")
            || path.starts_with("~\\")
            || path.starts_with("\\\\")
            || (path.len() > 2
                && path.as_bytes()[1] == b':'
                && [b'/', b'\\'].contains(&path.as_bytes()[2])))
}
pub fn normalize(path: &str, windows: bool) -> String {
    let mut s = path.replace('\\', "/").trim_end_matches('/').to_owned();
    if windows {
        if let Some(tail) = s.strip_prefix("//?/UNC/") {
            s = format!("//{tail}");
        } else if let Some(tail) = s.strip_prefix("//?/") {
            s = tail.to_owned();
        }
        s.to_lowercase()
    } else {
        s
    }
}
pub fn safe_name(path: &str) -> bool {
    !SECRETS.is_match(path)
        && !path.replace('\\', "/").split('/').any(|p| {
            p.starts_with('.')
                || [
                    "node_modules",
                    "vendor",
                    "build",
                    "dist",
                    "target",
                    "bin",
                    "obj",
                    "__pycache__",
                    "library",
                ]
                .contains(&p.to_lowercase().as_str())
        })
}
pub fn eligible(d: &Device, path: &str) -> bool {
    if !d.enabled || crawl_protection(path, &d.platform).is_some() || !safe_name(path) || path.replace('\\', "/").split('/').any(|p| p == "..") {
        return false;
    }
    let win = d.platform == "windows";
    let p = normalize(path, win);
    d.roots.iter().any(|root| {
        let r = normalize(root, win);
        if p != r && !p.starts_with(&(r.clone() + "/")) {
            return false;
        }
        let rel = p.strip_prefix(&r).unwrap_or("").trim_start_matches('/');
        !d.exclude.iter().any(|ex| {
            let ex = normalize(ex, win);
            p == ex
                || p.starts_with(&(ex.clone() + "/"))
                || globset::Glob::new(&ex).ok().is_some_and(|g| {
                    let m = g.compile_matcher();
                    m.is_match(rel)
                        || rel.split('/').any(|part| m.is_match(part))
                        || rel
                            .split('/')
                            .scan(String::new(), |prefix, part| {
                                if !prefix.is_empty() {
                                    prefix.push('/');
                                }
                                prefix.push_str(part);
                                Some(prefix.clone())
                            })
                            .any(|prefix| m.is_match(prefix))
                })
        })
    })
}
/// An unconditional crawl boundary, independent of the user's include list.
/// Browsing remains available under normal OS permissions; this only limits indexing.
pub fn crawl_protection(path: &str, platform: &str) -> Option<&'static str> {
    let value = normalize(path,platform=="windows").to_lowercase();
    if value.starts_with("//") { return Some("Network roots require a separately configured device"); }
    let parts: Vec<_> = value.split('/').filter(|p| !p.is_empty()).map(|p| p.trim_end_matches([' ', '.'])).collect();
    if platform == "windows" {
        let first = parts.get(1).copied().unwrap_or("");
        if ["windows", "winnt", "program files", "program files (x86)", "programdata", "system volume information", "$recycle.bin", "recovery", "config.msi", "msocache", "perflogs", "documents and settings", "$windows.~bt", "$windows.~ws", "boot", "efi"].contains(&first) {
            return Some("Windows system, application and recovery folders are excluded");
        }
        if parts.contains(&"appdata") { return Some("Application data and caches are excluded"); }
    } else {
        let first = if parts.first() == Some(&"volumes") { parts.get(2) } else { parts.first() }.copied().unwrap_or("");
        if ["system", "applications", "library", "usr", "bin", "sbin", "private", "dev", "proc", "sys", "etc", "var", "opt", "cores"].contains(&first) {
            return Some("Operating system, application and runtime folders are excluded");
        }
    }
    None
}
pub fn expand(path: &str) -> PathBuf {
    if let Some(tail) = path.strip_prefix("~/").or_else(|| path.strip_prefix("~\\")) {
        PathBuf::from(
            std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
                .unwrap_or_default(),
        )
        .join(tail)
    } else {
        PathBuf::from(path)
    }
}
pub fn reject_links(path: &Path) -> Result<()> {
    for p in path.ancestors() {
        match fs::symlink_metadata(p) {
            Ok(m) => {
                if m.file_type().is_symlink() {
                    bail!("Symbolic links are not allowed");
                }
                #[cfg(windows)]
                {
                    use std::os::windows::fs::MetadataExt;
                    if m.file_attributes() & 0x400 != 0 {
                        bail!("Reparse points are not allowed");
                    }
                    if m.file_attributes() & (0x1000 | 0x40000 | 0x400000) != 0 {
                        bail!("Offline and cloud placeholder files are not read");
                    }
                }
                #[cfg(target_os="macos")]
                {
                    use std::os::macos::fs::MetadataExt;
                    // SF_DATALESS (Apple sys/stat.h): lstat only; never hydrate.
                    if m.st_flags() & 0x40000000 != 0 {
                        bail!("Cloud placeholder files are not read");
                    }
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
    }
    Ok(())
}
pub fn checked(d: &Device, path: &str, forbidden: &[PathBuf], must_exist: bool) -> Result<PathBuf> {
    let p = expand(path);
    if !p.is_absolute() || p.components().any(|c| matches!(c, Component::ParentDir)) {
        bail!("Use an absolute path without traversal");
    }
    if !eligible(d, &p.to_string_lossy()) {
        bail!("Path is outside included folders or is excluded");
    }
    reject_links(&p)?;
    let canonical = if must_exist {
        p.canonicalize()?
    } else {
        p.parent()
            .context("Missing parent")?
            .canonicalize()?
            .join(p.file_name().context("Missing filename")?)
    };
    if forbidden.iter().any(|f| {
        f.canonicalize()
            .ok()
            .is_some_and(|f| canonical.starts_with(f))
    }) {
        bail!("Application state is not a source folder");
    }
    Ok(p)
}
