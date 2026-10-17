use retriever_engine::{extract, media};
use std::{io::Write, path::Path};

#[test]
fn streaming_source_preserves_hash_snapshot_and_cross_buffer_screening() {
    let source = tempfile::NamedTempFile::new().unwrap();
    let bytes = vec![b'v'; 3 * 65536 + 17];
    std::fs::write(source.path(), &bytes).unwrap();
    let mut snapshot = tempfile::NamedTempFile::new().unwrap();
    let (digest, size) = extract::stream_source(
        source.path(),
        bytes.len() as u64,
        Some(snapshot.as_file_mut()),
    )
    .unwrap();
    assert_eq!(digest, retriever_engine::digest(&bytes));
    assert_eq!(size, bytes.len() as u64);
    assert_eq!(std::fs::read(snapshot.path()).unwrap(), bytes);
    assert!(extract::stream_source(source.path(), 65536, None).is_err());
    let mut secret = vec![b' '; 65536 - 7];
    secret.extend_from_slice(b"api_key = abcdefgh123456789");
    std::fs::write(source.path(), secret).unwrap();
    assert!(
        extract::stream_source(source.path(), 100000, None)
            .unwrap_err()
            .to_string()
            .contains("secret_content")
    );
    let limits = serde_json::json!({"max_file_bytes":1024,"max_media_bytes":2147483648u64});
    assert_eq!(extract::source_limit(&limits, "recording.MP4"), 2147483648);
    assert_eq!(extract::source_limit(&limits, "report.pdf"), 1024);
    assert_eq!(
        extract::source_limit(&serde_json::json!({"max_file_bytes":1024}), "recording.mp4"),
        1024
    );
}

fn office(parts: &[(&str, &str)]) -> Vec<u8> {
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, content) in parts {
        zip.start_file(*name, zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(content.as_bytes()).unwrap();
    }
    zip.finish().unwrap().into_inner()
}

#[test]
fn spreadsheet_dates_respect_style_epoch_and_cached_formula_values() {
    for (epoch, serial, expected) in [
        ("0", "46270", "2026-09-05"),
        ("1", "0", "1904-01-01"),
        ("0", "60", "1900-02-29"),
    ] {
        let workbook = format!(
            r#"<workbook xmlns:r="r"><workbookPr date1904="{epoch}"/><sheets><sheet name="Schedule" r:id="one"/></sheets></workbook>"#
        );
        let sheet = format!(
            r#"<worksheet><sheetData><row r="1"><c r="A1" s="0"><v>{serial}</v></c><c r="B1" s="1"><f>A1+0.5</f><v>{}</v></c><c r="C1"><v>46270</v></c></row></sheetData></worksheet>"#,
            serial.parse::<f64>().unwrap() + 0.5
        );
        let book = office(&[
            ("xl/workbook.xml", &workbook),
            (
                "xl/_rels/workbook.xml.rels",
                r#"<Relationships><Relationship Id="one" Type="x/worksheet" Target="worksheets/sheet1.xml"/></Relationships>"#,
            ),
            (
                "xl/styles.xml",
                r#"<styleSheet><numFmts><numFmt numFmtId="164" formatCode="yyyy-mm-dd hh:mm:ss"/></numFmts><cellXfs><xf numFmtId="14"/><xf numFmtId="164"/></cellXfs></styleSheet>"#,
            ),
            ("xl/worksheets/sheet1.xml", &sheet),
        ]);
        let result = extract::extract(Path::new("dates.xlsx"), &book, 5000, 512).unwrap();
        assert!(
            result
                .text
                .contains(&format!("{expected}\t{expected} 12:00:00\t46270")),
            "{}",
            result.text
        );
        assert_eq!(
            result.passages[0].coordinates.cell_start.as_deref(),
            Some("A1")
        );
        assert_eq!(
            result.passages[0].coordinates.cell_end.as_deref(),
            Some("C1")
        );
    }
}

#[test]
fn windows_unicode_text_is_lossless_and_decoded_secrets_are_rejected() {
    let source = "# Planning\n日本語 🐕 — Quarterly budget";
    for little in [true, false] {
        let mut bytes = if little {
            vec![0xff, 0xfe]
        } else {
            vec![0xfe, 0xff]
        };
        for unit in source.encode_utf16() {
            bytes.extend(if little {
                unit.to_le_bytes()
            } else {
                unit.to_be_bytes()
            });
        }
        let result = extract::extract(Path::new("notes.md"), &bytes, 5000, 128).unwrap();
        assert_eq!(result.text, source);
        assert_eq!(
            result
                .passages
                .iter()
                .map(|p| p.text.as_str())
                .collect::<String>(),
            source
        );
    }
    let mut secret = vec![0xff, 0xfe];
    for unit in "api_key = abcdefgh123456789".encode_utf16() {
        secret.extend(unit.to_le_bytes());
    }
    assert!(
        extract::extract(Path::new("notes.txt"), &secret, 5000, 128)
            .unwrap_err()
            .to_string()
            .contains("secret_content")
    );
    assert!(extract::decode_text(&[0xff, 0xfe, 0x01]).is_err());
    assert!(extract::decode_text(b"binary\0data").is_err());
}

#[test]
fn office_evidence_preserves_sheets_cells_notes_and_headers() {
    let book = office(&[
        (
            "xl/workbook.xml",
            r#"<workbook xmlns:r="r"><sheet name="Budget 2026" r:id="one"/></workbook>"#,
        ),
        (
            "xl/_rels/workbook.xml.rels",
            r#"<Relationships><Relationship Id="one" Type="x/worksheet" Target="worksheets/book.xml"/></Relationships>"#,
        ),
        (
            "xl/worksheets/book.xml",
            r#"<worksheet><sheetData><row r="3"><c r="A3" t="inlineStr"><is><t>Hiring plan</t></is></c><c r="F3"><f>SUM(B3:E3)</f></c></row></sheetData></worksheet>"#,
        ),
    ]);
    let result = extract::extract(Path::new("plan.xlsx"), &book, 5000, 512).unwrap();
    assert!(result.text.contains("=SUM(B3:E3)"));
    assert_eq!(
        result.passages[0].coordinates.sheet.as_deref(),
        Some("Budget 2026")
    );
    assert_eq!(
        result.passages[0].coordinates.cell_start.as_deref(),
        Some("A3")
    );
    assert_eq!(
        result.passages[0].coordinates.cell_end.as_deref(),
        Some("F3")
    );
    let slides = office(&[
        (
            "ppt/presentation.xml",
            r#"<presentation xmlns:r="r"><sldId r:id="s"/></presentation>"#,
        ),
        (
            "ppt/_rels/presentation.xml.rels",
            r#"<Relationships><Relationship Id="s" Type="x/slide" Target="slides/s9.xml"/></Relationships>"#,
        ),
        ("ppt/slides/s9.xml", "<slide><p><t>Roadmap</t></p></slide>"),
        (
            "ppt/slides/_rels/s9.xml.rels",
            r#"<Relationships><Relationship Id="n" Type="x/notesSlide" Target="../notesSlides/n2.xml"/></Relationships>"#,
        ),
        (
            "ppt/notesSlides/n2.xml",
            "<notes><p><t>Launch after procurement approval</t></p></notes>",
        ),
    ]);
    let result = extract::extract(Path::new("roadmap.pptx"), &slides, 5000, 512).unwrap();
    let notes = result
        .passages
        .iter()
        .find(|p| p.text.contains("procurement"))
        .unwrap();
    assert_eq!(notes.coordinates.slide, Some(1));
    assert_eq!(notes.coordinates.section.as_deref(), Some("speaker_notes"));
    let document = office(&[
        (
            "word/document.xml",
            "<document><p><t>Body</t></p></document>",
        ),
        (
            "word/_rels/document.xml.rels",
            r#"<Relationships><Relationship Id="h" Type="x/header" Target="header2.xml"/><Relationship Id="evil" Type="x/header" TargetMode="External" Target="https://invalid.example/"/></Relationships>"#,
        ),
        (
            "word/header2.xml",
            "<hdr><p><t>Project Sequoia</t></p></hdr>",
        ),
    ]);
    let result = extract::extract(Path::new("spec.docx"), &document, 5000, 512).unwrap();
    assert!(result.text.contains("Sequoia"));
    assert_eq!(
        result.passages[1].coordinates.section.as_deref(),
        Some("word/header2.xml")
    );
}

#[test]
fn office_budget_and_relationship_traversal_fail_closed() {
    let doc = office(&[(
        "word/document.xml",
        "<document><p><t>Some long ordinary text</t></p></document>",
    )]);
    assert!(extract::extract(Path::new("doc.docx"), &doc, 8, 512).is_err());
    let doc = office(&[
        (
            "word/document.xml",
            "<document><p><t>Body</t></p></document>",
        ),
        (
            "word/_rels/document.xml.rels",
            r#"<Relationships><Relationship Id="h" Type="x/header" Target="../../escape.xml"/></Relationships>"#,
        ),
    ]);
    assert!(extract::extract(Path::new("doc.docx"), &doc, 5000, 512).is_err());
}

#[test]
fn thumbnail_sampling_covers_short_and_long_videos() {
    assert_eq!(media::thumbnail_times(0.2), vec![0.1]);
    assert_eq!(media::thumbnail_times(100.0), vec![10.0, 50.0, 90.0]);
    assert!(media::thumbnail_times(f64::INFINITY).is_empty());
    assert!(media::thumbnail_times(0.0).is_empty());
}

#[test]
fn bmp_and_tiff_decode_with_bounded_pixels() {
    for format in [image::ImageFormat::Bmp, image::ImageFormat::Tiff] {
        let image = image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
            32,
            16,
            image::Rgb([220, 180, 10]),
        ));
        let mut bytes = std::io::Cursor::new(Vec::new());
        image.write_to(&mut bytes, format).unwrap();
        assert!(
            media::image_pixels(bytes.get_ref()).unwrap()["content"][0]["image_url"]["url"]
                .as_str()
                .unwrap()
                .starts_with("data:image/jpeg;base64,")
        );
    }
}

#[test]
fn jpeg_camera_orientation_is_applied_before_vision() {
    use base64::Engine as _;
    let pixels = image::RgbImage::from_pixel(40, 20, image::Rgb([240, 10, 10]));
    let mut jpeg = Vec::new();
    image::codecs::jpeg::JpegEncoder::new(&mut jpeg)
        .encode_image(&pixels)
        .unwrap();
    // EXIF little-endian orientation=6 (90 degrees clockwise).
    let exif = b"Exif\0\0II\x2a\0\x08\0\0\0\x01\0\x12\x01\x03\0\x01\0\0\0\x06\0\0\0\0\0\0\0";
    let mut oriented = vec![0xff, 0xd8, 0xff, 0xe1];
    oriented.extend(((exif.len() + 2) as u16).to_be_bytes());
    oriented.extend(exif);
    oriented.extend(&jpeg[2..]);
    let input = media::image_pixels(&oriented).unwrap();
    let uri = input["content"][0]["image_url"]["url"].as_str().unwrap();
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(uri.strip_prefix("data:image/jpeg;base64,").unwrap())
        .unwrap();
    let result = image::load_from_memory(&decoded).unwrap();
    assert_eq!((result.width(), result.height()), (20, 40));
}

pub fn pdf_fixture(text: &str) -> Vec<u8> {
    let stream = format!("BT /F1 20 Tf 40 100 Td ({text}) Tj ET");
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_owned(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 400 200] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>".to_owned(),
        format!("<< /Length {} >>\nstream\n{stream}\nendstream",stream.len()),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_owned()
    ];
    let mut bytes = b"%PDF-1.4\n".to_vec();
    let mut offsets = Vec::new();
    for (i, object) in objects.iter().enumerate() {
        offsets.push(bytes.len());
        bytes.extend(format!("{} 0 obj\n{}\nendobj\n", i + 1, object).as_bytes());
    }
    let start = bytes.len();
    bytes.extend(b"xref\n0 6\n0000000000 65535 f \n");
    for offset in offsets {
        bytes.extend(format!("{offset:010} 00000 n \n").as_bytes());
    }
    bytes.extend(
        format!("trailer\n<< /Size 6 /Root 1 0 R >>\nstartxref\n{start}\n%%EOF\n").as_bytes(),
    );
    bytes
}

#[test]
#[ignore = "Requires packaged PDFium: RETRIEVER_PDFIUM"]
fn packaged_pdfium_extracts_and_renders_real_pdf_bytes() {
    let bytes = pdf_fixture("The Sequoia budget is 45000 dollars");
    let result = extract::extract(Path::new("budget.pdf"), &bytes, 5000, 256).unwrap();
    assert!(result.text.contains("45000"), "{}", result.detail);
    assert_eq!(result.passages[0].coordinates.page, Some(1));
    assert_eq!(media::pdf_page(&bytes, 0).unwrap().1.page, Some(1));
    let blank = media::pdf_text(&pdf_fixture(""), 5000, 256).unwrap();
    assert!(blank.text.is_empty());
    assert!(blank.passages.is_empty());
    assert!(blank.detail.contains("scanned_pdf"));
    assert!(
        extract::extract(
            Path::new("secret.pdf"),
            &pdf_fixture("api_key = abcdefgh123456789"),
            5000,
            256
        )
        .is_err()
    );
}

#[test]
#[ignore = "Requires packaged FFmpeg: RETRIEVER_MEDIA_BIN"]
fn video_analysis_uses_verified_snapshot_and_three_timestamps() {
    use base64::Engine as _;
    let dir = tempfile::tempdir().unwrap();
    let video = dir.path().join("colours ü.mp4");
    // Fixture encoding can use a development encoder; decoding must still use
    // RETRIEVER_MEDIA_BIN so the shipped helper is the one under test.
    let encoder = std::env::var("RETRIEVER_FIXTURE_MEDIA_BIN")
        .or_else(|_| std::env::var("RETRIEVER_MEDIA_BIN"))
        .unwrap();
    let binary = Path::new(&encoder).join(if cfg!(windows) {
        "ffmpeg.exe"
    } else {
        "ffmpeg"
    });
    let status = std::process::Command::new(binary)
        .args([
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "color=c=red:s=96x64:d=4",
            "-c:v",
            "libx264",
            "-color_range",
            "tv",
            "-y",
        ])
        .arg(&video)
        .status()
        .unwrap();
    assert!(status.success());
    let result = media::video_thumbnails(std::fs::read(&video).unwrap(), "mp4").unwrap();
    assert_eq!(result["evidence"].as_array().unwrap().len(), 3);
    assert_eq!(result["audio_analyzed"], false);
    let times: Vec<f64> = result["evidence"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["coordinates"]["start_seconds"].as_f64().unwrap())
        .collect();
    assert!(times[0] < times[1] && times[1] < times[2]);
    let uri = result["image"]["content"][0]["image_url"]["url"]
        .as_str()
        .unwrap();
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(uri.strip_prefix("data:image/jpeg;base64,").unwrap())
        .unwrap();
    let pixels = image::load_from_memory(&bytes).unwrap().to_rgb8();
    assert_eq!(pixels.width(), 1024);
    for x in [170, 512, 850] {
        assert!(pixels.get_pixel(x, 192).0[0] > 200);
    }
    assert!(
        media::video_thumbnails(
            b"#EXTM3U\nhttps://invalid.example/movie.ts\n".to_vec(),
            "mp4"
        )
        .is_err()
    );
}

#[test]
#[ignore = "Requires packaged FFmpeg: RETRIEVER_MEDIA_BIN"]
fn video_with_long_audio_tail_retains_available_frames() {
    let dir = tempfile::tempdir().unwrap();
    let video = dir.path().join("short-picture-long-audio.mp4");
    let encoder = std::env::var("RETRIEVER_FIXTURE_MEDIA_BIN")
        .or_else(|_| std::env::var("RETRIEVER_MEDIA_BIN"))
        .unwrap();
    let binary = Path::new(&encoder).join(if cfg!(windows) {
        "ffmpeg.exe"
    } else {
        "ffmpeg"
    });
    let status = std::process::Command::new(binary)
        .args([
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "color=c=blue:s=96x64:d=2",
            "-f",
            "lavfi",
            "-i",
            "anullsrc=r=8000:cl=mono:d=10",
            "-c:v",
            "libx264",
            "-c:a",
            "aac",
            "-y",
        ])
        .arg(&video)
        .status()
        .unwrap();
    assert!(status.success());
    let result = media::video_thumbnails(std::fs::read(video).unwrap(), "mp4").unwrap();
    assert_eq!(result["evidence"].as_array().unwrap().len(), 1);
    assert_eq!(
        result["unavailable_sample_times"].as_array().unwrap().len(),
        2
    );
    assert_eq!(result["audio_analyzed"], false);
    assert!(
        result["evidence"][0]["coordinates"]["start_seconds"]
            .as_f64()
            .unwrap()
            < 2.0
    );
}

#[tokio::test]
#[ignore = "Requires packaged FFmpeg: RETRIEVER_MEDIA_BIN"]
async fn quick_video_preview_releases_worker_and_rejects_changed_pin() {
    use serde_json::json;
    use std::time::Duration;
    let directory = tempfile::Builder::new()
        .prefix("retriever-preview-test-")
        .tempdir()
        .unwrap();
    // macOS exposes its temporary directory through /var -> /private/var.
    // Use the real fixture root without weakening the agent's symlink guard.
    let root = directory.path().canonicalize().unwrap();
    let video = root.join("quick.mp4");
    let encoder = std::env::var("RETRIEVER_FIXTURE_MEDIA_BIN")
        .or_else(|_| std::env::var("RETRIEVER_MEDIA_BIN"))
        .unwrap();
    let binary = Path::new(&encoder).join(if cfg!(windows) {
        "ffmpeg.exe"
    } else {
        "ffmpeg"
    });
    assert!(
        std::process::Command::new(binary)
            .args([
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                "color=c=red:s=96x64:d=1",
                "-c:v",
                "mpeg4",
                "-y"
            ])
            .arg(&video)
            .status()
            .unwrap()
            .success()
    );
    let engine = retriever_engine::engine::Engine::open(&root.join("state")).unwrap();
    let mut config = retriever_engine::config::Config::default();
    config.devices[0].roots = vec![root.to_string_lossy().to_string()];
    let pin = retriever_engine::agent::pin(&video).unwrap();
    for _ in 0..2 {
        let result = tokio::time::timeout(
            Duration::from_secs(10),
            engine.source(
                &config.devices[0],
                "preview_image",
                json!({"path":video,"pin":pin}),
            ),
        )
        .await
        .expect("Preview worker must finish")
        .unwrap();
        assert!(
            result["image"]
                .as_str()
                .unwrap()
                .starts_with("data:image/jpeg;base64,")
        );
        assert_eq!(result["coverage"], "Video frame at 0:00 · audio not played");
    }
    assert!(
        engine
            .source(
                &config.devices[0],
                "preview_image",
                json!({"path":video,"pin":{"size":1}})
            )
            .await
            .unwrap_err()
            .to_string()
            .contains("changed")
    );
}
