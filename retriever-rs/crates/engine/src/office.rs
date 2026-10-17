//! Bounded Open XML extraction. Relationships determine document order; archive
//! filenames do not. External relationships and document type declarations are
//! never followed.
use crate::extract::{Extraction, passages};
use anyhow::{Context, Result, bail};
use quick_xml::events::Event;
use retriever_protocol::Coordinates;
use std::{
    collections::HashMap,
    io::{Cursor, Read},
};

#[derive(Default)]
struct Node {
    name: String,
    attrs: HashMap<String, String>,
    text: String,
    children: Vec<Node>,
}
impl Node {
    fn descendants<'a>(&'a self, name: &str, out: &mut Vec<&'a Node>) {
        if self.name == name {
            out.push(self);
        }
        for child in &self.children {
            child.descendants(name, out);
        }
    }
    fn find(&self, name: &str) -> Vec<&Node> {
        let mut out = Vec::new();
        self.descendants(name, &mut out);
        out
    }
    fn attr(&self, name: &str) -> &str {
        self.attrs.get(name).map(String::as_str).unwrap_or("")
    }
}

fn xml(text: &str) -> Result<Node> {
    let mut reader = quick_xml::Reader::from_str(text);
    reader.config_mut().expand_empty_elements = true;
    let mut stack = vec![Node::default()];
    let mut count = 0;
    loop {
        match reader.read_event()? {
            Event::Start(e) => {
                count += 1;
                if count > 500_000 || stack.len() > 64 {
                    bail!("archive_xml_budget");
                }
                let mut n = Node {
                    name: String::from_utf8(e.local_name().as_ref().to_vec())?,
                    ..Node::default()
                };
                for a in e.attributes() {
                    let a = a?;
                    n.attrs.insert(
                        String::from_utf8(a.key.as_ref().to_vec())?,
                        a.decode_and_unescape_value(reader.decoder())?.into_owned(),
                    );
                }
                stack.push(n);
            }
            Event::End(_) => {
                if stack.len() <= 1 {
                    bail!("Invalid Office XML nesting");
                }
                let n = stack.pop().unwrap();
                stack.last_mut().unwrap().children.push(n);
            }
            Event::Text(t) => stack
                .last_mut()
                .unwrap()
                .text
                .push_str(&quick_xml::escape::unescape(&t.decode()?)?),
            Event::CData(t) => stack.last_mut().unwrap().text.push_str(&t.decode()?),
            Event::GeneralRef(t) => {
                let encoded = format!("&{};", t.decode()?);
                stack
                    .last_mut()
                    .unwrap()
                    .text
                    .push_str(&quick_xml::escape::unescape(&encoded)?);
            }
            Event::DocType(_) => bail!("XML document types are not allowed"),
            Event::Eof => break,
            _ => {}
        }
    }
    if stack.len() != 1 {
        bail!("Incomplete Office XML");
    }
    Ok(stack.pop().unwrap())
}

struct Archive<'a> {
    zip: zip::ZipArchive<Cursor<&'a [u8]>>,
    consumed: u64,
}

/// Search dates as dates, not Excel's implementation-specific serial numbers.
/// Respect the workbook epoch, including Excel's fictitious 1900 leap day.
fn date_value(value: &str, format: &str, date1904: bool) -> Option<String> {
    let mut quoted = false;
    let mut bracket = false;
    let mut escape = false;
    let mut cleaned = String::new();
    for c in format.to_lowercase().chars() {
        if escape {
            escape = false;
            continue;
        }
        if c == '\\' || c == '_' || c == '*' {
            escape = true;
            continue;
        }
        if c == '"' {
            quoted = !quoted;
            continue;
        }
        if quoted {
            continue;
        }
        if c == '[' {
            bracket = true;
            continue;
        }
        if c == ']' {
            bracket = false;
            continue;
        }
        if !bracket {
            cleaned.push(c);
        }
    }
    if !cleaned.contains(['y', 'd']) {
        return None;
    }
    let number = value.parse::<f64>().ok()?;
    if !number.is_finite() || !(0.0..2958466.0).contains(&number) {
        return None;
    }
    let days = number.floor() as i64;
    let date = if !date1904 && days == 60 {
        "1900-02-29".to_owned()
    } else {
        let base = chrono::NaiveDate::from_ymd_opt(
            if date1904 { 1904 } else { 1899 },
            if date1904 { 1 } else { 12 },
            if date1904 { 1 } else { 31 },
        )?;
        base.checked_add_signed(chrono::Duration::days(
            days - i64::from(!date1904 && days > 60),
        ))?
        .format("%Y-%m-%d")
        .to_string()
    };
    if cleaned.contains('h') {
        let seconds = (number.fract() * 86400.0).round().clamp(0.0, 86399.0) as u32;
        Some(format!(
            "{date} {:02}:{:02}:{:02}",
            seconds / 3600,
            (seconds / 60) % 60,
            seconds % 60
        ))
    } else {
        Some(date)
    }
}
impl<'a> Archive<'a> {
    fn exists(&self, name: &str) -> bool {
        self.zip.index_for_name(name).is_some()
    }
    fn part(&mut self, name: &str) -> Result<Node> {
        let f = self
            .zip
            .by_name(name)
            .with_context(|| format!("Missing Office part: {name}"))?;
        self.consumed = self.consumed.saturating_add(f.size());
        if f.size() > 16 * 1024 * 1024 || self.consumed > 64 * 1024 * 1024 {
            bail!("archive_budget");
        }
        let mut text = String::new();
        f.take(16 * 1024 * 1024 + 1).read_to_string(&mut text)?;
        if text.len() > 16 * 1024 * 1024 {
            bail!("archive_entry_budget");
        }
        xml(&text)
    }
    fn ordered(
        &mut self,
        directory: &str,
        part: &str,
        tag: &str,
        kind: &str,
    ) -> Result<Vec<(String, String)>> {
        let document = self.part(&format!("{directory}/{part}.xml"))?;
        let relationships = self.part(&format!("{directory}/_rels/{part}.xml.rels"))?;
        let mut out = Vec::new();
        for node in document.find(tag) {
            let relationship = relationships
                .find("Relationship")
                .into_iter()
                .find(|r| {
                    r.attr("Id") == node.attr("r:id")
                        && r.attr("Type").ends_with(&format!("/{kind}"))
                })
                .context("Office relationship is missing")?;
            if !relationship.attr("TargetMode").is_empty()
                && relationship.attr("TargetMode") != "Internal"
            {
                bail!("External Office content is not allowed");
            }
            let target = relationship.attr("Target");
            if target.contains([':', '\\', '\0', '?', '#']) {
                bail!("Invalid Office part target");
            }
            let mut pieces = if target.starts_with('/') {
                vec![]
            } else {
                vec![directory]
            };
            for p in target.split('/') {
                match p {
                    "" | "." => {}
                    ".." => {
                        pieces.pop().context("Office target escapes the archive")?;
                    }
                    _ => pieces.push(p),
                }
            }
            out.push((node.attr("name").to_owned(), pieces.join("/")));
        }
        Ok(out)
    }
    fn related(&mut self, part: &str, kinds: &[&str]) -> Result<Vec<String>> {
        let (directory, name) = part.rsplit_once('/').context("Invalid Office part")?;
        let rels = format!("{directory}/_rels/{name}.rels");
        if !self.exists(&rels) {
            return Ok(vec![]);
        }
        let mut result = Vec::new();
        for relation in self.part(&rels)?.find("Relationship") {
            if relation.attr("TargetMode") == "External"
                || !kinds
                    .iter()
                    .any(|k| relation.attr("Type").ends_with(&format!("/{k}")))
            {
                continue;
            }
            let target = relation.attr("Target");
            if target.contains([':', '\\', '\0', '?', '#']) {
                bail!("Invalid Office part target");
            }
            let mut pieces: Vec<_> = if target.starts_with('/') {
                vec![]
            } else {
                directory.split('/').collect()
            };
            for piece in target.split('/') {
                match piece {
                    "" | "." => {}
                    ".." => {
                        pieces.pop().context("Office target escapes archive")?;
                    }
                    _ => pieces.push(piece),
                }
            }
            let path = pieces.join("/");
            if !result.contains(&path) {
                result.push(path);
            }
        }
        Ok(result)
    }
}

fn runs(node: &Node, result: &mut String) {
    if ["rPh", "phoneticPr", "del", "instrText"].contains(&node.name.as_str()) {
        return;
    }
    match node.name.as_str() {
        "t" => result.push_str(&node.text),
        "tab" => result.push('\t'),
        "br" | "cr" => result.push('\n'),
        _ => {}
    }
    for child in &node.children {
        runs(child, result);
    }
    if ["p", "tr"].contains(&node.name.as_str()) {
        result.push('\n');
    }
    if node.name == "tc" {
        result.push('\t');
    }
}

pub fn extract_structured(
    bytes: &[u8],
    extension: &str,
    maximum: usize,
    chunk: usize,
) -> Result<Extraction> {
    let zip = zip::ZipArchive::new(Cursor::new(bytes))?;
    if zip.len() > 20_000 {
        bail!("archive_entry_count_budget");
    }
    let mut archive = Archive { zip, consumed: 0 };
    let mut result = Extraction {
        text: String::new(),
        passages: vec![],
        detail: "text".into(),
    };
    let mut length = 0;
    let mut append = |text: String, coordinates: Coordinates| -> Result<()> {
        let size = text.chars().count();
        if length + size > maximum {
            bail!("extracted_text_budget");
        }
        if crate::paths::secret_content(text.as_bytes()) {
            bail!("secret_content");
        }
        for mut part in passages(&text, chunk, coordinates.page) {
            part.coordinates.slide = coordinates.slide;
            part.coordinates.sheet = coordinates.sheet.clone();
            part.coordinates.cell_start = coordinates.cell_start.clone();
            part.coordinates.cell_end = coordinates.cell_end.clone();
            part.coordinates.section = coordinates.section.clone();
            part.coordinates.char_start += length;
            part.coordinates.char_end += length;
            result.passages.push(part);
        }
        length += size;
        result.text.push_str(&text);
        Ok(())
    };
    match extension {
        "docx" => {
            let mut text = String::new();
            runs(&archive.part("word/document.xml")?, &mut text);
            append(
                text,
                Coordinates {
                    section: Some("document".into()),
                    ..Default::default()
                },
            )?;
            for path in archive.related(
                "word/document.xml",
                &["header", "footer", "footnotes", "endnotes"],
            )? {
                let mut text = String::new();
                runs(&archive.part(&path)?, &mut text);
                append(
                    text,
                    Coordinates {
                        section: Some(path),
                        ..Default::default()
                    },
                )?;
            }
        }
        "pptx" => {
            for (i, (_, path)) in archive
                .ordered("ppt", "presentation", "sldId", "slide")?
                .into_iter()
                .enumerate()
            {
                let coordinates = Coordinates {
                    slide: Some(i as u32 + 1),
                    section: Some("slide".into()),
                    ..Default::default()
                };
                let mut text = String::new();
                runs(&archive.part(&path)?, &mut text);
                text.push('\n');
                append(text, coordinates.clone())?;
                for notes in archive.related(&path, &["notesSlide"])? {
                    let mut text = String::new();
                    runs(&archive.part(&notes)?, &mut text);
                    append(
                        text,
                        Coordinates {
                            section: Some("speaker_notes".into()),
                            ..coordinates.clone()
                        },
                    )?;
                }
            }
        }
        "xlsx" => {
            let workbook = archive.part("xl/workbook.xml")?;
            let date1904 = workbook
                .find("workbookPr")
                .first()
                .is_some_and(|p| ["1", "true"].contains(&p.attr("date1904")));
            let mut formats: HashMap<u32, String> = [
                (14, "mm-dd-yy"),
                (15, "d-mmm-yy"),
                (16, "d-mmm"),
                (17, "mmm-yy"),
                (22, "m/d/yy h:mm"),
            ]
            .into_iter()
            .map(|(id, f)| (id, f.into()))
            .collect();
            let mut styles = Vec::new();
            if archive.exists("xl/styles.xml") {
                let sheet_styles = archive.part("xl/styles.xml")?;
                for format in sheet_styles.find("numFmt") {
                    if let Ok(id) = format.attr("numFmtId").parse::<u32>() {
                        formats.insert(id, format.attr("formatCode").to_owned());
                    }
                }
                if let Some(xfs) = sheet_styles.find("cellXfs").first() {
                    styles = xfs
                        .children
                        .iter()
                        .filter(|n| n.name == "xf")
                        .map(|n| n.attr("numFmtId").parse::<u32>().unwrap_or(0))
                        .collect();
                }
            }
            let shared = if archive
                .zip
                .file_names()
                .any(|n| n == "xl/sharedStrings.xml")
            {
                archive
                    .part("xl/sharedStrings.xml")?
                    .find("si")
                    .iter()
                    .map(|n| {
                        let mut text = String::new();
                        runs(n, &mut text);
                        text
                    })
                    .collect::<Vec<_>>()
            } else {
                Vec::new()
            };
            for (name, path) in archive.ordered("xl", "workbook", "sheet", "worksheet")? {
                let mut out = String::new();
                out.push_str(&name);
                out.push('\n');
                let sheet = archive.part(&path)?;
                let mut first_cell = None;
                let mut last_cell = None;
                let mut count = name.chars().count() + 1;
                for row in sheet.find("row") {
                    let row_start = out.len();
                    for (i, cell) in row.children.iter().filter(|n| n.name == "c").enumerate() {
                        let reference = cell.attr("r");
                        if !reference.is_empty() {
                            if first_cell.is_none() {
                                first_cell = Some(reference.to_owned());
                            }
                            last_cell = Some(reference.to_owned());
                        }
                        if i > 0 {
                            out.push('\t');
                        }
                        let value = cell
                            .children
                            .iter()
                            .find(|n| n.name == "v")
                            .map(|n| n.text.as_str())
                            .unwrap_or("");
                        match cell.attr("t") {
                            "s" => out.push_str(
                                shared
                                    .get(value.trim().parse::<usize>()?)
                                    .context("Invalid shared-string index")?,
                            ),
                            "inlineStr" => runs(cell, &mut out),
                            "b" => out.push_str(match value.trim() {
                                "0" => "FALSE",
                                "1" => "TRUE",
                                _ => {
                                    bail!("Invalid Boolean cell");
                                }
                            }),
                            _ if !value.is_empty() => {
                                let formatted =
                                    if cell.attr("t").is_empty() || cell.attr("t") == "n" {
                                        cell.attr("s")
                                            .parse::<usize>()
                                            .ok()
                                            .and_then(|s| styles.get(s))
                                            .and_then(|id| formats.get(id))
                                            .and_then(|format| date_value(value, format, date1904))
                                    } else {
                                        None
                                    };
                                out.push_str(formatted.as_deref().unwrap_or(value));
                            }
                            _ => {
                                // A missing cached result is still searchable as a formula.
                                if let Some(formula) = cell.children.iter().find(|n| n.name == "f")
                                {
                                    out.push('=');
                                    out.push_str(&formula.text);
                                }
                            }
                        }
                    }
                    out.push('\n');
                    count += out[row_start..].chars().count();
                    if count >= chunk.max(64) {
                        append(
                            std::mem::take(&mut out),
                            Coordinates {
                                sheet: Some(name.clone()),
                                cell_start: first_cell.take(),
                                cell_end: last_cell.take(),
                                ..Default::default()
                            },
                        )?;
                        count = 0;
                    }
                }
                out.push('\n');
                append(
                    out,
                    Coordinates {
                        sheet: Some(name),
                        cell_start: first_cell,
                        cell_end: last_cell,
                        ..Default::default()
                    },
                )?;
            }
        }
        _ => bail!("Unsupported Office document"),
    }
    if crate::paths::secret_content(result.text.as_bytes()) {
        bail!("secret_content");
    }
    Ok(result)
}
