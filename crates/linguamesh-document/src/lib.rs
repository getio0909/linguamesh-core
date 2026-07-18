#![doc = "`LinguaMesh` 文本文档检查、分段和重建契约。"]

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// 文本文档的受支持格式。
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DocumentFormat {
    /// 纯 UTF-8 文本。
    Txt,
    /// UTF-8 Markdown 文本。
    Markdown,
    /// UTF-8 `SubRip` 字幕。
    Srt,
    /// UTF-8 `WebVTT` 字幕。
    WebVtt,
}

impl DocumentFormat {
    /// 根据文件名后缀判断格式。
    pub fn from_name(name: &str) -> Result<Self, DocumentError> {
        let extension = name
            .rsplit_once('.')
            .map_or("", |(_, extension)| extension)
            .to_ascii_lowercase();
        match extension.as_str() {
            "txt" => Ok(Self::Txt),
            "md" | "markdown" => Ok(Self::Markdown),
            "srt" => Ok(Self::Srt),
            "vtt" => Ok(Self::WebVtt),
            _ => Err(DocumentError::UnsupportedFormat),
        }
    }
}

/// 文档读取和分段的最大 UTF-8 字节数。
pub const MAX_DOCUMENT_BYTES: usize = 4 * 1024 * 1024;

/// 文档任务在本地数据库中的生命周期状态。
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DocumentJobState {
    /// 已创建但尚未开始翻译。
    Pending,
    /// 至少一个段已开始翻译，任务可以在重启后继续。
    Running,
    /// 用户请求暂停，保留已完成段并等待显式恢复。
    Paused,
    /// 所有可翻译段均已完成。
    Completed,
    /// 用户主动取消，保留快照供查看或重新开始。
    Cancelled,
    /// 任务因可恢复之外的错误停止。
    Failed,
}

impl DocumentJobState {
    /// 返回进程重启后应重新暴露给界面的任务。
    #[must_use]
    pub const fn is_resumable(self) -> bool {
        matches!(self, Self::Pending | Self::Running | Self::Paused)
    }
}

/// 文档段的语义类别。
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DocumentSegmentKind {
    /// 可以交给翻译引擎处理的文本。
    Prose,
    /// 必须原样保留的 Markdown 或字幕结构。
    Verbatim,
}

/// 一个保留换行符的文档段。
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DocumentSegment {
    /// 在文档中的稳定顺序号。
    pub index: usize,
    /// 段的类别。
    pub kind: DocumentSegmentKind,
    /// 不含行尾换行符的源文本。
    pub source_text: String,
    /// 已完成的译文；Verbatim 段不需要设置该字段。
    pub translated_text: Option<String>,
    /// 原始行尾，保持空字符串、LF、CRLF 或 CR。
    pub line_ending: String,
}

impl DocumentSegment {
    /// 返回该段重建时应使用的文本。
    pub fn output_text(&self) -> Result<&str, DocumentError> {
        match self.kind {
            DocumentSegmentKind::Verbatim => Ok(&self.source_text),
            DocumentSegmentKind::Prose => self
                .translated_text
                .as_deref()
                .ok_or(DocumentError::SegmentIncomplete(self.index)),
        }
    }
}

/// 一个可恢复的文本文档任务快照。
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DocumentJob {
    /// 文档格式。
    pub format: DocumentFormat,
    /// 原始文件名，仅用于格式和报告，不是本地路径。
    pub source_name: String,
    /// 按原始顺序排列的文档段。
    pub segments: Vec<DocumentSegment>,
}

impl DocumentJob {
    /// 从受限 UTF-8 内容创建文档任务。
    pub fn from_utf8(
        source_name: impl Into<String>,
        contents: &[u8],
    ) -> Result<Self, DocumentError> {
        if contents.len() > MAX_DOCUMENT_BYTES {
            return Err(DocumentError::TooLarge);
        }
        let source_name = source_name.into();
        let format = DocumentFormat::from_name(&source_name)?;
        let contents = contents.strip_prefix(b"\xef\xbb\xbf").unwrap_or(contents);
        let text = std::str::from_utf8(contents).map_err(|_| DocumentError::InvalidUtf8)?;
        validate_structure(format, text)?;
        Ok(Self::from_text(source_name, format, text))
    }

    /// 从已解码的文本创建文档任务。
    #[must_use]
    pub fn from_text(source_name: impl Into<String>, format: DocumentFormat, text: &str) -> Self {
        let mut in_fenced_code = false;
        let subtitle_kinds = subtitle_line_kinds(format, text);
        let segments = split_lines(text)
            .into_iter()
            .enumerate()
            .map(|(index, (line, line_ending))| {
                let trimmed = line.trim_start();
                let is_fence = matches!(format, DocumentFormat::Markdown)
                    && (trimmed.starts_with("```") || trimmed.starts_with("~~~"));
                let kind = if subtitle_kinds.get(index).copied().unwrap_or(false)
                    || (matches!(format, DocumentFormat::Markdown)
                        && (in_fenced_code || is_fence || line.trim().is_empty()))
                {
                    DocumentSegmentKind::Verbatim
                } else {
                    DocumentSegmentKind::Prose
                };
                if is_fence {
                    in_fenced_code = !in_fenced_code;
                }
                DocumentSegment {
                    index,
                    kind,
                    source_text: line,
                    translated_text: None,
                    line_ending,
                }
            })
            .collect();
        Self {
            format,
            source_name: source_name.into(),
            segments,
        }
    }

    /// 返回未完成的可翻译段数量。
    #[must_use]
    pub fn pending_count(&self) -> usize {
        self.segments
            .iter()
            .filter(|segment| {
                segment.kind == DocumentSegmentKind::Prose && segment.translated_text.is_none()
            })
            .count()
    }

    /// 返回未修改的源文档文本，用于导入预览和源文件保护。
    #[must_use]
    pub fn source_text(&self) -> String {
        let mut output = String::new();
        for segment in &self.segments {
            output.push_str(&segment.source_text);
            output.push_str(&segment.line_ending);
        }
        output
    }

    /// 提交一个段的译文，并拒绝越界或结构段写入。
    pub fn apply_translation(
        &mut self,
        index: usize,
        translated_text: impl Into<String>,
    ) -> Result<(), DocumentError> {
        let segment = self
            .segments
            .get_mut(index)
            .ok_or(DocumentError::UnknownSegment(index))?;
        if segment.kind != DocumentSegmentKind::Prose {
            return Err(DocumentError::VerbatimSegment(index));
        }
        segment.translated_text = Some(translated_text.into());
        Ok(())
    }

    /// 重建完整 UTF-8 文档；未完成的可翻译段会被拒绝。
    pub fn reconstruct(&self) -> Result<String, DocumentError> {
        let mut output = String::new();
        for segment in &self.segments {
            output.push_str(segment.output_text()?);
            output.push_str(&segment.line_ending);
        }
        if output.len() > MAX_DOCUMENT_BYTES {
            return Err(DocumentError::OutputTooLarge);
        }
        validate_structure(self.format, &output)?;
        Ok(output)
    }
}

/// 文档操作的安全、可本地化错误。
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum DocumentError {
    /// 内容超过 4 MiB 上限。
    #[error("The document exceeds the 4 MiB limit.")]
    TooLarge,
    /// 内容不是 UTF-8。
    #[error("The document is not valid UTF-8 text.")]
    InvalidUtf8,
    /// 文件后缀不受支持。
    #[error("The document format is not supported.")]
    UnsupportedFormat,
    /// 字幕结构不完整或包含无效时间轴。
    #[error("The subtitle structure is invalid.")]
    InvalidStructure,
    /// 译文导致输出超过上限。
    #[error("The reconstructed document exceeds the 4 MiB limit.")]
    OutputTooLarge,
    /// 请求了不存在的段。
    #[error("The document segment is not present.")]
    UnknownSegment(usize),
    /// 不允许修改结构段。
    #[error("The document structure segment must remain unchanged.")]
    VerbatimSegment(usize),
    /// 仍有段没有译文。
    #[error("The document segment is not translated.")]
    SegmentIncomplete(usize),
}

fn split_lines(text: &str) -> Vec<(String, String)> {
    let mut lines = Vec::new();
    let mut start = 0;
    for (offset, character) in text.char_indices() {
        if character == '\n' {
            let line = &text[start..offset];
            let (line, line_ending) = line
                .strip_suffix('\r')
                .map_or((line, "\n"), |line| (line, "\r\n"));
            lines.push((line.to_owned(), line_ending.to_owned()));
            start = offset + character.len_utf8();
        }
    }
    if start < text.len() || text.is_empty() {
        lines.push((text[start..].to_owned(), String::new()));
    }
    lines
}

// 校验字幕时间戳的时钟字段和毫秒字段。
fn valid_timestamp_line(line: &str, format: DocumentFormat) -> bool {
    let Some((start, end)) = line.split_once("-->") else {
        return false;
    };
    valid_timestamp(start.trim(), format)
        && valid_timestamp(end.split_whitespace().next().unwrap_or(""), format)
}

// 校验 SRT 或 WebVTT 的单个时间戳。
fn valid_timestamp(value: &str, format: DocumentFormat) -> bool {
    let separator = if matches!(format, DocumentFormat::Srt) {
        ','
    } else {
        '.'
    };
    let Some((clock, milliseconds)) = value.rsplit_once(separator) else {
        return false;
    };
    if milliseconds.len() != 3
        || !milliseconds
            .chars()
            .all(|character| character.is_ascii_digit())
    {
        return false;
    }
    let fields = clock.split(':').collect::<Vec<_>>();
    if !(fields.len() == 2 || fields.len() == 3)
        || fields.iter().any(|field| {
            field.is_empty() || !field.chars().all(|character| character.is_ascii_digit())
        })
    {
        return false;
    }
    let minute = fields
        .get(fields.len() - 2)
        .and_then(|field| field.parse::<u32>().ok());
    let second = fields.last().and_then(|field| field.parse::<u32>().ok());
    minute.is_some_and(|value| value < 60) && second.is_some_and(|value| value < 60)
}

// 返回字幕结构行是否必须原样保留。
fn subtitle_line_kinds(format: DocumentFormat, text: &str) -> Vec<bool> {
    if !matches!(format, DocumentFormat::Srt | DocumentFormat::WebVtt) {
        return Vec::new();
    }
    let lines = split_lines(text);
    let mut kinds = Vec::with_capacity(lines.len());
    let mut cue_start = true;
    let mut expecting_timestamp = false;
    let mut metadata = false;
    for (index, (line, _)) in lines.iter().enumerate() {
        let trimmed = line.trim();
        let is_blank = trimmed.is_empty();
        let is_timestamp = valid_timestamp_line(trimmed, format);
        let is_header =
            matches!(format, DocumentFormat::WebVtt) && index == 0 && trimmed.starts_with("WEBVTT");
        let is_metadata = matches!(format, DocumentFormat::WebVtt)
            && matches!(trimmed, "NOTE" | "STYLE" | "REGION");
        let structural = is_blank
            || is_header
            || metadata
            || is_metadata
            || expecting_timestamp
            || (cue_start && is_timestamp)
            || (matches!(format, DocumentFormat::Srt)
                && cue_start
                && trimmed.chars().all(|c| c.is_ascii_digit())
                && !trimmed.is_empty())
            || (matches!(format, DocumentFormat::WebVtt)
                && cue_start
                && !is_timestamp
                && !is_header);
        kinds.push(structural);
        if is_blank {
            cue_start = true;
            expecting_timestamp = false;
            metadata = false;
        } else if is_metadata {
            metadata = true;
            cue_start = false;
        } else if metadata {
            cue_start = false;
        } else if expecting_timestamp {
            expecting_timestamp = false;
            cue_start = false;
        } else if cue_start && !is_timestamp {
            expecting_timestamp = true;
            cue_start = false;
        } else if is_timestamp {
            cue_start = false;
        }
    }
    kinds
}

// 校验字幕头、cue 顺序、时间轴和每个 cue 的文本。
fn validate_structure(format: DocumentFormat, text: &str) -> Result<(), DocumentError> {
    if !matches!(format, DocumentFormat::Srt | DocumentFormat::WebVtt) {
        return Ok(());
    }
    let lines = split_lines(text);
    if lines.is_empty() {
        return Err(DocumentError::InvalidStructure);
    }
    if matches!(format, DocumentFormat::WebVtt)
        && !lines
            .first()
            .is_some_and(|(line, _)| line.trim_start().starts_with("WEBVTT"))
    {
        return Err(DocumentError::InvalidStructure);
    }
    let mut cue_start = true;
    let mut expecting_timestamp = false;
    let mut cue_count = 0usize;
    let mut cue_has_text = false;
    for (line, _) in lines {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            if expecting_timestamp || (cue_count > 0 && !cue_has_text) {
                return Err(DocumentError::InvalidStructure);
            }
            cue_start = true;
            expecting_timestamp = false;
            cue_has_text = false;
            continue;
        }
        if matches!(format, DocumentFormat::WebVtt)
            && cue_count == 0
            && (trimmed.starts_with("WEBVTT") || matches!(trimmed, "NOTE" | "STYLE" | "REGION"))
        {
            cue_start = false;
            continue;
        }
        if expecting_timestamp {
            if !valid_timestamp_line(trimmed, format) {
                return Err(DocumentError::InvalidStructure);
            }
            cue_count += 1;
            expecting_timestamp = false;
            cue_start = false;
            cue_has_text = false;
        } else if cue_start && valid_timestamp_line(trimmed, format) {
            cue_count += 1;
            cue_start = false;
            cue_has_text = false;
        } else if cue_start {
            let valid_id = matches!(format, DocumentFormat::WebVtt)
                || (matches!(format, DocumentFormat::Srt)
                    && trimmed.chars().all(|character| character.is_ascii_digit()));
            if !valid_id {
                return Err(DocumentError::InvalidStructure);
            }
            expecting_timestamp = true;
        } else {
            cue_has_text = true;
        }
    }
    if expecting_timestamp || cue_count == 0 || (!cue_start && !cue_has_text) {
        return Err(DocumentError::InvalidStructure);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        DocumentError, DocumentFormat, DocumentJob, DocumentSegmentKind, MAX_DOCUMENT_BYTES,
    };

    #[test]
    fn detects_supported_formats_case_insensitively() {
        assert_eq!(
            DocumentFormat::from_name("README.MD"),
            Ok(DocumentFormat::Markdown)
        );
        assert_eq!(
            DocumentFormat::from_name("notes.txt"),
            Ok(DocumentFormat::Txt)
        );
        assert_eq!(
            DocumentFormat::from_name("captions.SRT"),
            Ok(DocumentFormat::Srt)
        );
        assert_eq!(
            DocumentFormat::from_name("captions.vtt"),
            Ok(DocumentFormat::WebVtt)
        );
        assert_eq!(
            DocumentFormat::from_name("notes.docx"),
            Err(DocumentError::UnsupportedFormat)
        );
    }

    #[test]
    fn keeps_bom_and_line_endings_out_of_source_text() {
        let job = DocumentJob::from_utf8("notes.txt", b"\xef\xbb\xbfone\r\ntwo").unwrap();
        assert_eq!(job.segments[0].source_text, "one");
        assert_eq!(job.segments[0].line_ending, "\r\n");
        assert_eq!(job.segments[1].source_text, "two");
        assert_eq!(job.pending_count(), 2);
    }

    #[test]
    fn markdown_code_fences_are_verbatim_and_reconstruct_exactly() {
        let source = "# Heading\n\n```rust\nlet value = 1;\n```\n";
        let mut job = DocumentJob::from_text("readme.md", DocumentFormat::Markdown, source);
        assert_eq!(
            job.segments
                .iter()
                .filter(|segment| segment.kind == DocumentSegmentKind::Verbatim)
                .count(),
            4
        );
        job.apply_translation(0, "# 标题").unwrap();
        assert_eq!(
            job.reconstruct().unwrap(),
            "# 标题\n\n```rust\nlet value = 1;\n```\n"
        );
    }

    #[test]
    fn reconstruction_rejects_pending_prose_and_accepts_completed_segments() {
        let mut job = DocumentJob::from_text("notes.txt", DocumentFormat::Txt, "one\ntwo");
        assert_eq!(job.reconstruct(), Err(DocumentError::SegmentIncomplete(0)));
        job.apply_translation(0, "一").unwrap();
        job.apply_translation(1, "二").unwrap();
        assert_eq!(job.reconstruct().unwrap(), "一\n二");
    }

    #[test]
    fn rejects_oversized_and_invalid_input() {
        let oversized = vec![b'x'; MAX_DOCUMENT_BYTES + 1];
        assert_eq!(
            DocumentJob::from_utf8("notes.txt", &oversized),
            Err(DocumentError::TooLarge)
        );
        assert_eq!(
            DocumentJob::from_utf8("notes.txt", &[0xff]),
            Err(DocumentError::InvalidUtf8)
        );
    }

    #[test]
    fn subtitles_preserve_timing_and_translate_only_cue_text() {
        let source = "1\n00:00:01,000 --> 00:00:02,500\nHello\n\n";
        let mut job = DocumentJob::from_utf8("captions.srt", source.as_bytes()).expect("srt");
        assert_eq!(job.pending_count(), 1);
        assert_eq!(job.segments[0].kind, DocumentSegmentKind::Verbatim);
        assert_eq!(job.segments[1].kind, DocumentSegmentKind::Verbatim);
        job.apply_translation(2, "你好").expect("cue text");
        assert_eq!(
            job.reconstruct().expect("reconstruct"),
            "1\n00:00:01,000 --> 00:00:02,500\n你好\n\n"
        );
        let webvtt = "WEBVTT\n\ncue-1\n00:00.000 --> 00:01.000\nHello\n";
        let mut job = DocumentJob::from_utf8("captions.vtt", webvtt.as_bytes()).expect("vtt");
        let index = job
            .segments
            .iter()
            .position(|segment| segment.kind == DocumentSegmentKind::Prose)
            .expect("cue text");
        job.apply_translation(index, "你好").expect("cue text");
        assert_eq!(
            job.reconstruct().expect("reconstruct"),
            "WEBVTT\n\ncue-1\n00:00.000 --> 00:01.000\n你好\n"
        );
    }

    #[test]
    fn rejects_malformed_subtitle_structure() {
        assert_eq!(
            DocumentJob::from_utf8("captions.srt", b"1\nnot a timestamp\nHello"),
            Err(DocumentError::InvalidStructure)
        );
        assert_eq!(
            DocumentJob::from_utf8("captions.vtt", b"WEBVTT\n\n00:00.000 --> nope\nHello"),
            Err(DocumentError::InvalidStructure)
        );
    }
}
