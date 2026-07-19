use std::collections::HashSet;
use std::io::{Cursor, Read, Write};
use std::path::Path;

use quick_xml::Reader;
use quick_xml::escape::{escape, unescape};
use quick_xml::events::Event;
use zip::ZipArchive;
use zip::write::ZipWriter;

use crate::{DocumentError, DocumentJob, DocumentSegment, DocumentSegmentKind, MAX_DOCUMENT_BYTES};

const MAX_OOXML_ENTRIES: usize = 512;
const MAX_OOXML_COMPRESSION_RATIO: u64 = 200;
const MIN_OOXML_RATIO_CHECK_BYTES: u64 = 1024;

#[derive(Clone, Copy)]
pub(crate) enum PackageKind {
    Docx,
    Pptx,
    Xlsx,
}

impl PackageKind {
    fn main_part(self) -> &'static str {
        match self {
            Self::Docx => "word/document.xml",
            Self::Pptx => "ppt/presentation.xml",
            Self::Xlsx => "xl/workbook.xml",
        }
    }

    fn paragraph_close(self) -> &'static str {
        match self {
            Self::Docx => "</w:p>",
            Self::Pptx => "</a:p>",
            Self::Xlsx => "",
        }
    }

    fn text_open(self) -> &'static str {
        match self {
            Self::Docx => "<w:t",
            Self::Pptx => "<a:t",
            Self::Xlsx => "<t",
        }
    }
}

#[derive(Clone, Copy)]
struct TextSpan {
    content_start: usize,
    content_end: usize,
}

/// 检查 DOCX 包路径、条目数量和解压后的总大小，并返回排序后的条目名称。
fn archive_names(package: &[u8], kind: PackageKind) -> Result<Vec<String>, DocumentError> {
    if package.len() > MAX_DOCUMENT_BYTES {
        return Err(DocumentError::TooLarge);
    }
    let mut archive =
        ZipArchive::new(Cursor::new(package)).map_err(|_| DocumentError::InvalidStructure)?;
    if archive.is_empty() || archive.len() > MAX_OOXML_ENTRIES {
        return Err(DocumentError::InvalidStructure);
    }
    let mut total_size = 0usize;
    let mut names = Vec::with_capacity(archive.len());
    let mut seen = HashSet::with_capacity(archive.len());
    for index in 0..archive.len() {
        let file = archive
            .by_index(index)
            .map_err(|_| DocumentError::InvalidStructure)?;
        if file.encrypted() || file.is_symlink() || file.enclosed_name().is_none() {
            return Err(DocumentError::InvalidStructure);
        }
        let name = file.name();
        if name.is_empty() || name.contains('\\') || !seen.insert(name.to_owned()) {
            return Err(DocumentError::InvalidStructure);
        }
        let size = usize::try_from(file.size()).map_err(|_| DocumentError::TooLarge)?;
        let uncompressed_size = file.size();
        let compressed_size = file.compressed_size();
        if uncompressed_size >= MIN_OOXML_RATIO_CHECK_BYTES
            && (compressed_size == 0
                || uncompressed_size > compressed_size.saturating_mul(MAX_OOXML_COMPRESSION_RATIO))
        {
            return Err(DocumentError::TooLarge);
        }
        total_size = total_size
            .checked_add(size)
            .ok_or(DocumentError::TooLarge)?;
        if total_size > MAX_DOCUMENT_BYTES {
            return Err(DocumentError::TooLarge);
        }
        names.push(name.to_owned());
    }
    names.sort_unstable();
    if !seen.contains("[Content_Types].xml") || !seen.contains(kind.main_part()) {
        return Err(DocumentError::InvalidStructure);
    }
    let has_required_child = match kind {
        PackageKind::Pptx => names
            .iter()
            .any(|name| name.starts_with("ppt/slides/slide") && has_xml_extension(name)),
        PackageKind::Xlsx => names
            .iter()
            .any(|name| name.starts_with("xl/worksheets/sheet") && has_xml_extension(name)),
        PackageKind::Docx => true,
    };
    if !has_required_child {
        return Err(DocumentError::InvalidStructure);
    }
    Ok(names)
}

/// 只处理包含可见文本的 OOXML 部件，保留所有关系和二进制资源。
fn translatable_part(name: &str, kind: PackageKind) -> bool {
    if !has_xml_extension(name) {
        return false;
    }
    match kind {
        PackageKind::Docx => {
            if !name.starts_with("word/") {
                return false;
            }
            matches!(
                name,
                "word/document.xml"
                    | "word/footnotes.xml"
                    | "word/endnotes.xml"
                    | "word/comments.xml"
                    | "word/glossary/document.xml"
            ) || name
                .strip_prefix("word/header")
                .or_else(|| name.strip_prefix("word/footer"))
                .is_some_and(has_xml_extension)
        }
        PackageKind::Pptx => [
            "ppt/slides/slide",
            "ppt/notesSlides/notesSlide",
            "ppt/slideMasters/slideMaster",
            "ppt/slideLayouts/slideLayout",
            "ppt/handoutMasters/handoutMaster",
            "ppt/notesMasters/notesMaster",
            "ppt/comments/comment",
        ]
        .iter()
        .any(|prefix| name.starts_with(prefix)),
        PackageKind::Xlsx => {
            name == "xl/sharedStrings.xml"
                || (name.starts_with("xl/worksheets/sheet") && has_xml_extension(name))
        }
    }
}

/// 判断归档部件是否为 XML 文件而不依赖大小写形式。
fn has_xml_extension(name: &str) -> bool {
    Path::new(name)
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("xml"))
}

/// 解析一个 UTF-8 XML 部件，拒绝 DTD 和外部实体而不执行任何内容。
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

/// 在 XML 中定位 w:t 和 a:t 文本节点，同时保留原始标签和属性。
fn text_spans(xml: &str, kind: PackageKind) -> Result<Vec<TextSpan>, DocumentError> {
    validate_xml(xml)?;
    let bytes = xml.as_bytes();
    let mut spans = Vec::new();
    let mut cursor = 0usize;
    while let Some(relative) = xml[cursor..].find('<') {
        let start = cursor + relative;
        let end = tag_end(xml, start)?;
        let raw = &xml[start..end];
        let Some(name) = element_name(raw) else {
            cursor = end;
            continue;
        };
        let is_text = match kind {
            PackageKind::Docx => name == "w:t",
            PackageKind::Pptx => name == "a:t",
            PackageKind::Xlsx => name == "t",
        };
        if !is_text || raw.starts_with("</") || raw.ends_with("/>") {
            cursor = end;
            continue;
        }
        let close_marker = format!("</{name}>");
        let content_start = end;
        let content_end = xml[content_start..]
            .find(&close_marker)
            .map(|offset| content_start + offset)
            .ok_or(DocumentError::InvalidStructure)?;
        if bytes[content_start..content_end].contains(&b'<') {
            return Err(DocumentError::InvalidStructure);
        }
        spans.push(TextSpan {
            content_start,
            content_end,
        });
        cursor = content_end + close_marker.len();
    }
    if bytes.contains(&b'\0') {
        return Err(DocumentError::InvalidStructure);
    }
    Ok(spans)
}

/// 扫描引号安全的 XML 标签边界。
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

/// 提取带命名空间前缀的元素名，拒绝空标签名。
fn element_name(raw: &str) -> Option<&str> {
    let body = raw.strip_prefix('<')?;
    let body = body.strip_prefix('/').unwrap_or(body);
    let body = body.trim_start();
    let end = body
        .find(|character: char| character.is_ascii_whitespace() || matches!(character, '/' | '>'))
        .unwrap_or(body.len());
    (!body[..end].is_empty()).then_some(&body[..end])
}

/// 将 XML 文本节点转为有序文档段，并为每个段保留段落换行提示。
pub(crate) fn inspect(package: &[u8]) -> Result<Vec<DocumentSegment>, DocumentError> {
    inspect_kind(package, PackageKind::Docx)
}

pub(crate) fn inspect_pptx(package: &[u8]) -> Result<Vec<DocumentSegment>, DocumentError> {
    inspect_kind(package, PackageKind::Pptx)
}

pub(crate) fn inspect_xlsx(package: &[u8]) -> Result<Vec<DocumentSegment>, DocumentError> {
    inspect_kind(package, PackageKind::Xlsx)
}

fn inspect_kind(package: &[u8], kind: PackageKind) -> Result<Vec<DocumentSegment>, DocumentError> {
    let names = archive_names(package, kind)?;
    let mut archive =
        ZipArchive::new(Cursor::new(package)).map_err(|_| DocumentError::InvalidStructure)?;
    let mut segments = Vec::new();
    for name in names.iter().filter(|name| translatable_part(name, kind)) {
        let mut file = archive
            .by_name(name)
            .map_err(|_| DocumentError::InvalidStructure)?;
        let mut data = Vec::new();
        file.read_to_end(&mut data)
            .map_err(|_| DocumentError::InvalidStructure)?;
        let xml = std::str::from_utf8(&data).map_err(|_| DocumentError::InvalidStructure)?;
        let spans = text_spans(xml, kind)?;
        for span in spans {
            let raw = &xml[span.content_start..span.content_end];
            let text = unescape(raw)
                .map_err(|_| DocumentError::InvalidStructure)?
                .into_owned();
            let line_ending = if matches!(kind, PackageKind::Xlsx) {
                ""
            } else if xml[span.content_end..]
                .find(kind.paragraph_close())
                .is_some_and(|paragraph_end| {
                    xml[span.content_end..]
                        .find(kind.text_open())
                        .is_none_or(|next_text| paragraph_end < next_text)
                })
            {
                "\n"
            } else {
                ""
            };
            let kind = if text.chars().any(|character| !character.is_whitespace()) {
                DocumentSegmentKind::Prose
            } else {
                DocumentSegmentKind::Verbatim
            };
            segments.push(DocumentSegment {
                index: segments.len(),
                kind,
                source_text: text,
                translated_text: None,
                line_ending: line_ending.to_owned(),
            });
        }
    }
    Ok(segments)
}

/// 返回只用于 Linux 编辑器预览的 DOCX 文本。
pub(crate) fn preview(job: &DocumentJob) -> Result<String, DocumentError> {
    let mut output = String::new();
    for segment in &job.segments {
        output.push_str(segment.output_text()?);
        output.push_str(&segment.line_ending);
    }
    if output.len() > MAX_DOCUMENT_BYTES {
        return Err(DocumentError::OutputTooLarge);
    }
    Ok(output)
}

/// 按原始 XML 结构替换文本节点，并拒绝段顺序或源文本不一致。
fn rewrite_xml(
    xml: &str,
    segments: &[DocumentSegment],
    cursor: &mut usize,
    kind: PackageKind,
) -> Result<Vec<u8>, DocumentError> {
    let spans = text_spans(xml, kind)?;
    let mut output = String::with_capacity(xml.len());
    let mut cursor_bytes = 0usize;
    for span in spans {
        let segment = segments
            .get(*cursor)
            .ok_or(DocumentError::InvalidStructure)?;
        let source = unescape(&xml[span.content_start..span.content_end])
            .map_err(|_| DocumentError::InvalidStructure)?;
        if source.as_ref() != segment.source_text {
            return Err(DocumentError::InvalidStructure);
        }
        output.push_str(&xml[cursor_bytes..span.content_start]);
        match segment.kind {
            DocumentSegmentKind::Verbatim => {
                output.push_str(&xml[span.content_start..span.content_end]);
            }
            DocumentSegmentKind::Prose => {
                let translated = segment
                    .translated_text
                    .as_deref()
                    .ok_or(DocumentError::SegmentIncomplete(segment.index))?;
                output.push_str(&escape(translated));
            }
        }
        cursor_bytes = span.content_end;
        *cursor = (*cursor).saturating_add(1);
    }
    output.push_str(&xml[cursor_bytes..]);
    Ok(output.into_bytes())
}

/// 重建 OOXML ZIP，保留所有未参与翻译的包部件和资源。
pub(crate) fn reconstruct(job: &DocumentJob, package: &[u8]) -> Result<Vec<u8>, DocumentError> {
    reconstruct_kind(job, package, PackageKind::Docx)
}

pub(crate) fn reconstruct_pptx(
    job: &DocumentJob,
    package: &[u8],
) -> Result<Vec<u8>, DocumentError> {
    reconstruct_kind(job, package, PackageKind::Pptx)
}

pub(crate) fn reconstruct_xlsx(
    job: &DocumentJob,
    package: &[u8],
) -> Result<Vec<u8>, DocumentError> {
    reconstruct_kind(job, package, PackageKind::Xlsx)
}

fn reconstruct_kind(
    job: &DocumentJob,
    package: &[u8],
    kind: PackageKind,
) -> Result<Vec<u8>, DocumentError> {
    let names = archive_names(package, kind)?;
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
        if translatable_part(&name, kind) {
            let xml = std::str::from_utf8(&data).map_err(|_| DocumentError::InvalidStructure)?;
            data = rewrite_xml(xml, &job.segments, &mut segment_cursor, kind)?;
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
