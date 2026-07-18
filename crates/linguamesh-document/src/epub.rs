use std::collections::HashSet;
use std::io::{Cursor, Read, Write};
use std::path::Path;

use quick_xml::Reader;
use quick_xml::escape::{escape, unescape};
use quick_xml::events::Event;
use zip::ZipArchive;
use zip::write::ZipWriter;

use crate::{DocumentError, DocumentJob, DocumentSegment, DocumentSegmentKind, MAX_DOCUMENT_BYTES};

const MAX_EPUB_ENTRIES: usize = 512;
const MAX_EPUB_SEGMENTS: usize = 10_000;

#[derive(Clone, Copy)]
struct TextSpan {
    content_start: usize,
    content_end: usize,
}

fn has_extension(name: &str, extension: &str) -> bool {
    Path::new(name)
        .extension()
        .and_then(|value| value.to_str())
        .is_some_and(|value| value.eq_ignore_ascii_case(extension))
}

fn archive_names(package: &[u8]) -> Result<Vec<String>, DocumentError> {
    if package.len() > MAX_DOCUMENT_BYTES {
        return Err(DocumentError::TooLarge);
    }
    let mut archive =
        ZipArchive::new(Cursor::new(package)).map_err(|_| DocumentError::InvalidStructure)?;
    if archive.is_empty() || archive.len() > MAX_EPUB_ENTRIES {
        return Err(DocumentError::InvalidStructure);
    }
    let first = archive
        .by_index(0)
        .map_err(|_| DocumentError::InvalidStructure)?;
    if first.name() != "mimetype" || first.is_dir() || first.encrypted() || first.is_symlink() {
        return Err(DocumentError::InvalidStructure);
    }
    drop(first);
    let mut names = Vec::with_capacity(archive.len());
    let mut seen = HashSet::with_capacity(archive.len());
    let mut total_size = 0usize;
    for index in 0..archive.len() {
        let mut file = archive
            .by_index(index)
            .map_err(|_| DocumentError::InvalidStructure)?;
        if file.encrypted() || file.is_symlink() || file.enclosed_name().is_none() {
            return Err(DocumentError::InvalidStructure);
        }
        let name = file.name().to_owned();
        if name.is_empty() || name.contains('\\') || !seen.insert(name.clone()) {
            return Err(DocumentError::InvalidStructure);
        }
        let size = usize::try_from(file.size()).map_err(|_| DocumentError::TooLarge)?;
        total_size = total_size
            .checked_add(size)
            .ok_or(DocumentError::TooLarge)?;
        if total_size > MAX_DOCUMENT_BYTES {
            return Err(DocumentError::TooLarge);
        }
        if name == "mimetype" {
            let mut contents = Vec::new();
            file.read_to_end(&mut contents)
                .map_err(|_| DocumentError::InvalidStructure)?;
            if contents != b"application/epub+zip" {
                return Err(DocumentError::InvalidStructure);
            }
        }
        names.push(name);
    }
    if !seen.contains("mimetype") || !seen.contains("META-INF/container.xml") {
        return Err(DocumentError::InvalidStructure);
    }
    if !names.iter().any(|name| has_extension(name, "opf"))
        || !names
            .iter()
            .any(|name| has_extension(name, "xhtml") || has_extension(name, "html"))
    {
        return Err(DocumentError::InvalidStructure);
    }
    for name in names
        .iter()
        .filter(|name| *name == "META-INF/container.xml" || has_extension(name, "opf"))
    {
        let mut file = archive
            .by_name(name)
            .map_err(|_| DocumentError::InvalidStructure)?;
        let mut data = Vec::new();
        file.read_to_end(&mut data)
            .map_err(|_| DocumentError::InvalidStructure)?;
        let xml = std::str::from_utf8(&data).map_err(|_| DocumentError::InvalidStructure)?;
        validate_xml(xml)?;
    }
    Ok(names)
}

fn validate_xml(xml: &str) -> Result<(), DocumentError> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(false);
    loop {
        match reader.read_event() {
            Ok(Event::Eof) => return Ok(()),
            Ok(Event::DocType(_)) | Err(_) => return Err(DocumentError::InvalidStructure),
            Ok(_) => {}
        }
    }
}

fn tag_end(xml: &str, start: usize) -> Result<usize, DocumentError> {
    let bytes = xml.as_bytes();
    let mut quote = None;
    for (index, byte) in bytes.iter().copied().enumerate().skip(start + 1) {
        if let Some(expected) = quote {
            if byte == expected {
                quote = None;
            }
            continue;
        }
        match byte {
            b'"' | b'\'' => quote = Some(byte),
            b'>' => return Ok(index + 1),
            _ => {}
        }
    }
    Err(DocumentError::InvalidStructure)
}

fn element_name(raw: &str) -> Option<&str> {
    let body = raw
        .strip_prefix('<')?
        .strip_prefix('/')
        .unwrap_or(raw.strip_prefix('<')?);
    let body = body.trim_start();
    let end = body
        .find(|character: char| character.is_ascii_whitespace() || matches!(character, '/' | '>'))
        .unwrap_or(body.len());
    (!body[..end].is_empty()).then_some(&body[..end])
}

fn scan_xhtml(xml: &str) -> Result<Vec<TextSpan>, DocumentError> {
    validate_xml(xml)?;
    if xml.is_empty() || xml.as_bytes().contains(&0) {
        return Err(DocumentError::InvalidStructure);
    }
    let mut spans = Vec::new();
    let mut stack = Vec::<String>::new();
    let mut cursor = 0usize;
    while cursor < xml.len() {
        if xml.as_bytes()[cursor] != b'<' {
            let end = xml[cursor..]
                .find('<')
                .map_or(xml.len(), |offset| cursor + offset);
            if !stack
                .iter()
                .any(|name| matches!(name.as_str(), "script" | "style"))
                && unescape(&xml[cursor..end])
                    .map_err(|_| DocumentError::InvalidStructure)?
                    .chars()
                    .any(|character| !character.is_whitespace())
            {
                spans.push(TextSpan {
                    content_start: cursor,
                    content_end: end,
                });
            }
            cursor = end;
            continue;
        }
        if xml[cursor..].starts_with("<!--") {
            let end = xml[cursor + 4..]
                .find("-->")
                .map(|offset| cursor + 4 + offset + 3)
                .ok_or(DocumentError::InvalidStructure)?;
            cursor = end;
            continue;
        }
        let end = tag_end(xml, cursor)?;
        let raw = &xml[cursor..end];
        if raw.starts_with("<?") || raw.starts_with("<!") {
            cursor = end;
            continue;
        }
        let is_closing = raw.starts_with("</");
        let is_self_closing = !is_closing && raw.trim_end().ends_with("/>");
        if let Some(name) = element_name(raw) {
            let name = name.to_ascii_lowercase();
            if is_closing {
                if stack.pop().as_deref() != Some(name.as_str()) {
                    return Err(DocumentError::InvalidStructure);
                }
            } else if !is_self_closing {
                stack.push(name);
            }
        }
        cursor = end;
        if spans.len() > MAX_EPUB_SEGMENTS {
            return Err(DocumentError::InvalidStructure);
        }
    }
    if !stack.is_empty() {
        return Err(DocumentError::InvalidStructure);
    }
    Ok(spans)
}

fn translatable_part(name: &str) -> bool {
    !name.starts_with("META-INF/") && (has_extension(name, "xhtml") || has_extension(name, "html"))
}

pub(crate) fn inspect(package: &[u8]) -> Result<Vec<DocumentSegment>, DocumentError> {
    let names = archive_names(package)?;
    let mut archive =
        ZipArchive::new(Cursor::new(package)).map_err(|_| DocumentError::InvalidStructure)?;
    let mut segments = Vec::new();
    for name in names.iter().filter(|name| translatable_part(name)) {
        let mut file = archive
            .by_name(name)
            .map_err(|_| DocumentError::InvalidStructure)?;
        let mut data = Vec::new();
        file.read_to_end(&mut data)
            .map_err(|_| DocumentError::InvalidStructure)?;
        let xml = std::str::from_utf8(&data).map_err(|_| DocumentError::InvalidStructure)?;
        for span in scan_xhtml(xml)? {
            let text = unescape(&xml[span.content_start..span.content_end])
                .map_err(|_| DocumentError::InvalidStructure)?
                .into_owned();
            segments.push(DocumentSegment {
                index: segments.len(),
                kind: if text.chars().any(|character| !character.is_whitespace()) {
                    DocumentSegmentKind::Prose
                } else {
                    DocumentSegmentKind::Verbatim
                },
                source_text: text,
                translated_text: None,
                line_ending: String::new(),
            });
        }
    }
    if segments.is_empty() {
        return Err(DocumentError::InvalidStructure);
    }
    Ok(segments)
}

fn rewrite_xhtml(
    xml: &str,
    segments: &[DocumentSegment],
    cursor: &mut usize,
) -> Result<Vec<u8>, DocumentError> {
    let spans = scan_xhtml(xml)?;
    let mut output = String::with_capacity(xml.len());
    let mut byte_cursor = 0usize;
    for span in spans {
        let segment = segments
            .get(*cursor)
            .ok_or(DocumentError::InvalidStructure)?;
        let source = unescape(&xml[span.content_start..span.content_end])
            .map_err(|_| DocumentError::InvalidStructure)?;
        if source.as_ref() != segment.source_text {
            return Err(DocumentError::InvalidStructure);
        }
        output.push_str(&xml[byte_cursor..span.content_start]);
        match segment.kind {
            DocumentSegmentKind::Verbatim => {
                output.push_str(&xml[span.content_start..span.content_end])
            }
            DocumentSegmentKind::Prose => {
                let translated = segment
                    .translated_text
                    .as_deref()
                    .ok_or(DocumentError::SegmentIncomplete(segment.index))?;
                output.push_str(&escape(translated));
            }
        }
        byte_cursor = span.content_end;
        *cursor = cursor.saturating_add(1);
    }
    output.push_str(&xml[byte_cursor..]);
    Ok(output.into_bytes())
}

fn rewrite_language(xml: &str, target_locale: Option<&str>) -> Result<Vec<u8>, DocumentError> {
    validate_xml(xml)?;
    let Some(target_locale) = target_locale.filter(|value| !value.is_empty()) else {
        return Ok(xml.as_bytes().to_vec());
    };
    let mut cursor = 0usize;
    while let Some(relative) = xml[cursor..].find('<') {
        let start = cursor + relative;
        let end = tag_end(xml, start)?;
        let raw = &xml[start..end];
        if element_name(raw).is_some_and(|name| name.eq_ignore_ascii_case("dc:language"))
            && !raw.starts_with("</")
            && !raw.trim_end().ends_with("/>")
        {
            let content_start = end;
            let content_end = xml[content_start..]
                .find("</dc:language>")
                .map(|offset| content_start + offset)
                .ok_or(DocumentError::InvalidStructure)?;
            let mut output = String::with_capacity(xml.len());
            output.push_str(&xml[..content_start]);
            output.push_str(&escape(target_locale));
            output.push_str(&xml[content_end..]);
            return Ok(output.into_bytes());
        }
        cursor = end;
    }
    Ok(xml.as_bytes().to_vec())
}

pub(crate) fn reconstruct(
    job: &DocumentJob,
    package: &[u8],
    target_locale: Option<&str>,
) -> Result<Vec<u8>, DocumentError> {
    let names = archive_names(package)?;
    let mut archive =
        ZipArchive::new(Cursor::new(package)).map_err(|_| DocumentError::InvalidStructure)?;
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    let mut segment_cursor = 0usize;
    for name in names {
        let mut file = archive
            .by_name(&name)
            .map_err(|_| DocumentError::InvalidStructure)?;
        let is_dir = file.is_dir();
        let options = file.options();
        let mut data = Vec::new();
        file.read_to_end(&mut data)
            .map_err(|_| DocumentError::InvalidStructure)?;
        drop(file);
        if translatable_part(&name) {
            let xml = std::str::from_utf8(&data).map_err(|_| DocumentError::InvalidStructure)?;
            data = rewrite_xhtml(xml, &job.segments, &mut segment_cursor)?;
        } else if has_extension(&name, "opf") {
            let xml = std::str::from_utf8(&data).map_err(|_| DocumentError::InvalidStructure)?;
            data = rewrite_language(xml, target_locale)?;
        }
        if is_dir {
            writer
                .add_directory(&name, options)
                .map_err(|_| DocumentError::InvalidStructure)?;
        } else {
            writer
                .start_file(&name, options)
                .map_err(|_| DocumentError::InvalidStructure)?;
            writer
                .write_all(&data)
                .map_err(|_| DocumentError::InvalidStructure)?;
        }
    }
    if segment_cursor != job.segments.len() {
        return Err(DocumentError::InvalidStructure);
    }
    let output = writer
        .finish()
        .map_err(|_| DocumentError::InvalidStructure)?
        .into_inner();
    if output.len() > MAX_DOCUMENT_BYTES {
        return Err(DocumentError::OutputTooLarge);
    }
    Ok(output)
}
