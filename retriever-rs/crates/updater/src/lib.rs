//! Public GitHub release discovery and verified downloads. No library access,
//! credentials, privileged installation or remote command execution.
use anyhow::{Context, Result, bail, ensure};
use futures_util::StreamExt;
use reqwest::{Client, StatusCode, Url, header};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const MAX_METADATA: usize = 4 * 1024 * 1024;
const MAX_ASSET: u64 = 2 * 1024 * 1024 * 1024;
const INTERVAL: u64 = 6 * 60 * 60;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Asset {
    pub id: u64,
    pub name: String,
    pub size: u64,
    pub state: String,
    pub browser_download_url: String,
    pub digest: Option<String>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Release {
    pub id: u64,
    pub tag_name: String,
    pub draft: bool,
    pub prerelease: bool,
    pub html_url: String,
    #[serde(default)]
    pub body: Option<String>,
    pub assets: Vec<Asset>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Update {
    pub release_id: u64,
    pub version: String,
    pub notes: String,
    pub release_url: String,
    pub asset: Asset,
}
#[derive(Default, Deserialize, Serialize)]
struct Cache {
    repository: String,
    next_check: u64,
    etag: Option<String>,
    #[serde(default)]
    retry_after_until: u64,
    #[serde(default)]
    releases: Vec<Release>,
}
#[derive(Debug, Serialize)]
pub struct Check {
    pub available: bool,
    pub update: Option<Update>,
    pub cached: bool,
    pub next_check: u64,
    pub message: String,
}
#[derive(Debug, Deserialize, Serialize)]
pub struct Download {
    pub path: PathBuf,
    pub bytes: u64,
    pub sha256: String,
    pub repository: String,
    pub version: String,
    pub release_id: u64,
}

pub fn repository(value: &str) -> Result<&str> {
    let parts: Vec<_> = value.split('/').collect();
    ensure!(
        parts.len() == 2
            && parts.iter().all(|p| !p.is_empty()
                && p.len() <= 100
                && p.as_bytes()[0].is_ascii_alphanumeric()
                && p.bytes()
                    .all(|c| c.is_ascii_alphanumeric() || b"-_.".contains(&c)))
            && !parts.contains(&".")
            && !parts.contains(&".."),
        "Invalid GitHub repository"
    );
    Ok(value)
}
pub fn version(value: &str) -> Result<semver::Version> {
    let value = value.strip_prefix('v').unwrap_or(value);
    // Historic native builds used alpha.20-native. Compare their numeric alpha
    // component correctly (alpha.10 must sort after alpha.9).
    let value = value
        .strip_suffix("-native")
        .or_else(|| value.strip_suffix("-offline"))
        .unwrap_or(value);
    Ok(semver::Version::parse(value)?)
}
fn checked_url(value: &str, host: &str) -> Result<Url> {
    let url = Url::parse(value)?;
    ensure!(
        url.scheme() == "https"
            && url.host_str() == Some(host)
            && url.port().is_none()
            && url.username().is_empty()
            && url.password().is_none()
            && url.fragment().is_none(),
        "Untrusted update URL"
    );
    Ok(url)
}
fn asset_name(tag: &str, platform: &str) -> Result<String> {
    let suffix = match platform {
        "windows-x64" => "windows-x64-setup.exe",
        "macos-arm64" => "macos-arm64.dmg",
        _ => bail!("Unsupported update platform"),
    };
    let raw = tag.strip_prefix('v').unwrap_or(tag);
    version(raw)?;
    Ok(format!("Retriever-{raw}-{suffix}"))
}
pub fn validate(
    repo: &str,
    release: &Release,
    current: &str,
    platform: &str,
    preview: bool,
) -> Result<Update> {
    repository(repo)?;
    let new = version(&release.tag_name)?;
    ensure!(
        !release.draft && new > version(current)?,
        "Release is not newer than this installation"
    );
    ensure!(
        preview || (!release.prerelease && new.pre.is_empty()),
        "Preview updates are disabled"
    );
    let url = checked_url(&release.html_url, "github.com")?;
    ensure!(
        url.query().is_none() && url.path() == format!("/{repo}/releases/tag/{}", release.tag_name),
        "Release belongs to another repository"
    );
    let name = asset_name(&release.tag_name, platform)?;
    let matches: Vec<_> = release.assets.iter().filter(|a| a.name == name).collect();
    ensure!(
        matches.len() == 1,
        "Release has no unique installer for this platform"
    );
    let asset = matches[0];
    ensure!(
        asset.id > 0 && asset.state == "uploaded" && (1..=MAX_ASSET).contains(&asset.size),
        "Invalid installer asset"
    );
    let digest = asset
        .digest
        .as_deref()
        .and_then(|d| d.strip_prefix("sha256:"))
        .context("GitHub has not provided a SHA-256 digest for this installer")?;
    ensure!(
        digest.len() == 64 && digest.bytes().all(|b| b.is_ascii_hexdigit()),
        "Invalid installer checksum"
    );
    let download = checked_url(&asset.browser_download_url, "github.com")?;
    ensure!(
        download.query().is_none()
            && download.path()
                == format!(
                    "/{repo}/releases/download/{}/{}",
                    release.tag_name, asset.name
                ),
        "Installer belongs to another release"
    );
    Ok(Update {
        release_id: release.id,
        version: release.tag_name.trim_start_matches('v').into(),
        notes: release
            .body
            .as_deref()
            .unwrap_or("")
            .chars()
            .take(12_000)
            .collect(),
        release_url: release.html_url.clone(),
        asset: asset.clone(),
    })
}
pub fn select(
    repo: &str,
    releases: &[Release],
    current: &str,
    platform: &str,
    preview: bool,
) -> Result<Option<Update>> {
    repository(repo)?;
    version(current)?;
    asset_name(current, platform)?;
    Ok(releases
        .iter()
        .filter_map(|r| validate(repo, r, current, platform, preview).ok())
        .max_by_key(|r| version(&r.version).ok()))
}
fn client() -> Result<Client> {
    Ok(Client::builder()
        .user_agent(concat!("Retriever-Updater/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::none())
        .build()?)
}
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
fn save(path: &Path, value: &impl Serialize) -> Result<()> {
    std::fs::create_dir_all(path.parent().context("Missing cache directory")?)?;
    let mut temp = tempfile::NamedTempFile::new_in(path.parent().unwrap())?;
    serde_json::to_writer(&mut temp, value)?;
    temp.flush()?;
    temp.as_file().sync_all()?;
    temp.persist(path).map_err(|e| e.error)?;
    Ok(())
}
async fn json_body<T: serde::de::DeserializeOwned>(response: reqwest::Response) -> Result<T> {
    ensure!(
        response.content_length().unwrap_or(0) <= MAX_METADATA as u64,
        "Update metadata is too large"
    );
    let mut stream = response.bytes_stream();
    let mut bytes = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        ensure!(
            bytes.len() + chunk.len() <= MAX_METADATA,
            "Update metadata is too large"
        );
        bytes.extend_from_slice(&chunk);
    }
    Ok(serde_json::from_slice(&bytes)?)
}
pub async fn check(
    repo: &str,
    current: &str,
    platform: &str,
    preview: bool,
    cache_path: &Path,
    force: bool,
) -> Result<Check> {
    repository(repo)?;
    version(current)?;
    let mut cache: Cache = std::fs::metadata(cache_path)
        .ok()
        .filter(|m| m.len() <= MAX_METADATA as u64)
        .and_then(|_| std::fs::read(cache_path).ok())
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default();
    if cache.repository != repo {
        cache = Cache {
            repository: repo.into(),
            ..Default::default()
        };
    }
    ensure!(
        cache.retry_after_until <= now(),
        "GitHub asked Retriever to wait before checking again. Try later"
    );
    let mut cached = true;
    if force || cache.next_check <= now() {
        let client = client()?;
        let mut request = client
            .get(format!(
                "https://api.github.com/repos/{repo}/releases?per_page=100"
            ))
            .header(header::ACCEPT, "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2026-03-10");
        if let Some(etag) = &cache.etag {
            request = request.header(header::IF_NONE_MATCH, etag);
        }
        let response = request
            .send()
            .await
            .context("Could not reach GitHub. Your files and search are unaffected")?;
        match response.status() {
            StatusCode::NOT_MODIFIED => {}
            StatusCode::OK => {
                cache.etag = response
                    .headers()
                    .get(header::ETAG)
                    .and_then(|v| v.to_str().ok())
                    .map(String::from);
                cache.releases = json_body(response).await?;
                cached = false;
            }
            StatusCode::NOT_FOUND => {
                cache.releases.clear();
                cache.etag = None;
            }
            StatusCode::FORBIDDEN | StatusCode::TOO_MANY_REQUESTS => {
                let wait = response
                    .headers()
                    .get(header::RETRY_AFTER)
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| v.parse::<u64>().ok())
                    .unwrap_or(3600)
                    .clamp(60, 86400);
                cache.retry_after_until = now() + wait;
                cache.next_check = cache.retry_after_until;
                save(cache_path, &cache)?;
                bail!("GitHub asked Retriever to wait before checking again. Try later");
            }
            status => bail!("GitHub update check returned {status}"),
        }
        cache.retry_after_until = 0;
        cache.next_check = now() + INTERVAL;
        save(cache_path, &cache)?;
    }
    let update = select(repo, &cache.releases, current, platform, preview)?;
    Ok(Check {
        available: update.is_some(),
        update,
        cached,
        next_check: cache.next_check,
        message: if cache.releases.is_empty() {
            "No published releases are available yet"
        } else {
            "Update check complete"
        }
        .into(),
    })
}
async fn asset_response(client: &Client, initial: &str) -> Result<reqwest::Response> {
    let mut url = checked_url(initial, "github.com")?;
    for _ in 0..5 {
        let response = client
            .get(url.clone())
            .timeout(Duration::from_secs(1800))
            .send()
            .await?;
        if response.status().is_redirection() {
            let next = url.join(
                response
                    .headers()
                    .get(header::LOCATION)
                    .context("Missing download redirect")?
                    .to_str()?,
            )?;
            let host = next.host_str().unwrap_or("");
            ensure!(
                matches!(
                    host,
                    "github.com"
                        | "release-assets.githubusercontent.com"
                        | "objects.githubusercontent.com"
                ),
                "Untrusted download redirect"
            );
            checked_url(next.as_str(), host)?;
            url = next;
            continue;
        }
        return Ok(response.error_for_status()?);
    }
    bail!("Too many download redirects")
}
pub async fn download(
    repo: &str,
    release_id: u64,
    current: &str,
    platform: &str,
    preview: bool,
    output: &Path,
    progress: impl Fn(u64, u64),
    cancel: impl std::future::Future<Output = ()>,
) -> Result<Download> {
    // Cancellation covers metadata and response headers as well as the body.
    // Dropping the inner future also drops any uncommitted temporary download.
    tokio::select! {
        biased;
        _ = cancel => bail!("Download cancelled"),
        result = download_inner(repo, release_id, current, platform, preview, output, progress) => result,
    }
}
async fn download_inner(
    repo: &str,
    release_id: u64,
    current: &str,
    platform: &str,
    preview: bool,
    output: &Path,
    progress: impl Fn(u64, u64),
) -> Result<Download> {
    repository(repo)?;
    ensure!(release_id > 0, "Missing release identity");
    let client = client()?;
    // Re-fetch before downloading: never execute a URL/receipt supplied by a UI,
    // an editable local cache, or an out-of-date release notification.
    let release: Release = json_body(
        client
            .get(format!(
                "https://api.github.com/repos/{repo}/releases/{release_id}"
            ))
            .header(header::ACCEPT, "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2026-03-10")
            .send()
            .await?
            .error_for_status()?,
    )
    .await?;
    let update = validate(repo, &release, current, platform, preview)?;
    let response = asset_response(&client, &update.asset.browser_download_url).await?;
    ensure!(
        response
            .content_length()
            .is_none_or(|n| n == update.asset.size),
        "Installer size changed"
    );
    commit_download(
        repo,
        &update,
        output,
        response.bytes_stream(),
        progress,
        std::future::pending(),
    )
    .await
}
async fn commit_download<B: AsRef<[u8]>, E: Into<anyhow::Error>>(
    repo: &str,
    update: &Update,
    output: &Path,
    stream: impl futures_util::Stream<Item = std::result::Result<B, E>>,
    progress: impl Fn(u64, u64),
    cancel: impl std::future::Future<Output = ()>,
) -> Result<Download> {
    std::fs::create_dir_all(output)?;
    let mut temporary = tempfile::NamedTempFile::new_in(output)?;
    let mut hash = Sha256::new();
    let mut bytes = 0u64;
    let mut percent = 0;
    tokio::pin!(stream);
    tokio::pin!(cancel);
    loop {
        let chunk = tokio::select! { _=&mut cancel => bail!("Download cancelled"), chunk=stream.next()=>chunk };
        let Some(chunk) = chunk else { break };
        let chunk = chunk.map_err(Into::into)?;
        let chunk = chunk.as_ref();
        bytes += chunk.len() as u64;
        ensure!(
            bytes <= update.asset.size,
            "Installer exceeds the declared size"
        );
        temporary.write_all(chunk)?;
        hash.update(chunk);
        let next = bytes * 100 / update.asset.size;
        if next != percent {
            percent = next;
            progress(bytes, update.asset.size);
        }
    }
    let digest = hex::encode(hash.finalize());
    ensure!(
        bytes == update.asset.size
            && update
                .asset
                .digest
                .as_deref()
                .is_some_and(|expected| expected.eq_ignore_ascii_case(&format!("sha256:{digest}"))),
        "Installer checksum or size mismatch; nothing was installed"
    );
    temporary.flush()?;
    temporary.as_file().sync_all()?;
    let path = output.join(&update.asset.name);
    temporary
        .persist_noclobber(&path)
        .map_err(|e| e.error)
        .context("Download already exists; choose a new download directory")?;
    let receipt = Download {
        path: path.canonicalize()?,
        bytes,
        sha256: digest,
        repository: repo.into(),
        version: update.version.clone(),
        release_id: update.release_id,
    };
    save(&output.join("download.json"), &receipt)?;
    Ok(receipt)
}
pub fn verify(receipt_path: &Path, repo: &str) -> Result<Download> {
    let receipt: Download = serde_json::from_slice(&std::fs::read(receipt_path)?)?;
    ensure!(
        receipt.repository == repository(repo)?,
        "Download belongs to another repository"
    );
    let directory = receipt_path
        .parent()
        .context("Missing download folder")?
        .canonicalize()?;
    ensure!(
        receipt.path.canonicalize()?.parent() == Some(directory.as_path()),
        "Download escaped its staging directory"
    );
    let mut file = std::fs::File::open(&receipt.path)?;
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    let mut size = 0;
    loop {
        let n = file.read(&mut buffer)?;
        if n == 0 {
            break;
        };
        size += n as u64;
        hash.update(&buffer[..n]);
    }
    ensure!(
        size == receipt.bytes && hex::encode(hash.finalize()) == receipt.sha256,
        "Downloaded installer changed; download it again"
    );
    Ok(receipt)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn release(tag: &str) -> Release {
        let name = format!(
            "Retriever-{}-windows-x64-setup.exe",
            tag.trim_start_matches('v')
        );
        Release {
            id: 7,
            tag_name: tag.into(),
            draft: false,
            prerelease: tag.contains('-'),
            html_url: format!("https://github.com/example/retriever/releases/tag/{tag}"),
            body: None,
            assets: vec![Asset {
                id: 8,
                name: name.clone(),
                size: 123,
                state: "uploaded".into(),
                browser_download_url: format!(
                    "https://github.com/example/retriever/releases/download/{tag}/{name}"
                ),
                digest: Some(format!("sha256:{}", "a".repeat(64))),
            }],
        }
    }
    #[test]
    fn historic_alpha_versions_sort_numerically() {
        assert!(
            version("0.13.0-alpha.10-native").unwrap() > version("0.13.0-alpha.9-native").unwrap()
        );
        assert!(version("v0.13.0").unwrap() > version("0.13.0-alpha.20-native").unwrap());
    }
    #[test]
    fn stable_channel_rejects_preview_even_when_flag_lies() {
        let mut r = release("v0.14.0-alpha.1");
        r.prerelease = false;
        assert!(validate("example/retriever", &r, "0.13.0", "windows-x64", false).is_err());
    }
    #[test]
    fn refuses_downgrade_and_reinstall() {
        for tag in ["v0.12.0", "v0.13.0"] {
            assert!(
                validate(
                    "example/retriever",
                    &release(tag),
                    "0.13.0",
                    "windows-x64",
                    true
                )
                .is_err()
            );
        }
    }
    #[test]
    fn chooses_highest_usable_release_not_api_order() {
        let rows = vec![
            release("v0.14.0"),
            release("v0.13.1"),
            release("v0.15.0-alpha.1"),
        ];
        assert_eq!(
            select("example/retriever", &rows, "0.13.0", "windows-x64", false)
                .unwrap()
                .unwrap()
                .version,
            "0.14.0"
        );
    }
    #[test]
    fn wrong_repository_redirect_and_credentials_are_rejected() {
        let mut r = release("v0.14.0");
        for url in [
            "https://github.com.evil.test/x",
            "https://github.com/other/repo/releases/download/v0.14.0/file.exe",
            "http://github.com/example/retriever/file.exe",
            "https://user:password@github.com/example/retriever/file.exe",
        ] {
            r.assets[0].browser_download_url = url.into();
            assert!(validate("example/retriever", &r, "0.13.0", "windows-x64", false).is_err());
        }
    }
    #[test]
    fn requires_checksum_platform_and_unique_asset() {
        let mut r = release("v0.14.0");
        assert!(validate("example/retriever", &r, "0.13.0", "macos-arm64", false).is_err());
        r.assets[0].digest = None;
        assert!(validate("example/retriever", &r, "0.13.0", "windows-x64", false).is_err());
        let mut r = release("v0.14.0");
        r.assets.push(r.assets[0].clone());
        assert!(validate("example/retriever", &r, "0.13.0", "windows-x64", false).is_err());
    }
    #[test]
    fn rejects_unbounded_or_draft_assets() {
        for size in [0, MAX_ASSET + 1] {
            let mut r = release("v0.14.0");
            r.assets[0].size = size;
            assert!(validate("example/retriever", &r, "0.13.0", "windows-x64", false).is_err());
        }
        let mut r = release("v0.14.0");
        r.draft = true;
        assert!(validate("example/retriever", &r, "0.13.0", "windows-x64", false).is_err());
    }
    #[test]
    fn detects_download_tampering() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("installer.exe");
        std::fs::write(&path, b"original").unwrap();
        let receipt = Download {
            path: path.clone(),
            bytes: 8,
            sha256: hex::encode(Sha256::digest(b"original")),
            repository: "example/retriever".into(),
            version: "1.0.0".into(),
            release_id: 1,
        };
        let receipt_path = dir.path().join("download.json");
        save(&receipt_path, &receipt).unwrap();
        assert!(verify(&receipt_path, "example/retriever").is_ok());
        std::fs::write(path, b"tampered").unwrap();
        assert!(verify(&receipt_path, "example/retriever").is_err());
    }
    #[test]
    fn repository_cannot_inject_url_or_path() {
        for repo in [
            "../repo",
            "owner/../repo",
            "owner/repo?token=x",
            "owner/repo#x",
            "owner/repo%2fother",
            "/repo",
        ] {
            assert!(repository(repo).is_err());
        }
    }
    #[tokio::test]
    async fn forced_check_honors_rate_limit_without_network() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cache.json");
        save(
            &path,
            &Cache {
                repository: "example/retriever".into(),
                retry_after_until: now() + 600,
                ..Default::default()
            },
        )
        .unwrap();
        let error = check(
            "example/retriever",
            "0.13.0",
            "windows-x64",
            false,
            &path,
            true,
        )
        .await
        .unwrap_err();
        assert!(error.to_string().contains("wait before checking"));
    }
    #[tokio::test]
    async fn cached_metadata_reselects_channel_without_network() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cache.json");
        save(
            &path,
            &Cache {
                repository: "example/retriever".into(),
                next_check: now() + 600,
                releases: vec![release("v0.14.0"), release("v0.15.0-alpha.1")],
                ..Default::default()
            },
        )
        .unwrap();
        let stable = check(
            "example/retriever",
            "0.13.0",
            "windows-x64",
            false,
            &path,
            false,
        )
        .await
        .unwrap();
        assert!(stable.cached);
        assert_eq!(stable.update.unwrap().version, "0.14.0");
        let preview = check(
            "example/retriever",
            "0.13.0",
            "windows-x64",
            true,
            &path,
            false,
        )
        .await
        .unwrap();
        assert_eq!(preview.update.unwrap().version, "0.15.0-alpha.1");
    }
    fn download_fixture() -> Update {
        let mut r = release("v0.14.0");
        r.assets[0].size = 8;
        r.assets[0].digest = Some(format!(
            "sha256:{}",
            hex::encode(Sha256::digest(b"original"))
        ));
        validate("example/retriever", &r, "0.13.0", "windows-x64", false).unwrap()
    }
    #[tokio::test]
    async fn streamed_download_commits_verified_bytes_only() {
        let dir = tempfile::tempdir().unwrap();
        let chunks = futures_util::stream::iter([
            Ok::<_, anyhow::Error>(b"orig".to_vec()),
            Ok(b"inal".to_vec()),
        ]);
        let result = commit_download(
            "example/retriever",
            &download_fixture(),
            dir.path(),
            chunks,
            |_, _| {},
            std::future::pending(),
        )
        .await
        .unwrap();
        assert_eq!(std::fs::read(result.path).unwrap(), b"original");
        assert!(verify(&dir.path().join("download.json"), "example/retriever").is_ok());
    }
    #[tokio::test]
    async fn truncated_corrupt_and_oversize_downloads_leave_no_installer() {
        for bytes in [
            b"short".to_vec(),
            b"tampered".to_vec(),
            b"too many bytes".to_vec(),
        ] {
            let dir = tempfile::tempdir().unwrap();
            let chunks = futures_util::stream::iter([Ok::<_, anyhow::Error>(bytes)]);
            assert!(
                commit_download(
                    "example/retriever",
                    &download_fixture(),
                    dir.path(),
                    chunks,
                    |_, _| {},
                    std::future::pending()
                )
                .await
                .is_err()
            );
            assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
        }
    }
    #[tokio::test]
    async fn cancellation_removes_partial_file() {
        let dir = tempfile::tempdir().unwrap();
        let chunks = futures_util::stream::pending::<Result<Vec<u8>>>();
        assert!(
            commit_download(
                "example/retriever",
                &download_fixture(),
                dir.path(),
                chunks,
                |_, _| {},
                std::future::ready(())
            )
            .await
            .unwrap_err()
            .to_string()
            .contains("cancelled")
        );
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }
    #[tokio::test]
    async fn cancelled_download_does_not_connect_or_create_output() {
        let dir = tempfile::tempdir().unwrap();
        let output = dir.path().join("download");
        let error = download(
            "example/retriever",
            1,
            "0.13.0",
            "windows-x64",
            false,
            &output,
            |_, _| {},
            std::future::ready(()),
        )
        .await
        .unwrap_err();
        assert_eq!(error.to_string(), "Download cancelled");
        assert!(!output.exists());
    }
    #[tokio::test]
    async fn existing_installer_is_never_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let update = download_fixture();
        let path = dir.path().join(&update.asset.name);
        std::fs::write(&path, b"keep").unwrap();
        let chunks = futures_util::stream::iter([Ok::<_, anyhow::Error>(b"original".to_vec())]);
        assert!(
            commit_download(
                "example/retriever",
                &update,
                dir.path(),
                chunks,
                |_, _| {},
                std::future::pending()
            )
            .await
            .is_err()
        );
        assert_eq!(std::fs::read(path).unwrap(), b"keep");
        assert!(!dir.path().join("download.json").exists());
    }
}
