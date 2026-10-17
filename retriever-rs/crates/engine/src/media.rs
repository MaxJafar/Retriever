//! Source decoding is separate from inference and never invokes a shell.
use crate::extract::{Extraction, passages};
use anyhow::{Context, Result, bail};
use base64::Engine as _;
use pdfium_render::prelude::*;
use retriever_protocol::Coordinates;
use serde_json::{Value, json};
use std::{
    io::Cursor,
    path::{Path, PathBuf},
    sync::LazyLock,
    time::Duration,
};
static PDF_LOCK: LazyLock<parking_lot::Mutex<()>> = LazyLock::new(|| parking_lot::Mutex::new(()));
fn pdfium() -> Result<Pdfium> {
    let path = std::env::var_os("RETRIEVER_PDFIUM")
        .map(PathBuf::from)
        .unwrap_or(
            std::env::current_exe()?
                .parent()
                .context("Application directory unavailable")?
                .join(Pdfium::pdfium_platform_library_name()),
        );
    match Pdfium::bind_to_library(path) {
        Ok(bindings) => Ok(Pdfium::new(bindings)),
        // 0.9 keeps bindings process-wide. Rebinding after the first PDF fails;
        // Default explicitly reuses those initialized bindings.
        Err(PdfiumError::PdfiumLibraryBindingsAlreadyInitialized) => Ok(Pdfium::default()),
        Err(error) => Err(error).context("pdfium_not_packaged"),
    }
}
pub fn pdf_text(bytes: &[u8], max: usize, chunk: usize) -> Result<Extraction> {
    let _lock = PDF_LOCK.lock();
    let pdfium = match pdfium() {
        Ok(v) => v,
        Err(_) => {
            return Ok(Extraction {
                text: String::new(),
                detail: "metadata: pdfium_not_packaged".into(),
                passages: vec![],
            });
        }
    };
    let doc = match pdfium.load_pdf_from_byte_slice(bytes, None) {
        Ok(v) => v,
        Err(e) => {
            return Ok(Extraction {
                text: String::new(),
                detail: format!("metadata: pdf_open_failed ({e})"),
                passages: vec![],
            });
        }
    };
    let mut text = String::new();
    let mut all = Vec::new();
    let mut length = 0;
    for (i, page) in doc.pages().iter().enumerate() {
        let part = page.text()?.all();
        let offset = length;
        length += part.chars().count() + 2;
        if length > max {
            bail!("extracted_text_budget");
        }
        for mut passage in passages(&part, chunk, Some(i as u32 + 1)) {
            passage.coordinates.char_start += offset;
            passage.coordinates.char_end += offset;
            all.push(passage);
        }
        text.push_str(&part);
        text.push_str("\n\n");
    }
    // Compressed PDF streams must be screened after decoding as well as before.
    if crate::paths::secret_content(text.as_bytes()) {
        bail!("secret_content");
    }
    if text.trim().is_empty() {
        // PDFium yields page separators even when every page is an image.
        // Empty separators must never route a scanned PDF to text inference.
        text.clear();
        all.clear();
    }
    Ok(Extraction {
        detail: if text.trim().is_empty() {
            "metadata: scanned_pdf_requires_opt_in"
        } else {
            "text"
        }
        .into(),
        text,
        passages: all,
    })
}
fn image_input(
    image: image::DynamicImage,
    coordinates: Coordinates,
) -> Result<(Value, Coordinates)> {
    let image = if image.width() > 1024 || image.height() > 1024 {
        image.thumbnail(1024, 1024)
    } else {
        image
    }
    .to_rgb8();
    let mut output = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut output, 82).encode_image(&image)?;
    let uri = format!(
        "data:image/jpeg;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(output)
    );
    Ok((
        json!({"content":[{"type":"image_url","image_url":{"url":uri}}]}),
        coordinates,
    ))
}
pub fn image_pixels(bytes: &[u8]) -> Result<Value> {
    let mut reader = image::ImageReader::new(Cursor::new(bytes)).with_guessed_format()?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(16384);
    limits.max_image_height = Some(16384);
    limits.max_alloc = Some(128 * 1024 * 1024);
    reader.limits(limits);
    use image::ImageDecoder;
    let mut decoder = reader.into_decoder()?;
    let orientation = decoder.orientation()?;
    let mut pixels = image::DynamicImage::from_decoder(decoder)?;
    pixels.apply_orientation(orientation);
    Ok(image_input(pixels, Coordinates::default())?.0)
}
pub fn pdf_page(bytes: &[u8], page: u32) -> Result<(Value, Coordinates)> {
    let _lock = PDF_LOCK.lock();
    let engine = pdfium()?;
    let doc = engine.load_pdf_from_byte_slice(bytes, None)?;
    let page_object = doc.pages().get(page as i32)?;
    let render = page_object.render_with_config(
        &PdfRenderConfig::new()
            .set_target_width(1024)
            .set_maximum_height(1600),
    )?;
    image_input(
        render.as_image()?,
        Coordinates {
            page: Some(page + 1),
            ..Default::default()
        },
    )
}
fn ffmpeg_tool(name: &str) -> Result<PathBuf> {
    let folder = std::env::var_os("RETRIEVER_MEDIA_BIN")
        .map(PathBuf::from)
        .unwrap_or(
            std::env::current_exe()?
                .parent()
                .context("Missing application directory")?
                .to_owned(),
        );
    let path = folder.join(if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    });
    if !path.is_file() {
        bail!("Packaged {name} is unavailable");
    }
    Ok(path)
}
async fn command(name: &str, args: &[String], max: usize) -> Result<Vec<u8>> {
    use tokio::io::AsyncReadExt;
    let mut process = tokio::process::Command::new(ffmpeg_tool(name)?);
    #[cfg(windows)]
    process.creation_flags(0x08000000); // CREATE_NO_WINDOW for native launchers.
    let mut child = process
        .args(args)
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()?;
    tokio::time::timeout(Duration::from_secs(25), async {
        let mut bytes = Vec::new();
        child
            .stdout
            .take()
            .unwrap()
            .take((max + 1) as u64)
            .read_to_end(&mut bytes)
            .await?;
        if bytes.len() > max {
            bail!("Decoded media exceeds budget");
        }
        if !child.wait().await?.success() {
            bail!("Media decoding failed");
        }
        Ok(bytes)
    })
    .await
    .context("Media decoding deadline exceeded")?
}
pub async fn duration(path: &Path) -> Result<f64> {
    let bytes = command(
        "ffprobe",
        &[
            "-v",
            "error",
            "-protocol_whitelist",
            "file,pipe",
            "-format_whitelist",
            "mov,matroska,webm,avi,wav,mp3,flac,ogg,aac",
            "-show_entries",
            "format=duration",
            "-of",
            "json",
        ]
        .map(str::to_string)
        .into_iter()
        .chain([path.to_string_lossy().into_owned()])
        .collect::<Vec<_>>(),
        65536,
    )
    .await?;
    let value: Value = serde_json::from_slice(&bytes)?;
    let seconds = value["format"]["duration"]
        .as_str()
        .context("Missing duration")?
        .parse::<f64>()?;
    if !seconds.is_finite() || seconds <= 0.0 {
        bail!("Invalid media duration");
    }
    Ok(seconds)
}
pub fn sample_times(duration: f64, step: f64, maximum: usize) -> Vec<f64> {
    if !duration.is_finite() || duration <= 0.0 || step <= 0.0 {
        return vec![];
    }
    let count = (duration / step).ceil().max(1.0) as usize;
    (0..count.min(maximum))
        .map(|i| {
            if count == 1 {
                duration / 2.0
            } else {
                (i as f64 * step).min(duration - 0.001)
            }
        })
        .collect()
}
pub async fn video_frame(path: &Path, seconds: f64) -> Result<(Value, Coordinates)> {
    if !seconds.is_finite() || seconds < 0.0 {
        bail!("Invalid frame time");
    }
    let args = vec![
        "-nostdin".into(),
        "-v".into(),
        "error".into(),
        "-protocol_whitelist".into(),
        "file,pipe".into(),
        "-format_whitelist".into(),
        "mov,matroska,webm,avi".into(),
        "-threads".into(),
        "2".into(),
        "-ss".into(),
        seconds.to_string(),
        "-i".into(),
        path.to_string_lossy().into(),
        "-frames:v".into(),
        "1".into(),
        "-an".into(),
        "-sn".into(),
        "-vf".into(),
        "scale=1024:1024:force_original_aspect_ratio=decrease:out_range=full,format=yuvj420p"
            .into(),
        "-f".into(),
        "image2pipe".into(),
        "-vcodec".into(),
        "mjpeg".into(),
        "pipe:1".into(),
    ];
    let bytes = command("ffmpeg", &args, 8 * 1024 * 1024).await?;
    Ok((
        image_pixels(&bytes)?,
        Coordinates {
            start_seconds: Some(seconds),
            end_seconds: Some(seconds),
            ..Default::default()
        },
    ))
}
pub async fn audio_window(path: &Path, start: f64, end: f64) -> Result<(Value, Coordinates)> {
    if !start.is_finite() || !end.is_finite() || start < 0.0 || end <= start || end - start > 30.0 {
        bail!("Invalid audio window");
    }
    let args = vec![
        "-nostdin".into(),
        "-v".into(),
        "error".into(),
        "-protocol_whitelist".into(),
        "file,pipe".into(),
        "-ss".into(),
        start.to_string(),
        "-i".into(),
        path.to_string_lossy().into(),
        "-t".into(),
        (end - start).to_string(),
        "-vn".into(),
        "-ac".into(),
        "1".into(),
        "-ar".into(),
        "16000".into(),
        "-f".into(),
        "wav".into(),
        "pipe:1".into(),
    ];
    let bytes = command("ffmpeg", &args, 2 * 1024 * 1024).await?;
    Ok((
        json!({"content":[{"type":"input_audio","input_audio":{"data":base64::engine::general_purpose::STANDARD.encode(bytes),"format":"wav"}}]}),
        Coordinates {
            start_seconds: Some(start),
            end_seconds: Some(end),
            ..Default::default()
        },
    ))
}
/// Cover the whole clip with three evenly spaced, interior thumbnails. A short
/// clip gets one frame, avoiding zero timestamps and duplicate opening frames.
pub fn thumbnail_times(seconds: f64) -> Vec<f64> {
    if !seconds.is_finite() || seconds <= 0.0 {
        return vec![];
    }
    if seconds < 3.0 {
        return vec![seconds / 2.0];
    }
    [0.1, 0.5, 0.9].map(|fraction| seconds * fraction).to_vec()
}

/// Decode a private byte snapshot, never the source path again. The file may be
/// replaced while FFmpeg runs; its digest and the resulting evidence stay bound.
pub fn video_thumbnails(bytes: Vec<u8>, extension: &str) -> Result<Value> {
    if !["mp4", "mov", "mkv", "webm", "avi", "m4v"].contains(&extension) {
        bail!("Unsupported video");
    }
    use std::io::Write;
    let mut source = tempfile::Builder::new()
        .prefix("retriever-video-")
        .suffix(&format!(".{extension}"))
        .tempfile()?;
    source.write_all(&bytes)?;
    source.flush()?;
    drop(bytes);
    video_thumbnails_snapshot(source)
}
pub fn video_thumbnails_snapshot(source: tempfile::NamedTempFile) -> Result<Value> {
    std::thread::spawn(move || -> Result<Value> {
        let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build()?;
        runtime.block_on(async {
            let seconds = duration(source.path()).await?;
            let times = thumbnail_times(seconds);
            if times.is_empty() { bail!("Video duration is unavailable"); }
            let mut decoded = Vec::new();
            let mut unavailable = Vec::new();
            for time in times {
                match video_frame(source.path(), time).await {
                    Ok(frame) => decoded.push(frame),
                    Err(_) => unavailable.push(time),
                }
            }
            // Edited recordings can advertise long empty tails. Preserve real
            // available evidence rather than losing the entire video because
            // a later timestamp contains no frame. Never invent a timestamp.
            if decoded.is_empty() { decoded.push(video_frame(source.path(), 0.0).await?); }
            let mut canvas = image::RgbImage::from_pixel(1024, 384, image::Rgb([24, 24, 24]));
            let mut evidence = Vec::new();
            let width = 1024 / decoded.len() as u32;
            for (i, (frame, coordinates)) in decoded.into_iter().enumerate() {
                let uri = frame["content"][0]["image_url"]["url"].as_str().context("Missing video frame")?;
                let raw = base64::engine::general_purpose::STANDARD.decode(uri.strip_prefix("data:image/jpeg;base64,").context("Invalid frame")?)?;
                let pixels = image::load_from_memory(&raw)?.thumbnail(width, 384).to_rgb8();
                let x = i as u32 * width + (width - pixels.width()) / 2;
                let y = (384 - pixels.height()) / 2;
                image::imageops::replace(&mut canvas, &pixels, x as i64, y as i64);
                evidence.push(json!({"id":format!("frame:{i}"),"coordinates":coordinates,"panel":i+1,"position":"left_to_right","coverage":"sampled_video_frame"}));
            }
            let input = image_input(image::DynamicImage::ImageRgb8(canvas), Coordinates::default())?.0;
            Ok(json!({"image":input,"coverage":"sampled_video_frames","duration_seconds":seconds,"evidence":evidence,"unavailable_sample_times":unavailable,"audio_analyzed":false}))
        })
    }).join().map_err(|_| anyhow::anyhow!("Video decoding worker failed"))?
}
/// Give the decoder its own runtime thread, just like crawl thumbnails. Dropping
/// a nested runtime inside the coordinator's blocking pool can wait on itself
/// after a cancelled Windows pipe read.
pub fn video_preview_snapshot(source: tempfile::NamedTempFile) -> Result<Value> {
    std::thread::spawn(move || -> Result<Value> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        runtime.block_on(async { Ok(video_frame(source.path(), 0.0).await?.0) })
    })
    .join()
    .map_err(|_| anyhow::anyhow!("Video preview worker failed"))?
}
pub fn probe_image() -> Result<Value> {
    image_input(
        image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
            32,
            32,
            image::Rgb([220, 170, 30]),
        )),
        Coordinates::default(),
    )
    .map(|p| p.0)
}
pub fn vision_probe() -> Result<(Value, Value)> {
    let colors = [
        ("red", [240, 10, 10]),
        ("green", [10, 220, 10]),
        ("blue", [10, 10, 240]),
    ];
    let seed = uuid::Uuid::new_v4();
    let left = seed.as_bytes()[0] as usize % colors.len();
    let right = (left + 1 + seed.as_bytes()[1] as usize % 2) % colors.len();
    let pixels = image::RgbImage::from_fn(128, 64, |x, _| {
        image::Rgb(colors[if x < 64 { left } else { right }].1)
    });
    Ok((
        image_input(
            image::DynamicImage::ImageRgb8(pixels),
            Coordinates::default(),
        )?
        .0,
        json!({"left_color":colors[left].0,"right_color":colors[right].0}),
    ))
}
pub fn probe_audio() -> Value {
    let mut bytes = Vec::new();
    let samples = 1600u32;
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36 + samples * 2).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16u32.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&16000u32.to_le_bytes());
    bytes.extend_from_slice(&32000u32.to_le_bytes());
    bytes.extend_from_slice(&2u16.to_le_bytes());
    bytes.extend_from_slice(&16u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&(samples * 2).to_le_bytes());
    bytes.resize(44 + samples as usize * 2, 0);
    json!({"content":[{"type":"input_audio","input_audio":{"data":base64::engine::general_purpose::STANDARD.encode(bytes),"format":"wav"}}]})
}
