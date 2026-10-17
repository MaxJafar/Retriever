pub mod agent;
pub mod nodes;
pub mod burner;
pub mod config;
pub mod crawl;
pub mod disk;
pub mod engine;
pub mod explorer;
pub mod assistant;
pub mod remote;
pub mod extract;
pub mod http;
mod inventory;
pub mod knowledge;
pub mod media;
pub mod migration;
mod office;
pub mod operations;
pub mod onboarding;
pub mod paths;
pub mod properties;
pub mod native_models;
mod native_index;
pub mod providers;
pub mod scheduler;
pub mod store;
pub mod transfer;
pub mod transfer_codec;
pub mod native_copy;
pub mod vectors;
mod vector_storage;
pub mod workspace;
pub mod updates;

pub fn now() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}
pub fn digest(bytes: impl AsRef<[u8]>) -> String {
    use sha2::{Digest, Sha256};
    hex::encode(Sha256::digest(bytes.as_ref()))
}

pub mod health;
pub mod health_platform;
