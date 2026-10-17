//! Owned, single-threaded handle to the pinned native EmbeddingGemma encoder.
use anyhow::{Result, bail};
use std::{ffi::{CString, c_char, c_void}, path::Path, ptr::NonNull};

unsafe extern "C" {
    fn retriever_embedding_create(path: *const c_char, error: *mut c_char, capacity: usize) -> *mut c_void;
    fn retriever_embedding_encode(handle: *mut c_void, texts: *const *const c_char,
        lengths: *const usize, count: usize, output: *mut f32, output_length: usize,
        error: *mut c_char, capacity: usize) -> i32;
    fn retriever_embedding_free(handle: *mut c_void);
}
pub struct Embedder(NonNull<c_void>);
impl Embedder {
    pub fn load(path: &Path) -> Result<Self> {
        let path = CString::new(path.to_str().ok_or_else(||anyhow::anyhow!("Model path must be Unicode"))?)?;
        let mut error = [0u8; 512];
        // The bridge copies the path, catches C++ exceptions, and returns an owned handle.
        let handle = unsafe { retriever_embedding_create(path.as_ptr(), error.as_mut_ptr().cast(), error.len()) };
        Ok(Self(NonNull::new(handle).ok_or_else(||anyhow::anyhow!(message(&error)))?))
    }
    pub fn embed(&mut self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        if texts.is_empty() || texts.len() > 32 { bail!("Invalid embedding batch"); }
        let pointers: Vec<_> = texts.iter().map(|s|s.as_ptr().cast()).collect();
        let lengths: Vec<_> = texts.iter().map(String::len).collect();
        let mut output = vec![0f32; texts.len()*768];
        let mut error = [0u8;512];
        // Input buffers and output remain alive for this synchronous call. &mut self
        // prevents concurrent use; NonNull deliberately keeps this handle !Send/!Sync.
        let status = unsafe { retriever_embedding_encode(self.0.as_ptr(), pointers.as_ptr(), lengths.as_ptr(),
            texts.len(), output.as_mut_ptr(), output.len(), error.as_mut_ptr().cast(), error.len()) };
        if status != 0 { bail!("{}", message(&error)); }
        Ok(output.chunks_exact(768).map(|row|row.to_vec()).collect())
    }
}
impl Drop for Embedder {
    fn drop(&mut self) { unsafe { retriever_embedding_free(self.0.as_ptr()); } }
}
fn message(buffer: &[u8]) -> String {
    String::from_utf8_lossy(&buffer[..buffer.iter().position(|b|*b==0).unwrap_or(buffer.len())]).into_owned()
}
