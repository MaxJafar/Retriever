use super::*;
use std::io::Cursor;
#[tokio::test]
async fn updater_refuses_every_active_burner_phase() {
    let state=fixture_dir();let engine=crate::engine::Engine::open(state.path()).unwrap();
    for phase in ["inspecting","preflight","writing","verifying"] {
        let (job,cancel)=engine.burner.begin("inspect",phase,None,None).unwrap();
        assert!(engine.burner.is_active());
        assert!(engine.stop_for_update(&engine.update_token,env!("CARGO_PKG_VERSION")).await.unwrap_err().to_string().contains("Disk Burner"));
        engine.burner.finish(&job.id,Ok(()),&cancel);
        assert!(!engine.burner.is_active());
    }
}
fn fixture_dir() -> tempfile::TempDir {
    // macOS exposes its temporary root through /var -> /private/var. The
    // engine resolves its state root before opening services; fixtures must
    // use that same contract without relaxing the symlink guard.
    tempfile::tempdir_in(std::env::temp_dir().canonicalize().unwrap()).unwrap()
}
fn wait(service:&DiskBurner,id:&str)->Job {
    for _ in 0..1000 {
        let job=service.journal.lock().jobs.iter().find(|j|j.id==id).unwrap().clone();
        if !job.running() {return job;} std::thread::sleep(Duration::from_millis(10));
    } panic!("Task did not finish");
}
fn inspect(service:&Arc<DiskBurner>,path:&str)->Job {
    let result=service.inspect(InspectRequest{path:path.into(),sha256:String::new()}).unwrap();
    wait(service,result["job"]["id"].as_str().unwrap())
}
#[test]
fn burner_roundtrip_hashes_every_byte_and_removes_target() {
    let state=fixture_dir();let service=DiskBurner::open(&state.path().canonicalize().unwrap()).unwrap();
    let sample=service.sample().unwrap();let path=sample["path"].as_str().unwrap();
    let original=fs::read(path).unwrap();let job=inspect(&service,path);
    assert_eq!(job.state,"ready");let image=job.image.as_ref().unwrap();assert_eq!(image.partition_style,"MBR");
    assert_eq!(image.sha256,crate::digest(&original));assert_eq!(image.bootability,"Not verified");
    let started=service.start(StartRequest{inspection_id:job.id,target_id:"sandbox-8".into()}).unwrap();
    let done=wait(&service,started["job"]["id"].as_str().unwrap());assert_eq!(done.state,"complete");
    assert_eq!(done.verified_bytes,original.len() as u64);assert_eq!(done.written_sha256,Some(image.sha256.clone()));assert_eq!(done.verified_sha256,done.written_sha256);
    assert_eq!(original,fs::read(path).unwrap());
    assert!(!fs::read_dir(&service.dir).unwrap().any(|p|p.unwrap().file_name().to_string_lossy().starts_with("rehearsal-")));
    drop(service);let reopened=DiskBurner::open(state.path()).unwrap();assert_eq!(reopened.status()["current"]["state"],"complete");assert_eq!(reopened.status()["physical_writes"],false);
}
#[test]
fn burner_source_change_never_produces_success() {
    let state=fixture_dir();let service=DiskBurner::open(state.path()).unwrap();let sample=service.sample().unwrap();
    let path=sample["path"].as_str().unwrap();let ready=inspect(&service,path);let mut data=fs::read(path).unwrap();data[1000]^=1;fs::write(path,data).unwrap();
    let result=service.start(StartRequest{inspection_id:ready.id,target_id:"sandbox-8".into()}).unwrap();let failed=wait(&service,result["job"]["id"].as_str().unwrap());
    assert_eq!(failed.state,"failed");assert!(failed.error.unwrap().contains("changed"));assert!(failed.verified_sha256.is_none());
}
#[test]
fn burner_device_targets_and_source_names_are_blocked() {
    for id in ["C:","/dev/disk2","\\\\.\\PhysicalDrive0","sandbox-8/../x","", "physical-1"] {assert!(target_capacity(id).is_err());}
    for p in ["/dev/disk2","/dev/null","/proc/1/mem","/sys/a","//host/image.img","../image.img","C:/x/../image.img","C:/NUL.img","C:/x/CON","C:/image.img:stream","\\\\.\\PhysicalDrive0","\\\\?\\C:\\image.img"] {assert!(validate_path(Path::new(p)).is_err(),"{p}");}
    assert!(serde_json::from_value::<StartRequest>(json!({"inspection_id":"x","target_id":"sandbox-8","force":true})).is_err());
    assert!(serde_json::from_value::<InspectRequest>(json!({"path":"x","live":true})).is_err());
}
#[test]
fn burner_checksum_and_capacity_are_enforced() {
    assert!(checksum("xyz").is_err());assert_eq!(checksum(&"A".repeat(64)).unwrap(),"a".repeat(64));
    let state=fixture_dir();let service=DiskBurner::open(state.path()).unwrap();let sample=service.sample().unwrap();
    let result=service.inspect(InspectRequest{path:sample["path"].as_str().unwrap().into(),sha256:"0".repeat(64)}).unwrap();
    assert_eq!(wait(&service,result["job"]["id"].as_str().unwrap()).state,"failed");
    let job=inspect(&service,sample["path"].as_str().unwrap());service.update(&job.id,|j|j.image.as_mut().unwrap().bytes=9*1024*1024*1024);
    assert!(service.start(StartRequest{inspection_id:job.id,target_id:"sandbox-8".into()}).unwrap_err().to_string().contains("larger"));
}
#[test]
fn burner_stream_handles_short_reads_writes_truncation_growth_and_cancel() {
    struct ShortReader(Cursor<Vec<u8>>);impl Read for ShortReader {fn read(&mut self,b:&mut[u8])->std::io::Result<usize>{let n=b.len().min(7);self.0.read(&mut b[..n])}}
    struct ShortWriter(Vec<u8>);impl Write for ShortWriter {fn write(&mut self,b:&[u8])->std::io::Result<usize>{let n=b.len().min(3);self.0.extend_from_slice(&b[..n]);Ok(n)}fn flush(&mut self)->std::io::Result<()>{Ok(())}}
    let bytes=vec![42;512];let cancel=AtomicBool::new(false);let mut output=ShortWriter(Vec::new());
    assert_eq!(copy_exact(&mut ShortReader(Cursor::new(bytes.clone())),&mut output,512,&cancel,|_|{}).unwrap(),crate::digest(&bytes));assert_eq!(output.0,bytes);
    assert!(hash_exact(&mut Cursor::new(vec![1;511]),512,&cancel,|_|{}).is_err());
    assert!(hash_exact(&mut Cursor::new(vec![1;513]),512,&cancel,|_|{}).is_err());
    cancel.store(true,Ordering::Release);assert!(hash_exact(&mut Cursor::new(bytes),512,&cancel,|_|{}).is_err());
    struct Full;impl Write for Full {fn write(&mut self,_:&[u8])->std::io::Result<usize>{Err(std::io::ErrorKind::StorageFull.into())}fn flush(&mut self)->std::io::Result<()>{Ok(())}}
    assert!(copy_exact(&mut Cursor::new(vec![1;512]),&mut Full,512,&AtomicBool::new(false),|_|{}).is_err());
}
fn header(data:&[u8],extension:&str)->Result<ImageInfo> {
    let dir=fixture_dir();let path=dir.path().join(format!("test.{extension}"));fs::write(&path,data)?;inspect_headers(&mut File::open(&path)?,&path,data.len() as u64)
}
#[test]
fn burner_recognizes_containers_and_installer_iso_without_boot_claims() {
    let mut data=vec![0;65536];data[32769..32774].copy_from_slice(b"CD001");
    let iso=header(&data,"iso").unwrap();assert!(!iso.raw_copy);assert_eq!(iso.format,"ISO 9660 image");
    let mut hybrid=sample_bytes();hybrid[32769..32774].copy_from_slice(b"CD001");let image=header(&hybrid,"iso").unwrap();assert!(image.raw_copy);assert_eq!(image.format,"Hybrid ISO image");
    for magic in [b"PK\x03\x04".as_slice(),b"\x1f\x8b",b"\xfd7zXZ\0",b"BZh",b"vhdxfile"] {let mut bytes=vec![0;512];bytes[..magic.len()].copy_from_slice(magic);assert!(!header(&bytes,"img").unwrap().raw_copy);}
    let mut dmg=vec![0;1024];dmg[512..516].copy_from_slice(b"koly");assert_eq!(header(&dmg,"img").unwrap().format,"Apple DMG container");
    assert!(!header(&vec![0;513],"img").unwrap().raw_copy);assert!(!header(&vec![0;512],"txt").unwrap().raw_copy);
}
#[test]
fn burner_rejects_corrupt_mbr_and_gpt_bounds_and_checksums() {
    let mut image=sample_bytes();image[458..462].copy_from_slice(&u32::MAX.to_le_bytes());assert!(header(&image,"img").is_err());
    let mut image=sample_bytes();image[450]=0xee;assert!(header(&image,"img").is_err());
    image[512..520].copy_from_slice(b"EFI PART");image[524..528].copy_from_slice(&513_u32.to_le_bytes());assert!(header(&image,"img").is_err());
    image[524..528].copy_from_slice(&92_u32.to_le_bytes());assert!(header(&image,"img").unwrap_err().to_string().contains("checksum"));
    assert_eq!(crc32(b"123456789"),0xcbf43926);
}
#[test]
fn burner_job_serialization_cancel_and_restart_are_fail_closed() {
    let dir=fixture_dir();let service=DiskBurner::open(dir.path()).unwrap();
    let (job,cancel)=service.begin("rehearsal","writing",None,None).unwrap();
    assert!(service.begin("inspect","inspecting",None,None).is_err());assert!(service.cancel("wrong-id").is_err());
    service.cancel(&job.id).unwrap();assert!(cancel.load(Ordering::Acquire));service.finish(&job.id,Ok(()),&cancel);assert_eq!(service.status()["current"]["state"],"cancelled");
    service.begin("rehearsal","verifying",None,None).unwrap();drop(service);
    let restarted=DiskBurner::open(dir.path()).unwrap();assert_eq!(restarted.status()["current"]["state"],"interrupted");assert!(restarted.active.lock().is_none());
}
#[test]
fn burner_corrupt_readback_and_midstream_cancel_fail_verification() {
    let original=vec![23;2*BUFFER];let expected=crate::digest(&original);let mut corrupt=original.clone();corrupt[BUFFER+3]^=1;
    let cancel=AtomicBool::new(false);
    assert!(verify_copy(&mut Cursor::new(corrupt),original.len() as u64,&expected,&cancel,|_|{}).unwrap_err().to_string().contains("Read-back verification failed"));
    assert!(verify_copy(&mut Cursor::new(original),2*BUFFER as u64,&expected,&cancel,|_|cancel.store(true,Ordering::Release)).is_err());
}
#[test]
fn burner_valid_gpt_and_corrupt_array_are_distinguished() {
    let mut data=vec![0;1024*1024];data[510]=0x55;data[511]=0xaa;data[450]=0xee;
    data[454..458].copy_from_slice(&1_u32.to_le_bytes());data[458..462].copy_from_slice(&2047_u32.to_le_bytes());
    data[512..520].copy_from_slice(b"EFI PART");data[524..528].copy_from_slice(&92_u32.to_le_bytes());
    for (offset,value) in [(536,1_u64),(544,2047),(552,34),(560,2014),(584,2)]{data[offset..offset+8].copy_from_slice(&value.to_le_bytes());}
    data[592..596].copy_from_slice(&128_u32.to_le_bytes());data[596..600].copy_from_slice(&128_u32.to_le_bytes());
    data[1024]=1;data[1056..1064].copy_from_slice(&34_u64.to_le_bytes());data[1064..1072].copy_from_slice(&2014_u64.to_le_bytes());
    let array_crc=crc32(&data[1024..1024+128*128]);data[600..604].copy_from_slice(&array_crc.to_le_bytes());
    let header_crc=crc32(&data[512..604]);data[528..532].copy_from_slice(&header_crc.to_le_bytes());
    let image=header(&data,"img").unwrap();assert_eq!(image.partition_style,"GPT");assert_eq!(image.partitions,1);assert!(image.raw_copy);
    data[1030]^=1;assert!(header(&data,"img").unwrap_err().to_string().contains("partition array checksum"));
}
#[cfg(unix)]
#[test]
fn burner_rejects_symlinks_and_nonregular_files() {
    let dir=fixture_dir();let path=dir.path().join("file.img");fs::write(&path,vec![0;512]).unwrap();
    let link=dir.path().join("link.img");std::os::unix::fs::symlink(&path,&link).unwrap();assert!(open_source(&link).is_err());assert!(open_source(dir.path()).is_err());
}
