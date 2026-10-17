//! Disk Burner: image inspection and verified, disposable file-target rehearsal.
//! There is deliberately no physical-device writer, device enumeration or bypass.
use anyhow::{Context, Result, bail, ensure};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    path::{Component, Path, PathBuf},
    sync::{Arc, atomic::{AtomicBool, Ordering}},
    time::{Duration, Instant},
};

const MAX_IMAGE: u64 = 64 * 1024 * 1024 * 1024;
const BUFFER: usize = 1024 * 1024;
const HISTORY: usize = 24;
pub const PHYSICAL_BLOCK: &str = "Physical disk writing is unavailable in this candidate. Use a disposable file target. Separate-hardware validation and review are required before a device writer can be enabled.";

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct ImageInfo {
    pub path: String,
    pub name: String,
    pub bytes: u64,
    pub sha256: String,
    pub format: String,
    pub partition_style: String,
    pub partitions: u32,
    pub raw_copy: bool,
    pub bootability: String,
    pub notes: Vec<String>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Job {
    pub id: String,
    pub kind: String,
    pub state: String,
    pub created_at: f64,
    pub finished_at: Option<f64>,
    pub bytes_done: u64,
    pub bytes_total: u64,
    pub elapsed_seconds: f64,
    pub bytes_per_second: f64,
    pub image: Option<ImageInfo>,
    pub target_id: Option<String>,
    pub written_sha256: Option<String>,
    pub verified_sha256: Option<String>,
    pub verified_bytes: u64,
    pub error: Option<String>,
    pub fixture: bool,
}
impl Job {
    fn running(&self) -> bool { matches!(self.state.as_str(), "inspecting"|"preflight"|"writing"|"verifying") }
}

#[derive(Serialize, Deserialize, Default)]
struct Journal { jobs: Vec<Job> }
struct Active { id: String, cancel: Arc<AtomicBool> }
pub struct DiskBurner {
    dir: PathBuf,
    journal: Mutex<Journal>,
    active: Mutex<Option<Active>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InspectRequest { pub path: String, #[serde(default)] pub sha256: String }
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StartRequest { pub inspection_id: String, pub target_id: String }

fn target_capacity(id: &str) -> Result<u64> {
    match id {
        "sandbox-8" => Ok(8 * 1024 * 1024 * 1024),
        "sandbox-16" => Ok(16 * 1024 * 1024 * 1024),
        "sandbox-32" => Ok(32 * 1024 * 1024 * 1024),
        _ => bail!(PHYSICAL_BLOCK),
    }
}
fn checksum(value: &str) -> Result<String> {
    let value = value.trim().to_ascii_lowercase();
    ensure!(value.is_empty() || (value.len()==64 && value.bytes().all(|b|b.is_ascii_hexdigit())), "Use a 64-character SHA-256 checksum, or leave it empty");
    Ok(value)
}
fn cancelled(cancel: &AtomicBool) -> Result<()> {
    ensure!(!cancel.load(Ordering::Acquire), "Cancelled"); Ok(())
}

impl DiskBurner {
    pub fn open(state: &Path) -> Result<Arc<Self>> {
        let dir = state.join("disk-burner");
        crate::paths::reject_links(&dir)?;
        fs::create_dir_all(&dir)?;
        let path = dir.join("history.json");
        crate::paths::reject_links(&path)?;
        let mut journal: Journal = if path.exists() {
            ensure!(fs::metadata(&path)?.len()<=2*1024*1024, "Disk Burner journal exceeds its limit");
            serde_json::from_slice(&fs::read(&path)?).context("Disk Burner history could not be read")?
        } else { Journal::default() };
        journal.jobs.truncate(HISTORY);
        for job in &mut journal.jobs {
            if job.running() {
                job.state="interrupted".into();job.finished_at=Some(crate::now());
                job.error=Some("Retriever restarted before verification finished. Nothing is resumed automatically.".into());
            }
        }
        let service=Arc::new(Self {dir, journal:Mutex::new(journal), active:Mutex::new(None)});
        service.save()?;
        Ok(service)
    }
    fn save(&self) -> Result<()> {
        let journal=self.journal.lock();
        crate::config::atomic_json(&self.dir.join("history.json"), &*journal)
    }
    pub fn is_active(&self) -> bool { self.active.lock().is_some() }
    pub fn status(&self) -> Value {
        let journal=self.journal.lock();
        json!({"physical_writes":false,"device_enumeration":false,"mode":"file-sandbox","notice":PHYSICAL_BLOCK,
            "history":journal.jobs,"current":journal.jobs.first(),
            "inspection":journal.jobs.iter().find(|j|j.kind=="inspect").filter(|j|j.state=="ready"),
            "targets":[
                {"id":"sandbox-8","name":"8 GiB file target","capacity":8_u64*1024*1024*1024,"fixture":true},
                {"id":"sandbox-16","name":"16 GiB file target","capacity":16_u64*1024*1024*1024,"fixture":true},
                {"id":"sandbox-32","name":"32 GiB file target","capacity":32_u64*1024*1024*1024,"fixture":true}],
            "target_policy":"A new temporary file is created for every rehearsal and removed afterwards. Existing files and physical disks are never targets.",
            "max_image_bytes":MAX_IMAGE})
    }
    fn begin(&self, kind:&str, state:&str, image:Option<ImageInfo>, target:Option<String>) -> Result<(Job,Arc<AtomicBool>)> {
        let mut active=self.active.lock();
        ensure!(active.is_none(), "A Disk Burner task is already running. Cancel it or wait for completion.");
        let cancel=Arc::new(AtomicBool::new(false));
        let job=Job {id:uuid::Uuid::new_v4().to_string(),kind:kind.into(),state:state.into(),created_at:crate::now(),finished_at:None,
            bytes_done:0,bytes_total:image.as_ref().map_or(0,|i|i.bytes),elapsed_seconds:0.,bytes_per_second:0.,image,
            target_id:target,written_sha256:None,verified_sha256:None,verified_bytes:0,error:None,fixture:kind=="rehearsal"};
        {let mut journal=self.journal.lock();journal.jobs.insert(0,job.clone());journal.jobs.truncate(HISTORY);}
        self.save()?;
        *active=Some(Active{id:job.id.clone(),cancel:cancel.clone()});
        Ok((job,cancel))
    }
    fn update(&self,id:&str,edit:impl FnOnce(&mut Job)) {
        if let Some(job)=self.journal.lock().jobs.iter_mut().find(|j|j.id==id) {edit(job);}
    }
    fn phase(&self,id:&str,state:&str,total:u64) -> Result<()> {
        self.update(id,|j|{j.state=state.into();j.bytes_done=0;j.bytes_total=total;j.bytes_per_second=0.;});self.save()
    }
    fn progress(&self,id:&str,bytes:u64,started:Instant) {
        self.update(id,|j|{j.bytes_done=bytes;j.elapsed_seconds=(crate::now()-j.created_at).max(0.);j.bytes_per_second=bytes as f64/started.elapsed().as_secs_f64().max(0.001);});
    }
    fn finish(&self,id:&str,result:Result<()>,cancel:&AtomicBool) {
        // Completion/cancellation and a subsequent job are serialized by active.
        let mut active=self.active.lock();
        self.update(id,|j|{
            j.elapsed_seconds=(crate::now()-j.created_at).max(0.);j.finished_at=Some(crate::now());
            j.state=match result {
                Ok(()) if !cancel.load(Ordering::Acquire) => if j.kind=="inspect" {"ready"} else {"complete"}.into(),
                Ok(()) => {j.error=Some("Cancelled".into());"cancelled".into()},
                Err(e) => {j.error=Some(e.to_string());if cancel.load(Ordering::Acquire) {"cancelled"} else {"failed"}.into()}
            };
        });
        if let Err(e)=self.save() {self.update(id,|j|{j.state="failed".into();j.error=Some(format!("Could not save verification receipt: {e}"));});}
        *active=None;
    }
    pub fn cancel(&self,id:&str) -> Result<Value> {
        let active=self.active.lock();
        let a=active.as_ref().context("No Disk Burner task is running")?;
        ensure!(a.id==id,"The active Disk Burner task has changed");
        a.cancel.store(true,Ordering::Release);Ok(json!({"cancel_requested":true,"id":id}))
    }
    pub fn inspect(self:&Arc<Self>, request:InspectRequest) -> Result<Value> {
        let expected=checksum(&request.sha256)?;
        let path=PathBuf::from(request.path);
        validate_path(&path)?;
        let (job,cancel)=self.begin("inspect","inspecting",None,None)?;
        let response=json!({"job":job});let service=self.clone();
        std::thread::spawn(move||{
            let result=(||{
                let mut source=open_source(&path)?;
                let bytes=source.metadata()?.len();
                ensure!((512..=MAX_IMAGE).contains(&bytes),"Image must be between 512 bytes and 64 GiB");
                service.phase(&job.id,"inspecting",bytes)?;
                let mut info=inspect_headers(&mut source,&path,bytes)?;
                source.seek(SeekFrom::Start(0))?;
                let started=Instant::now();
                info.sha256=hash_exact(&mut source,bytes,&cancel,|n|service.progress(&job.id,n,started))?;
                ensure!(expected.is_empty() || expected==info.sha256,"SHA-256 does not match the supplied checksum");
                service.update(&job.id,|j|j.image=Some(info));Ok(())
            })();service.finish(&job.id,result,&cancel);
        });Ok(response)
    }
    pub fn start(self:&Arc<Self>, request:StartRequest) -> Result<Value> {
        // Target validation is first and cannot reach a device or accept a path.
        let capacity=target_capacity(&request.target_id)?;
        let image=self.journal.lock().jobs.iter().find(|j|j.id==request.inspection_id && j.kind=="inspect" && j.state=="ready")
            .and_then(|j|j.image.clone()).context("Inspect the image before starting a rehearsal")?;
        ensure!(image.raw_copy,"This format needs conversion or installer preparation. Raw copying is unavailable.");
        ensure!(image.bytes<=capacity,"The image is larger than this target. Choose a larger file target.");
        let (job,cancel)=self.begin("rehearsal","preflight",Some(image.clone()),Some(request.target_id))?;
        let response=json!({"job":job});let service=self.clone();
        std::thread::spawn(move||{
            let result=service.rehearse(&job.id,&image,&cancel);
            service.finish(&job.id,result,&cancel);
        });Ok(response)
    }
    fn rehearse(&self,id:&str,image:&ImageInfo,cancel:&AtomicBool) -> Result<()> {
        let mut source=open_source(Path::new(&image.path))?;
        ensure!(source.metadata()?.len()==image.bytes,"Source changed since inspection. Inspect it again.");
        let started=Instant::now();
        let digest=hash_exact(&mut source,image.bytes,cancel,|n|self.progress(id,n,started))?;
        ensure!(digest==image.sha256,"Source changed since inspection. Inspect it again.");
        source.seek(SeekFrom::Start(0))?;
        cancelled(cancel)?;
        // A fresh regular temp file is the only writable target implementation.
        crate::paths::reject_links(&self.dir)?;
        let mut target=tempfile::Builder::new().prefix("rehearsal-").suffix(".tmp").tempfile_in(&self.dir)?;
        self.phase(id,"writing",image.bytes)?;
        let started=Instant::now();
        let written=copy_exact(&mut source,target.as_file_mut(),image.bytes,cancel,|n|self.progress(id,n,started))?;
        ensure!(written==image.sha256,"Source changed during writing. The rehearsal is invalid.");
        target.as_file().sync_all()?;
        self.update(id,|j|j.written_sha256=Some(written));
        cancelled(cancel)?;
        self.phase(id,"verifying",image.bytes)?;
        // Reopen a read handle after flushing, then read back the entire output.
        let mut reader=target.reopen()?;reader.seek(SeekFrom::Start(0))?;
        let started=Instant::now();
        let verified=verify_copy(&mut reader,image.bytes,&image.sha256,cancel,|n|self.progress(id,n,started))?;
        drop(reader);
        target.close().context("Could not remove the temporary rehearsal image")?;
        self.update(id,|j|{j.verified_sha256=Some(verified);j.verified_bytes=image.bytes;});Ok(())
    }
    pub fn sample(&self) -> Result<Value> {
        ensure!(self.active.lock().is_none(),"Wait for the current Disk Burner task before loading a sample");
        let path=self.dir.join("retriever-sample.img");
        crate::paths::reject_links(&path)?;
        if !path.exists() {
            let mut file=OpenOptions::new().write(true).create_new(true).open(&path)?;
            let bytes=sample_bytes();file.write_all(&bytes)?;file.sync_all()?;
        }
        // Engine state is canonicalized on Windows, which adds a verbatim prefix.
        // Export this internally generated regular-file path in picker form;
        // arbitrary incoming namespace paths remain forbidden by validate_path.
        let display=path.to_string_lossy().into_owned();
        #[cfg(windows)]
        let display=display.strip_prefix(r"\\?\").unwrap_or(&display).to_owned();
        Ok(json!({"path":display,"fixture":true,"description":"Synthetic 4 MiB MBR image. It contains no bootloader or operating system."}))
    }
}

// Reject device namespaces before metadata/open, on either host platform.
fn validate_path(path:&Path) -> Result<()> {
    let raw=path.to_string_lossy();let normalized=raw.replace('\\',"/").to_ascii_lowercase();
    ensure!(path.is_absolute() && !path.components().any(|p|matches!(p,Component::ParentDir)),"Choose an absolute local image-file path without traversal");
    ensure!(!normalized.starts_with("//") && !normalized.contains('\0'),"Device namespaces and network paths are not image files");
    for prefix in ["/dev","/proc","/sys"] {ensure!(normalized!=prefix && !normalized.starts_with(&format!("{prefix}/")),"Device namespaces are not image files");}
    for (i,part) in normalized.split('/').enumerate() {
        ensure!(!part.contains(':') || (i==0 && part.len()==2 && part.as_bytes()[0].is_ascii_alphabetic()),"Alternate streams and device paths are not image files");
        let base=part.trim_end_matches([' ','.']).split('.').next().unwrap_or("");
        ensure!(!["con","prn","aux","nul","conin$","conout$"].contains(&base) && !(base.len()==4 && (base.starts_with("com")||base.starts_with("lpt")) && base.as_bytes()[3].is_ascii_digit()),"Reserved device names are not image files");
    }
    Ok(())
}
fn open_source(path:&Path) -> Result<File> {
    validate_path(path)?;
    crate::paths::reject_links(path)?;
    ensure!(fs::symlink_metadata(path)?.file_type().is_file(),"Select a regular image file");
    let mut options=OpenOptions::new();options.read(true);
    #[cfg(unix)] {use std::os::unix::fs::OpenOptionsExt;options.custom_flags(libc::O_NOFOLLOW|libc::O_NONBLOCK);}
    let file=options.open(path)?;
    ensure!(file.metadata()?.is_file(),"Select a regular image file");
    Ok(file)
}

fn copy_exact<R:Read,W:Write>(source:&mut R,target:&mut W,bytes:u64,cancel:&AtomicBool,mut progress:impl FnMut(u64)) -> Result<String> {
    let mut remaining=bytes;let mut hash=Sha256::new();let mut buffer=vec![0;BUFFER];let mut tick=Instant::now();
    while remaining>0 {
        cancelled(cancel)?;
        let requested=remaining.min(BUFFER as u64) as usize;
        let n=source.read(&mut buffer[..requested])?;
        ensure!(n>0,"Image was truncated during reading");
        target.write_all(&buffer[..n])?;hash.update(&buffer[..n]);remaining-=n as u64;
        if tick.elapsed()>=Duration::from_millis(80) || remaining==0 {progress(bytes-remaining);tick=Instant::now();}
    }
    cancelled(cancel)?;let mut extra=[0];ensure!(source.read(&mut extra)?==0,"Image grew during reading");
    target.flush()?;Ok(hex::encode(hash.finalize()))
}
fn hash_exact<R:Read>(source:&mut R,bytes:u64,cancel:&AtomicBool,progress:impl FnMut(u64)) -> Result<String> {
    copy_exact(source,&mut std::io::sink(),bytes,cancel,progress)
}
fn verify_copy<R:Read>(source:&mut R,bytes:u64,expected:&str,cancel:&AtomicBool,progress:impl FnMut(u64))->Result<String> {
    let digest=hash_exact(source,bytes,cancel,progress)?;
    ensure!(digest==expected,"Read-back verification failed. No successful receipt was issued.");Ok(digest)
}
fn u32le(b:&[u8],i:usize)->u32 {u32::from_le_bytes(b[i..i+4].try_into().unwrap())}
fn u64le(b:&[u8],i:usize)->u64 {u64::from_le_bytes(b[i..i+8].try_into().unwrap())}
fn crc32(data:&[u8])->u32 {
    let mut crc=!0_u32;for b in data {crc^=*b as u32;for _ in 0..8 {crc=(crc>>1)^if crc&1!=0 {0xedb88320} else {0};}} !crc
}
fn inspect_headers(file:&mut File,path:&Path,bytes:u64) -> Result<ImageInfo> {
    let mut head=vec![0;bytes.min(65536) as usize];file.read_exact(&mut head)?;
    let mut tail=[0;512];file.seek(SeekFrom::End(-512))?;file.read_exact(&mut tail)?;
    let ext=path.extension().unwrap_or_default().to_string_lossy().to_ascii_lowercase();
    let mut info=ImageInfo{path:path.to_string_lossy().into(),name:path.file_name().unwrap_or_default().to_string_lossy().into(),bytes,sha256:String::new(),
        format:"Raw image".into(),partition_style:"Not detected".into(),partitions:0,raw_copy:false,bootability:"Not verified".into(),notes:Vec::new()};
    let container=if head.starts_with(&[0x1f,0x8b]) {Some("Gzip archive")} else if head.starts_with(b"PK\x03\x04") {Some("ZIP archive")}
        else if head.starts_with(b"\xfd7zXZ\0") {Some("XZ archive")} else if head.starts_with(b"BZh") {Some("Bzip2 archive")}
        else if tail.starts_with(b"koly") {Some("Apple DMG container")} else if tail.starts_with(b"conectix") {Some("VHD container")}
        else if head.starts_with(b"vhdxfile") {Some("VHDX container")} else if ["zip","gz","xz","bz2","dmg","vhd","vhdx","exe"].contains(&ext.as_str()) {Some("Unsupported container")} else {None};
    if let Some(format)=container {info.format=format.into();info.notes.push("Extract or convert to an uncompressed disk image first. This container is not a raw disk image.".into());return Ok(info);}
    let iso=head.len()>32774 && &head[32769..32774]==b"CD001";
    let mbr=head[510..512]==[0x55,0xaa];
    let mut protective=false;
    if mbr {
        for offset in [446,462,478,494] {
            let kind=head[offset+4];let first=u32le(&head,offset+8) as u64;let count=u32le(&head,offset+12) as u64;
            if kind==0 {continue;}
            ensure!([0,0x80].contains(&head[offset]),"Invalid MBR partition boot flag");
            ensure!(count>0 && first.checked_add(count).and_then(|n|n.checked_mul(512)).is_some_and(|end|end<=bytes),"Partition extends beyond the image or is empty");
            protective|=kind==0xee;info.partitions+=1;
        }
        if info.partitions>0 {info.partition_style="MBR".into();}
    }
    if head.len()>=1024 && &head[512..520]==b"EFI PART" {
        let size=u32le(&head,524) as usize;
        ensure!((92..=512).contains(&size),"Invalid GPT header length");
        let mut header=head[512..512+size].to_vec();let stored=u32le(&header,16);header[16..20].fill(0);
        ensure!(crc32(&header)==stored,"GPT header checksum mismatch");
        ensure!(u64le(&head,536)==1,"GPT primary header is not at LBA 1");
        let first=u64le(&head,552);let last=u64le(&head,560);let entry_lba=u64le(&head,584);
        let count=u32le(&head,592) as u64;let entry_size=u32le(&head,596) as u64;
        ensure!(first<=last && last<bytes/512 && (128..=4096).contains(&entry_size) && entry_size%128==0,"Invalid GPT geometry");
        let length=count.checked_mul(entry_size).context("GPT array overflow")?;
        let start=entry_lba.checked_mul(512).context("GPT offset overflow")?;
        ensure!(count>0 && length<=4*1024*1024 && entry_lba>=2 && start.checked_add(length).is_some_and(|end|end<=bytes && end<=first.saturating_mul(512)),"GPT entries exceed image bounds");
        let mut entries=vec![0;length as usize];file.seek(SeekFrom::Start(start))?;file.read_exact(&mut entries)?;
        ensure!(crc32(&entries)==u32le(&head,600),"GPT partition array checksum mismatch");
        info.partitions=0;
        for entry in entries.chunks_exact(entry_size as usize) {if entry[..16].iter().all(|b|*b==0){continue;}
            let a=u64le(entry,32);let b=u64le(entry,40);ensure!(a>=first && a<=b && b<=last,"GPT partition is outside usable image bounds");info.partitions+=1;
        }
        info.partition_style="GPT".into();info.notes.push("Primary GPT header and partition array checksums verified. Backup GPT and filesystem contents are not validated.".into());
    } else {ensure!(!protective,"Protective MBR is missing a valid GPT header");}
    if iso {
        info.format=if info.partitions>0 {"Hybrid ISO image"} else {"ISO 9660 image"}.into();
        info.raw_copy=info.partitions>0;
        info.notes.push(if info.raw_copy {"Partitioned ISO detected; raw-copy rehearsal is available. This does not prove USB boot compatibility."} else {"This ISO needs installer preparation. Byte copying alone does not make a Windows installer USB."}.into());
    } else {
        info.raw_copy=["img","raw","bin"].contains(&ext.as_str()) && bytes%512==0;
        if !info.raw_copy {info.notes.push("Use a sector-aligned .img, .raw or .bin disk image. Arbitrary files are not flashable images.".into());}
    }
    if info.partitions==0 {info.notes.push("No supported partition table detected. Bootability has not been established.".into());}
    info.notes.push("No bootloader, persistence partition or Windows installer customization is added.".into());Ok(info)
}

fn sample_bytes()->Vec<u8> {
    let mut data=vec![0;4*1024*1024];
    for (i,b) in data[1024*1024..].iter_mut().enumerate() {*b=(i%251) as u8;}
    data[446+4]=0x0c;data[454..458].copy_from_slice(&2048_u32.to_le_bytes());
    data[458..462].copy_from_slice(&6144_u32.to_le_bytes());data[510]=0x55;data[511]=0xaa;data
}

#[cfg(test)]
mod tests;
