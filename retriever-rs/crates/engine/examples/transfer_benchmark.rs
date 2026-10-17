//! Read-only codec measurements on explicitly supplied actual files.
use anyhow::Result;
use retriever_engine::transfer_codec::{self,BLOCK};
use std::{fs::File,io::Read,time::Instant};
fn main()->Result<()> {
    let mut report=Vec::new();
    for name in std::env::args().skip(1) {
        let mut file=File::open(&name)?;let size=file.metadata()?.len();
        let mut buffer=vec![0u8;BLOCK];let mut raw=0usize;let mut wire=0usize;let mut compressed=0usize;let mut encode=0f64;let mut decode=0f64;
        loop {
            let n=file.read(&mut buffer)?;if n==0{break;}
            let start=Instant::now();let block=transfer_codec::encode(&buffer[..n])?;encode+=start.elapsed().as_secs_f64();
            let start=Instant::now();let decoded=transfer_codec::decode(&block)?;decode+=start.elapsed().as_secs_f64();
            anyhow::ensure!(decoded==buffer[..n],"Lossless verification failed");
            raw+=n;wire+=block["data"].as_str().unwrap().len();compressed+=usize::from(block["encoding"]=="lz4-block");
        }
        anyhow::ensure!(raw as u64==size,"Source length changed");
        report.push(serde_json::json!({"path":name,"bytes":size,"encoded_base64_bytes":wire,"legacy_base64_bytes":raw.div_ceil(3)*4,"compressed_blocks":compressed,"encode_mib_s":raw as f64/1048576./encode,"decode_mib_s":raw as f64/1048576./decode,"roundtrip_verified":true}));
    }
    println!("{}",serde_json::to_string_pretty(&report)?);Ok(())
}
