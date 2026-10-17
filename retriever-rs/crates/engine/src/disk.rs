//! Independent, metadata-only disk accounting. No content reads, model calls,
//! deletion operations, or writes to the scanned tree. Search uses a different DB.
use anyhow::{Context, Result, bail};
use parking_lot::Mutex;
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::HashSet,
    fs,
    path::{Component, Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

const MAX_ENTRIES: u64 = 2_000_000;
const MAX_DEPTH: usize = 128;
const PAGE: i64 = 200;

pub struct DiskSpace {
    path: PathBuf,
    excluded: PathBuf,
    active: Mutex<Option<(String, Arc<AtomicBool>)>>,
}
#[derive(Default, Clone, Serialize, Deserialize)]
struct Totals {
    logical: i64,
    allocated: i64,
    files: i64,
    folders: i64,
    allocation_unknown: i64,
    hardlinks: i64,
}
impl Totals {
    fn add(&mut self, other: &Self) {
        self.logical = self.logical.saturating_add(other.logical);
        self.allocated = self.allocated.saturating_add(other.allocated);
        self.files += other.files;
        self.folders += other.folders;
        self.allocation_unknown += other.allocation_unknown;
        self.hardlinks += other.hardlinks;
    }
}
fn connect(path: &Path) -> Result<Connection> {
    let db = Connection::open(path)?;
    db.busy_timeout(Duration::from_secs(5))?;
    db.execute_batch("PRAGMA foreign_keys=ON; PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL; PRAGMA cache_size=-8192;")?;
    Ok(db)
}
fn display(path: &Path) -> String {
    let value = path.to_string_lossy().into_owned();
    #[cfg(windows)]
    {
        return value.strip_prefix(r"\\?\").unwrap_or(&value).to_owned();
    }
    #[cfg(not(windows))]
    {
        value
    }
}
fn row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Value> {
    Ok(
        json!({"id":row.get::<_,i64>(0)?,"parent":row.get::<_,i64>(1)?,"name":row.get::<_,String>(2)?,
        "path":row.get::<_,String>(3)?,"kind":row.get::<_,String>(4)?,"logical":row.get::<_,i64>(5)?,
        "allocated":row.get::<_,i64>(6)?,"files":row.get::<_,i64>(7)?,"folders":row.get::<_,i64>(8)?,
        "allocation_unknown":row.get::<_,i64>(9)?,"modified":row.get::<_,f64>(10)?,
        "extension":row.get::<_,String>(11)?,"warning":row.get::<_,String>(12)?}),
    )
}
const COLUMNS: &str = "id,parent,name,path,kind,logical,allocated,files,folders,allocation_unknown,modified,extension,warning";

impl DiskSpace {
    pub fn open(state: &Path) -> Result<Arc<Self>> {
        let path = state.join("disk-space.sqlite");
        crate::paths::reject_links(&path)?;
        let db = connect(&path)?;
        db.execute_batch("CREATE TABLE IF NOT EXISTS disk_scans(id TEXT PRIMARY KEY,root TEXT NOT NULL,started REAL NOT NULL,finished REAL,state TEXT NOT NULL,summary TEXT NOT NULL,error TEXT NOT NULL DEFAULT '');
            CREATE TABLE IF NOT EXISTS disk_nodes(scan TEXT NOT NULL REFERENCES disk_scans(id) ON DELETE CASCADE,id INTEGER NOT NULL,parent INTEGER NOT NULL,name TEXT NOT NULL,path TEXT NOT NULL,kind TEXT NOT NULL,logical INTEGER NOT NULL DEFAULT 0,allocated INTEGER NOT NULL DEFAULT 0,files INTEGER NOT NULL DEFAULT 0,folders INTEGER NOT NULL DEFAULT 0,allocation_unknown INTEGER NOT NULL DEFAULT 0,modified REAL NOT NULL DEFAULT 0,extension TEXT NOT NULL DEFAULT '',warning TEXT NOT NULL DEFAULT '',PRIMARY KEY(scan,id));
            CREATE INDEX IF NOT EXISTS disk_parent ON disk_nodes(scan,parent,logical DESC);
            CREATE INDEX IF NOT EXISTS disk_largest ON disk_nodes(scan,kind,logical DESC);
            CREATE INDEX IF NOT EXISTS disk_paths ON disk_nodes(scan,path);
            UPDATE disk_scans SET state='interrupted',error='Retriever stopped during this scan. Scan again for current totals.' WHERE state='scanning';")?;
        Ok(Arc::new(Self {
            path,
            excluded: state.canonicalize()?,
            active: Mutex::new(None),
        }))
    }
    pub fn status(&self) -> Result<Value> {
        let db = connect(&self.path)?;
        let mut stmt=db.prepare("SELECT id,root,started,finished,state,summary,error FROM disk_scans ORDER BY started DESC LIMIT 4")?;
        let scans = stmt
            .query_map([], scan_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(
            json!({"scans":scans,"active":self.active.lock().as_ref().map(|(id,_)|id),"read_only":true,"model_calls":0,"max_entries":MAX_ENTRIES}),
        )
    }
    pub fn start(self: &Arc<Self>, input: &str) -> Result<Value> {
        if input.is_empty() || input.len() > 32_768 || input.contains('\0') {
            bail!("Choose a local folder or drive");
        }
        let path = crate::paths::expand(input);
        if !path.is_absolute() || path.components().any(|c| matches!(c, Component::ParentDir)) {
            bail!("Choose an absolute folder without traversal");
        }
        #[cfg(windows)]
        if input.starts_with(r"\\") {
            bail!(
                "Disk analysis currently scans local folders and drives. Choose the folder on its source computer."
            );
        }
        // macOS user folders may have system symlink ancestors (e.g. /var).
        // The selected directory itself must never be a link or reparse point.
        let metadata = fs::symlink_metadata(&path)?;
        if is_link(&metadata) || !metadata.is_dir() {
            bail!("Choose a regular directory, not a link");
        }
        let path = path.canonicalize()?;
        if path.starts_with(&self.excluded) {
            bail!("Choose a folder outside Retriever's private state");
        }
        fs::read_dir(&path).context("The selected folder cannot be read")?;
        let mut active = self.active.lock();
        if active.is_some() {
            bail!("conflict: A disk scan is already running. Cancel it before starting another.");
        }
        let id = uuid::Uuid::new_v4().to_string();
        let cancel = Arc::new(AtomicBool::new(false));
        let db = connect(&self.path)?;
        let root = display(&path);
        let initial = json!({"entries":0,"totals":Totals::default(),"complete":false,"elapsed_ms":0,"model_calls":0});
        db.execute(
            "INSERT INTO disk_scans(id,root,started,state,summary) VALUES(?,?,?,'scanning',?)",
            params![id, root, crate::now(), initial.to_string()],
        )?;
        // Retain a small number of snapshots for growth comparisons. This only
        // removes analysis records, never user files or the search catalogue.
        db.execute("DELETE FROM disk_scans WHERE id NOT IN (SELECT id FROM disk_scans ORDER BY started DESC LIMIT 4)",[])?;
        *active = Some((id.clone(), cancel.clone()));
        let service = self.clone();
        let worker_id = id.clone();
        if let Err(error) = std::thread::Builder::new()
            .name("retriever-disk-scan".into())
            .spawn(move || {
                let result = service.scan(&worker_id, &path, cancel);
                if let Err(error) = result {
                    if let Ok(db) = connect(&service.path) {
                        let _ = db.execute(
                            "UPDATE disk_scans SET state='failed',finished=?,error=? WHERE id=?",
                            params![crate::now(), error.to_string(), worker_id],
                        );
                    }
                }
                let mut active = service.active.lock();
                if active.as_ref().is_some_and(|(id, _)| id == &worker_id) {
                    *active = None;
                }
            })
        {
            *active = None;
            db.execute(
                "UPDATE disk_scans SET state='failed',error=? WHERE id=?",
                params![error.to_string(), id],
            )?;
            return Err(error.into());
        }
        Ok(json!({"id":id,"root":root,"state":"scanning","read_only":true}))
    }
    pub fn cancel(&self, id: &str) -> Result<Value> {
        uuid::Uuid::parse_str(id)?;
        let active = self.active.lock();
        if let Some((current, cancel)) = &*active {
            if current == id {
                cancel.store(true, Ordering::Relaxed);
                return Ok(json!({"cancelling":true}));
            }
        }
        Ok(json!({"cancelling":false}))
    }
    fn scan(&self, id: &str, root: &Path, cancel: Arc<AtomicBool>) -> Result<()> {
        let db = connect(&self.path)?;
        db.execute_batch("BEGIN IMMEDIATE")?;
        let metadata = fs::metadata(root)?;
        let mut scan = Scan {
            db,
            id: id.to_owned(),
            excluded: self.excluded.clone(),
            cancel,
            started: Instant::now(),
            flushed: Instant::now(),
            entries: 0,
            totals: Totals::default(),
            errors: 0,
            links: 0,
            mounts: 0,
            excluded_count: 0,
            cloud: 0,
            hardlinks: HashSet::new(),
            stop: String::new(),
            current: display(root),
            samples: Vec::new(),
            root_device: device_id(&metadata),
            volume: volume(root),
        };
        let result = scan.walk(root, 0, 0);
        if result.is_err() {
            let _ = scan.db.execute_batch("ROLLBACK");
            return result.map(|_| ());
        }
        let state = if scan.stop == "cancelled" {
            "cancelled"
        } else if scan.stop.is_empty() {
            "complete"
        } else {
            "partial"
        };
        scan.save_progress(state)?;
        scan.db.execute_batch("COMMIT")?;
        Ok(())
    }
    pub fn view(&self, b: &Value) -> Result<Value> {
        let id = b["id"].as_str().context("Choose a disk scan")?;
        uuid::Uuid::parse_str(id)?;
        let node = b["node"].as_i64().unwrap_or(1);
        let offset = b["offset"]
            .as_i64()
            .unwrap_or(0)
            .clamp(0, MAX_ENTRIES as i64);
        let mode = b["mode"].as_str().unwrap_or("children");
        if !["children", "largest", "older", "growth", "types"].contains(&mode) {
            bail!("Unknown disk view");
        }
        let db = connect(&self.path)?;
        let scan = db
            .query_row(
                "SELECT id,root,started,finished,state,summary,error FROM disk_scans WHERE id=?",
                [id],
                scan_row,
            )
            .optional()?
            .context("This scan is no longer retained. Scan again.")?;
        let current = db
            .query_row(
                &format!("SELECT {COLUMNS} FROM disk_nodes WHERE scan=? AND id=?"),
                params![id, node],
                row,
            )
            .optional()?;
        let Some(current) = current else {
            return Ok(
                json!({"scan":scan,"current":null,"entries":[],"treemap":[],"types":[],"total":0,"offset":0,"next_offset":null,"insights":[]}),
            );
        };
        if current["kind"] != "folder" {
            bail!("Choose a folder to drill into");
        }
        let prefix = format!(
            "{}{}",
            current["path"]
                .as_str()
                .unwrap_or("")
                .trim_end_matches(['/', '\\']),
            std::path::MAIN_SEPARATOR
        );
        let previous: Option<String> = if scan["summary"]["complete"] == true {
            db.query_row("SELECT id FROM disk_scans WHERE root=? AND id<>? AND state='complete' AND json_extract(summary,'$.complete')=1 AND started<? ORDER BY started DESC LIMIT 1",params![scan["root"].as_str(),id,scan["started"].as_f64()],|r|r.get(0)).optional()?
        } else {
            None
        };
        let scope = if mode == "children" || mode == "growth" {
            "parent=:node"
        } else {
            "kind='file' AND substr(path,1,length(:prefix))=:prefix"
        };
        let old = if mode == "older" {
            " AND modified>0 AND modified<:cutoff"
        } else {
            " AND :cutoff>0"
        };
        let filters = format!("scan=:scan AND {scope}{old} AND :node>=1 AND length(:prefix)>0");
        let named = rusqlite::named_params! {":scan":id,":node":node,":prefix":prefix,":cutoff":crate::now()-365.0*86400.0};
        let total: i64 = db.query_row(
            &format!("SELECT count(*) FROM disk_nodes WHERE {filters}"),
            named,
            |r| r.get(0),
        )?;
        let mut stmt=db.prepare(&format!("SELECT {COLUMNS} FROM disk_nodes WHERE {filters} ORDER BY logical DESC,name COLLATE NOCASE,id LIMIT {PAGE} OFFSET {offset}"))?;
        let mut entries = stmt
            .query_map(named, row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        for entry in &mut entries {
            if let Some(previous) = &previous {
                let before: Option<i64> = db
                    .query_row(
                        "SELECT logical FROM disk_nodes WHERE scan=? AND path=?",
                        params![previous, entry["path"].as_str()],
                        |r| r.get(0),
                    )
                    .optional()?;
                entry["delta"] =
                    json!(entry["logical"].as_i64().unwrap_or(0) - before.unwrap_or(0));
            }
        }
        // Growth is a page of children sorted by current size, with factual deltas;
        // never label missing or incomplete baselines as zero-byte snapshots.
        let mut types_stmt=db.prepare("SELECT extension,sum(logical),sum(allocated),count(*) FROM disk_nodes WHERE scan=? AND kind='file' AND substr(path,1,length(?))=? GROUP BY extension ORDER BY sum(logical) DESC LIMIT 32")?;
        let types=types_stmt.query_map(params![id,prefix,prefix],|r|Ok(json!({"extension":r.get::<_,String>(0)?,"logical":r.get::<_,i64>(1)?,"allocated":r.get::<_,i64>(2)?,"files":r.get::<_,i64>(3)?})))?.collect::<rusqlite::Result<Vec<_>>>()?;
        let mut map_stmt=db.prepare("SELECT id,name,kind,logical FROM disk_nodes WHERE scan=? AND parent=? AND logical>0 ORDER BY logical DESC,name LIMIT 48")?;
        let mut tiles=map_stmt.query_map(params![id,node],|r|Ok(json!({"id":r.get::<_,i64>(0)?,"name":r.get::<_,String>(1)?,"kind":r.get::<_,String>(2)?,"logical":r.get::<_,i64>(3)?})))?.collect::<rusqlite::Result<Vec<_>>>()?;
        let measured: i64 = tiles
            .iter()
            .map(|t| t["logical"].as_i64().unwrap_or(0))
            .sum();
        let remainder = current["logical"]
            .as_i64()
            .unwrap_or(0)
            .saturating_sub(measured);
        if remainder > 0 {
            tiles.push(json!({"id":0,"name":"Other items","kind":"other","logical":remainder}));
        }
        layout(&mut tiles, 0.0, 0.0, 1.0, 1.0, 3.4);
        let mut insights = vec![
            json!({"kind":"accounting","text":"Logical size counts file entries. Allocated space counts each detected hard link once; shared APFS blocks and snapshots are not reclaimable-byte estimates."}),
        ];
        if let Some(top) = tiles.first() {
            let all = current["logical"].as_f64().unwrap_or(0.0);
            if all > 0.0 {
                insights.push(json!({"kind":"largest","text":format!("{} accounts for {:.1}% of this folder's measured size.",top["name"].as_str().unwrap_or("Largest item"),top["logical"].as_f64().unwrap_or(0.0)*100.0/all)}));
            }
        }
        if mode == "older" {
            insights.push(json!({"kind":"age","text":"Not modified in over a year does not mean unused. Review files before making changes."}));
        }
        if previous.is_none() {
            insights.push(json!({"kind":"baseline","text":"A second complete scan of this root enables size-change comparisons."}));
        }
        Ok(
            json!({"scan":scan,"current":current,"entries":entries,"treemap":tiles,"types":types,"total":total,"offset":offset,"next_offset":if offset+PAGE<total{Some(offset+PAGE)}else{None},"previous_scan":previous,"insights":insights,"read_only":true,"model_calls":0}),
        )
    }
}
fn scan_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Value> {
    let text: String = r.get(5)?;
    Ok(
        json!({"id":r.get::<_,String>(0)?,"root":r.get::<_,String>(1)?,"started":r.get::<_,f64>(2)?,"finished":r.get::<_,Option<f64>>(3)?,"state":r.get::<_,String>(4)?,"summary":serde_json::from_str::<Value>(&text).unwrap_or(json!({})),"error":r.get::<_,String>(6)?}),
    )
}
struct Scan {
    db: Connection,
    id: String,
    excluded: PathBuf,
    cancel: Arc<AtomicBool>,
    started: Instant,
    flushed: Instant,
    entries: u64,
    totals: Totals,
    errors: u64,
    links: u64,
    mounts: u64,
    excluded_count: u64,
    cloud: u64,
    hardlinks: HashSet<(u64, u64)>,
    stop: String,
    current: String,
    samples: Vec<Value>,
    root_device: u64,
    volume: Value,
}
impl Scan {
    fn keep_going(&mut self) -> bool {
        if self.cancel.load(Ordering::Relaxed) {
            self.stop = "cancelled".into();
        }
        if self.entries >= MAX_ENTRIES && self.stop.is_empty() {
            self.stop = "entry_limit".into();
        }
        if self.started.elapsed() > Duration::from_secs(3600) && self.stop.is_empty() {
            self.stop = "time_limit".into();
        }
        self.stop.is_empty()
    }
    fn issue(&mut self, path: &Path, error: impl ToString) {
        self.errors += 1;
        if self.samples.len() < 20 {
            self.samples
                .push(json!({"path":display(path),"error":error.to_string()}));
        }
    }
    fn progress(&mut self) -> Result<()> {
        if self.entries % 256 == 0 || self.flushed.elapsed() > Duration::from_millis(300) {
            self.save_progress("scanning")?;
            self.db.execute_batch("COMMIT; BEGIN IMMEDIATE;")?;
            self.flushed = Instant::now();
            std::thread::yield_now();
        }
        Ok(())
    }
    fn save_progress(&self, state: &str) -> Result<()> {
        let value = json!({"entries":self.entries,"totals":self.totals,"errors":self.errors,"links_skipped":self.links,
            "mounts_skipped":self.mounts,"private_state_skipped":self.excluded_count,"cloud_placeholders":self.cloud,
            "error_samples":self.samples,"current":self.current,"stop_reason":self.stop,
            "complete":state=="complete"&&self.errors==0,"elapsed_ms":self.started.elapsed().as_millis(),"volume":self.volume,"model_calls":0});
        self.db.execute(
            "UPDATE disk_scans SET state=?,summary=?,finished=? WHERE id=?",
            params![
                state,
                value.to_string(),
                if state == "scanning" {
                    None
                } else {
                    Some(crate::now())
                },
                self.id
            ],
        )?;
        Ok(())
    }
    fn walk(&mut self, path: &Path, parent: i64, depth: usize) -> Result<Totals> {
        if !self.keep_going() {
            return Ok(Totals::default());
        }
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .map(str::to_owned)
            .unwrap_or_else(|| display(path));
        let Some(path_text) = path.to_str() else {
            self.issue(path, "Filename is not valid Unicode");
            return Ok(Totals::default());
        };
        let _ = path_text;
        let metadata = match fs::symlink_metadata(path) {
            Ok(m) => m,
            Err(e) => {
                self.issue(path, e);
                return Ok(Totals::default());
            }
        };
        if path.starts_with(&self.excluded) {
            self.excluded_count += 1;
            return Ok(Totals::default());
        }
        if is_link(&metadata) {
            self.links += 1;
            return Ok(Totals::default());
        }
        if metadata.is_dir() && device_id(&metadata) != self.root_device {
            self.mounts += 1;
            return Ok(Totals::default());
        }
        if !metadata.is_file() && !metadata.is_dir() {
            return Ok(Totals::default());
        }
        self.entries += 1;
        let id = self.entries as i64;
        self.current = display(path);
        let modified = metadata
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs_f64())
            .unwrap_or(0.0);
        let kind = if metadata.is_dir() { "folder" } else { "file" };
        let extension = if metadata.is_file() {
            path.extension()
                .and_then(|v| v.to_str())
                .unwrap_or("")
                .to_lowercase()
        } else {
            String::new()
        };
        self.db.prepare_cached("INSERT INTO disk_nodes(scan,id,parent,name,path,kind,modified,extension) VALUES(?,?,?,?,?,?,?,?)")?.execute(params![self.id,id,parent,name,display(path),kind,modified,extension])?;
        let mut totals = Totals::default();
        let mut warning = String::new();
        if metadata.is_dir() {
            if depth >= MAX_DEPTH {
                warning = "Folder depth limit reached".into();
                self.issue(path, &warning);
            } else {
                match fs::read_dir(path) {
                    Ok(entries) => {
                        for entry in entries {
                            if !self.keep_going() {
                                break;
                            }
                            match entry {
                                Ok(entry) => {
                                    totals.add(&self.walk(&entry.path(), id, depth + 1)?)
                                }
                                Err(e) => self.issue(path, e),
                            }
                        }
                    }
                    Err(e) => {
                        warning = e.to_string();
                        self.issue(path, &warning);
                    }
                }
            }
            totals.folders += 1;
            self.totals.folders += 1;
        } else {
            totals.logical = metadata.len().min(i64::MAX as u64) as i64;
            totals.files = 1;
            let a = allocation(path, &metadata);
            if a.cloud {
                self.cloud += 1;
                warning = "Cloud placeholder; contents were not downloaded".into();
            }
            match a.bytes {
                Some(bytes) => totals.allocated = bytes.min(i64::MAX as u64) as i64,
                None => {
                    totals.allocation_unknown = 1;
                    if warning.is_empty() {
                        warning = "Allocated size unavailable".into();
                    }
                }
            }
            if let Some(key) = a.identity {
                if !self.hardlinks.insert(key) {
                    totals.allocated = 0;
                    totals.hardlinks = 1;
                    warning =
                        "Hard link; allocated bytes counted at the first encountered path".into();
                }
            }
            self.totals.add(&totals);
        }
        self.db.prepare_cached("UPDATE disk_nodes SET logical=?,allocated=?,files=?,folders=?,allocation_unknown=?,warning=? WHERE scan=? AND id=?")?.execute(params![totals.logical,totals.allocated,totals.files,totals.folders,totals.allocation_unknown,warning,self.id,id])?;
        self.progress()?;
        Ok(totals)
    }
}

struct Allocation {
    bytes: Option<u64>,
    identity: Option<(u64, u64)>,
    cloud: bool,
}
fn is_link(m: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        m.file_type().is_symlink() || m.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    {
        m.file_type().is_symlink()
    }
}
fn device_id(m: &fs::Metadata) -> u64 {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        m.dev()
    }
    #[cfg(not(unix))]
    {
        let _ = m;
        0
    }
}
#[cfg(unix)]
fn allocation(_path: &Path, m: &fs::Metadata) -> Allocation {
    use std::os::unix::fs::MetadataExt;
    Allocation {
        bytes: Some(m.blocks().saturating_mul(512)),
        identity: (m.nlink() > 1).then_some((m.dev(), m.ino())),
        cloud: false,
    }
}
#[cfg(windows)]
fn allocation(path: &Path, m: &fs::Metadata) -> Allocation {
    use std::os::windows::{ffi::OsStrExt, fs::MetadataExt};
    let unknown = || Allocation {
        bytes: None,
        identity: None,
        cloud: false,
    };
    if m.file_attributes() & (0x1000 | 0x00400000 | 0x00040000) != 0 {
        return Allocation {
            cloud: true,
            ..unknown()
        };
    }
    #[repr(C)]
    struct Standard {
        allocated: i64,
        size: i64,
        links: u32,
        delete: u8,
        directory: u8,
    }
    #[repr(C)]
    struct Info {
        attributes: u32,
        created: [u32; 2],
        accessed: [u32; 2],
        written: [u32; 2],
        volume: u32,
        high: u32,
        low: u32,
        links: u32,
        index_high: u32,
        index_low: u32,
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn CreateFileW(
            name: *const u16,
            access: u32,
            share: u32,
            security: *const std::ffi::c_void,
            creation: u32,
            flags: u32,
            template: isize,
        ) -> isize;
        fn GetFileInformationByHandleEx(
            handle: isize,
            class: u32,
            buffer: *mut std::ffi::c_void,
            size: u32,
        ) -> i32;
        fn GetFileInformationByHandle(handle: isize, info: *mut Info) -> i32;
        fn CloseHandle(handle: isize) -> i32;
    }
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    // Metadata access only; OPEN_REPARSE_POINT prevents following a swapped file link.
    let handle = unsafe { CreateFileW(wide.as_ptr(), 0x80, 7, std::ptr::null(), 3, 0x00200000, 0) };
    if handle == -1 {
        return unknown();
    }
    let mut standard: Standard = unsafe { std::mem::zeroed() };
    let mut identity = None;
    let ok = unsafe {
        GetFileInformationByHandleEx(
            handle,
            1,
            (&mut standard as *mut Standard).cast(),
            std::mem::size_of::<Standard>() as u32,
        )
    } != 0;
    if ok && standard.links > 1 {
        let mut info: Info = unsafe { std::mem::zeroed() };
        if unsafe { GetFileInformationByHandle(handle, &mut info) } != 0 {
            identity = Some((
                info.volume as u64,
                ((info.index_high as u64) << 32) | info.index_low as u64,
            ));
        }
    }
    unsafe {
        CloseHandle(handle);
    }
    Allocation {
        bytes: if ok && standard.allocated >= 0 {
            Some(standard.allocated as u64)
        } else {
            None
        },
        identity,
        cloud: false,
    }
}
#[cfg(not(any(unix, windows)))]
fn allocation(_path: &Path, _m: &fs::Metadata) -> Allocation {
    Allocation {
        bytes: None,
        identity: None,
        cloud: false,
    }
}

fn volume(path: &Path) -> Value {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn GetDiskFreeSpaceExW(
                path: *const u16,
                available: *mut u64,
                total: *mut u64,
                free: *mut u64,
            ) -> i32;
        }
        let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        let (mut available, mut total, mut free) = (0, 0, 0);
        if unsafe { GetDiskFreeSpaceExW(wide.as_ptr(), &mut available, &mut total, &mut free) } != 0
        {
            return json!({"available":available,"total":total,"free":free});
        }
    }
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        if let Ok(path) = std::ffi::CString::new(path.as_os_str().as_bytes()) {
            let mut v: libc::statvfs = unsafe { std::mem::zeroed() };
            if unsafe { libc::statvfs(path.as_ptr(), &mut v) } == 0 {
                let size = v.f_frsize as u64;
                return json!({"available":(v.f_bavail as u64).saturating_mul(size),"total":(v.f_blocks as u64).saturating_mul(size),"free":(v.f_bfree as u64).saturating_mul(size)});
            }
        }
    }
    json!(null)
}

// Balanced rectangular partition, independently implemented. Every item keeps
// its proportional area; the frontends share these normalized coordinates.
fn layout(items: &mut [Value], x: f64, y: f64, w: f64, h: f64, aspect: f64) {
    if items.is_empty() {
        return;
    }
    if items.len() == 1 {
        items[0]["rect"] = json!({"x":x,"y":y,"width":w,"height":h});
        return;
    }
    let total: f64 = items
        .iter()
        .map(|v| v["logical"].as_f64().unwrap_or(0.0))
        .sum();
    if total <= 0.0 {
        return;
    }
    let mut sum = 0.0;
    let mut split = 1;
    for (i, item) in items.iter().enumerate().take(items.len() - 1) {
        sum += item["logical"].as_f64().unwrap_or(0.0);
        split = i + 1;
        if sum >= total / 2.0 {
            break;
        }
    }
    let ratio = sum / total;
    let (a, b) = items.split_at_mut(split);
    if w * aspect >= h {
        layout(a, x, y, w * ratio, h, aspect);
        layout(b, x + w * ratio, y, w * (1.0 - ratio), h, aspect);
    } else {
        layout(a, x, y, w, h * ratio, aspect);
        layout(b, x, y + h * ratio, w, h * (1.0 - ratio), aspect);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn wait(service: &DiskSpace, id: &str) -> Value {
        for _ in 0..500 {
            let v = service.view(&json!({"id":id})).unwrap();
            if v["scan"]["state"] != "scanning" {
                return v;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("disk scan timed out")
    }
    #[test]
    fn accounting_drilldown_types_growth_and_persistence() {
        let state = tempfile::tempdir().unwrap();
        let root = tempfile::tempdir().unwrap();
        fs::create_dir(root.path().join("Folder")).unwrap();
        fs::write(root.path().join("a.txt"), vec![0; 100]).unwrap();
        fs::write(root.path().join("Folder/b.bin"), vec![0; 300]).unwrap();
        let service = DiskSpace::open(state.path()).unwrap();
        let start = service.start(root.path().to_str().unwrap()).unwrap();
        let id = start["id"].as_str().unwrap();
        let view = wait(&service, id);
        assert_eq!(view["scan"]["state"], "complete");
        assert_eq!(view["scan"]["summary"]["totals"]["logical"], 400);
        assert_eq!(view["scan"]["summary"]["totals"]["files"], 2);
        assert_eq!(view["current"]["logical"], 400);
        let child = view["entries"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["name"] == "Folder")
            .unwrap();
        let folder = service.view(&json!({"id":id,"node":child["id"]})).unwrap();
        assert_eq!(folder["entries"][0]["name"], "b.bin");
        let largest = service.view(&json!({"id":id,"mode":"largest"})).unwrap();
        assert_eq!(largest["entries"][0]["name"], "b.bin");
        assert_eq!(largest["types"].as_array().unwrap().len(), 2);
        while service.active.lock().is_some() {
            std::thread::yield_now();
        }
        fs::write(root.path().join("a.txt"), vec![0; 250]).unwrap();
        let second = service.start(root.path().to_str().unwrap()).unwrap();
        let second_id = second["id"].as_str().unwrap();
        let updated = wait(&service, second_id);
        assert_eq!(updated["scan"]["summary"]["totals"]["logical"], 550);
        assert_eq!(updated["previous_scan"], id);
        assert_eq!(
            updated["entries"]
                .as_array()
                .unwrap()
                .iter()
                .find(|e| e["name"] == "a.txt")
                .unwrap()["delta"],
            150
        );
        while service.active.lock().is_some() {
            std::thread::yield_now();
        }
        drop(service);
        let reopened = DiskSpace::open(state.path()).unwrap();
        assert_eq!(
            reopened.view(&json!({"id":second_id})).unwrap()["current"]["logical"],
            550
        );
    }
    #[test]
    fn hardlinks_are_not_double_counted_as_allocated_space() {
        let state = tempfile::tempdir().unwrap();
        let root = tempfile::tempdir().unwrap();
        let original = root.path().join("a.txt");
        fs::write(&original, vec![0; 6000]).unwrap();
        fs::hard_link(&original, root.path().join("b.txt")).unwrap();
        let expected = allocation(&original, &fs::metadata(&original).unwrap())
            .bytes
            .unwrap();
        let s = DiskSpace::open(state.path()).unwrap();
        let start = s.start(root.path().to_str().unwrap()).unwrap();
        let v = wait(&s, start["id"].as_str().unwrap());
        assert_eq!(v["current"]["logical"], 12000);
        assert_eq!(v["current"]["allocated"], expected);
        assert_eq!(v["scan"]["summary"]["totals"]["hardlinks"], 1);
    }
    #[test]
    fn cancellation_is_partial_and_new_scan_is_single_flight() {
        let state = tempfile::tempdir().unwrap();
        let root = tempfile::tempdir().unwrap();
        for i in 0..500 {
            fs::write(root.path().join(format!("{i}.txt")), [0; 20]).unwrap();
        }
        let s = DiskSpace::open(state.path()).unwrap();
        let start = s.start(root.path().to_str().unwrap()).unwrap();
        let id = start["id"].as_str().unwrap();
        assert!(
            s.start(root.path().to_str().unwrap())
                .unwrap_err()
                .to_string()
                .starts_with("conflict:")
        );
        s.cancel(id).unwrap();
        let v = wait(&s, id);
        assert_eq!(v["scan"]["state"], "cancelled");
        assert_eq!(v["scan"]["summary"]["complete"], false);
    }
    #[test]
    fn retention_and_bad_scopes() {
        let state = tempfile::tempdir().unwrap();
        let root = tempfile::tempdir().unwrap();
        let s = DiskSpace::open(state.path()).unwrap();
        assert!(s.start("relative").is_err());
        assert!(s.start(state.path().to_str().unwrap()).is_err());
        for _ in 0..5 {
            let start = s.start(root.path().to_str().unwrap()).unwrap();
            wait(&s, start["id"].as_str().unwrap());
            while s.active.lock().is_some() {
                std::thread::yield_now();
            }
        }
        assert_eq!(s.status().unwrap()["scans"].as_array().unwrap().len(), 4);
        assert!(s.view(&json!({"id":"../../state"})).is_err());
    }
    #[cfg(unix)]
    #[test]
    fn links_are_never_followed() {
        let state = tempfile::tempdir().unwrap();
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("large"), [0; 200]).unwrap();
        std::os::unix::fs::symlink(outside.path(), root.path().join("outside")).unwrap();
        let s = DiskSpace::open(state.path()).unwrap();
        let start = s.start(root.path().to_str().unwrap()).unwrap();
        let v = wait(&s, start["id"].as_str().unwrap());
        assert_eq!(v["current"]["logical"], 0);
        assert_eq!(v["scan"]["summary"]["links_skipped"], 1);
    }
    #[test]
    fn treemap_preserves_area_and_bounds() {
        let mut tiles = vec![
            json!({"logical":80}),
            json!({"logical":15}),
            json!({"logical":5}),
        ];
        layout(&mut tiles, 0.0, 0.0, 1.0, 1.0, 3.4);
        for tile in tiles {
            let r = &tile["rect"];
            let area = r["width"].as_f64().unwrap() * r["height"].as_f64().unwrap();
            assert!((area - tile["logical"].as_f64().unwrap() / 100.0).abs() < 1e-10);
            assert!(r["x"].as_f64().unwrap() + r["width"].as_f64().unwrap() <= 1.00001);
            assert!(r["y"].as_f64().unwrap() + r["height"].as_f64().unwrap() <= 1.00001);
        }
    }
}
