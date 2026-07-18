#![doc = "`LinguaMesh` 文本文档检查、分段和重建契约。"]

use std::borrow::Cow;

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
    /// UTF-8 逗号或兼容分隔符表格。
    Csv,
    /// UTF-8 HTML 文档。
    Html,
    /// UTF-8 JSON 文档。
    Json,
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
            "csv" => Ok(Self::Csv),
            "html" | "htm" => Ok(Self::Html),
            "json" => Ok(Self::Json),
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
        Self::from_utf8_with_json_paths(source_name, contents, None, None)
    }

    /// 从受限 UTF-8 内容创建文档任务，并可为 CSV 指定要翻译的列。
    ///
    /// `None` 表示按表头和字段内容选择可翻译的文本列；非 CSV 格式忽略该选项。
    pub fn from_utf8_with_csv_columns(
        source_name: impl Into<String>,
        contents: &[u8],
        selected_columns: Option<&[usize]>,
    ) -> Result<Self, DocumentError> {
        if contents.len() > MAX_DOCUMENT_BYTES {
            return Err(DocumentError::TooLarge);
        }
        let source_name = source_name.into();
        let format = DocumentFormat::from_name(&source_name)?;
        let contents = contents.strip_prefix(b"\xef\xbb\xbf").unwrap_or(contents);
        let text = std::str::from_utf8(contents).map_err(|_| DocumentError::InvalidUtf8)?;
        validate_structure(format, text)?;
        if matches!(format, DocumentFormat::Csv) {
            return from_csv_text(source_name, text, selected_columns);
        }
        Ok(Self::from_text(source_name, format, text))
    }

    /// 从受限 UTF-8 内容创建 JSON 任务，并按 JSON Pointer 选择字符串值。
    ///
    /// 默认翻译所有字符串值；对象键、数字、布尔值和 `null` 始终原样保留。
    /// `include_paths` 和 `exclude_paths` 使用 RFC 6901 JSON Pointer 的完整路径，
    /// 排除规则优先于包含规则。非 JSON 格式忽略这些选项。
    pub fn from_utf8_with_json_paths(
        source_name: impl Into<String>,
        contents: &[u8],
        include_paths: Option<&[String]>,
        exclude_paths: Option<&[String]>,
    ) -> Result<Self, DocumentError> {
        if contents.len() > MAX_DOCUMENT_BYTES {
            return Err(DocumentError::TooLarge);
        }
        let source_name = source_name.into();
        let format = DocumentFormat::from_name(&source_name)?;
        let contents = contents.strip_prefix(b"\xef\xbb\xbf").unwrap_or(contents);
        let text = std::str::from_utf8(contents).map_err(|_| DocumentError::InvalidUtf8)?;
        validate_structure(format, text)?;
        if matches!(format, DocumentFormat::Json) {
            return from_json_text(source_name, text, include_paths, exclude_paths);
        }
        if matches!(format, DocumentFormat::Csv) {
            return from_csv_text(source_name, text, None);
        }
        Ok(Self::from_text(source_name, format, text))
    }

    /// 从已解码的文本创建文档任务。
    #[must_use]
    pub fn from_text(source_name: impl Into<String>, format: DocumentFormat, text: &str) -> Self {
        let source_name = source_name.into();
        if matches!(format, DocumentFormat::Csv)
            && let Ok(job) = from_csv_text(source_name.clone(), text, None)
        {
            return job;
        }
        if matches!(format, DocumentFormat::Json)
            && let Ok(job) = from_json_text(source_name.clone(), text, None, None)
        {
            return job;
        }
        if matches!(format, DocumentFormat::Html)
            && let Ok(job) = from_html_text(source_name.clone(), text)
        {
            return job;
        }
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
            source_name,
            segments,
        }
    }

    /// 返回一个可翻译段实际提交给提供方的文本。
    pub fn translation_source_text(&self, index: usize) -> Result<Cow<'_, str>, DocumentError> {
        let segment = self
            .segments
            .get(index)
            .ok_or(DocumentError::UnknownSegment(index))?;
        if segment.kind != DocumentSegmentKind::Prose {
            return Ok(Cow::Borrowed(segment.source_text.as_str()));
        }
        match self.format {
            DocumentFormat::Csv => decode_csv_field(
                &segment.source_text,
                detect_csv_delimiter(&self.source_text()),
            )
            .map(Cow::Owned),
            DocumentFormat::Json => decode_json_string(&segment.source_text).map(Cow::Owned),
            _ => Ok(Cow::Borrowed(segment.source_text.as_str())),
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
        let csv_delimiter = matches!(self.format, DocumentFormat::Csv)
            .then(|| detect_csv_delimiter(&self.source_text()));
        let segment = self
            .segments
            .get_mut(index)
            .ok_or(DocumentError::UnknownSegment(index))?;
        if segment.kind != DocumentSegmentKind::Prose {
            return Err(DocumentError::VerbatimSegment(index));
        }
        let translated_text = translated_text.into();
        segment.translated_text = Some(match csv_delimiter {
            Some(delimiter) => encode_csv_field(&segment.source_text, &translated_text, delimiter),
            None if matches!(self.format, DocumentFormat::Json) => {
                encode_json_string(&translated_text)
            }
            None if matches!(self.format, DocumentFormat::Html) => {
                encode_html_text(&translated_text)
            }
            None => translated_text,
        });
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
        let csv_delimiter = matches!(self.format, DocumentFormat::Csv)
            .then(|| detect_csv_delimiter(&self.source_text()));
        validate_structure_with_delimiter(self.format, &output, csv_delimiter)?;
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
    /// 文档结构不完整或包含无效字段。
    #[error("The document structure is invalid.")]
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

const MAX_HTML_SEGMENTS: usize = 10_000;
const HTML_VOID_TAGS: [&str; 14] = [
    "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param", "source",
    "track", "wbr",
];
const HTML_RAW_TEXT_TAGS: [&str; 2] = ["script", "style"];

struct HtmlToken {
    start: usize,
    end: usize,
    translatable: bool,
}

// 在 HTML 标签边界内扫描引号，拒绝未闭合的结构而不执行任何实体或脚本。
fn html_tag_end(text: &str, start: usize) -> Result<usize, DocumentError> {
    let bytes = text.as_bytes();
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

fn html_find_case_insensitive(text: &str, pattern: &str, start: usize) -> Option<usize> {
    let pattern = pattern.as_bytes();
    let bytes = text.as_bytes();
    if pattern.is_empty() || start >= bytes.len() || pattern.len() > bytes.len() - start {
        return None;
    }
    (start..=bytes.len() - pattern.len()).find(|offset| {
        bytes[*offset..*offset + pattern.len()]
            .iter()
            .zip(pattern)
            .all(|(left, right)| left.eq_ignore_ascii_case(right))
    })
}

fn html_tag_name(raw: &str) -> Result<(Option<String>, bool, bool), DocumentError> {
    let mut body = raw[1..raw.len() - 1].trim();
    if body.is_empty() {
        return Err(DocumentError::InvalidStructure);
    }
    let closing = body.starts_with('/');
    if closing {
        body = body[1..].trim_start();
    }
    if body.starts_with('!') || body.starts_with('?') {
        return Ok((None, closing, false));
    }
    let self_closing = !closing && body.trim_end().ends_with('/');
    let name_end = body
        .find(|character: char| character.is_ascii_whitespace() || matches!(character, '/' | '>'))
        .unwrap_or(body.len());
    let name = &body[..name_end];
    if name.is_empty()
        || !name
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | ':'))
    {
        return Err(DocumentError::InvalidStructure);
    }
    Ok((Some(name.to_ascii_lowercase()), closing, self_closing))
}

// 通过标签栈验证 HTML 结构，并将可见文本节点与结构字节分开。
fn parse_html_tokens(text: &str) -> Result<Vec<HtmlToken>, DocumentError> {
    if text.is_empty() {
        return Err(DocumentError::InvalidStructure);
    }
    let mut tokens = Vec::new();
    let mut stack = Vec::<String>::new();
    let mut cursor = 0usize;
    while cursor < text.len() {
        if let Some(raw_tag) = stack
            .last()
            .filter(|tag| HTML_RAW_TEXT_TAGS.contains(&tag.as_str()))
            .cloned()
        {
            let close = html_find_case_insensitive(text, &format!("</{raw_tag}"), cursor)
                .ok_or(DocumentError::InvalidStructure)?;
            if close > cursor {
                tokens.push(HtmlToken {
                    start: cursor,
                    end: close,
                    translatable: false,
                });
                cursor = close;
                continue;
            }
        }
        if text.as_bytes().get(cursor) == Some(&b'<') {
            if text[cursor..].starts_with("<!--") {
                let end = text[cursor + 4..]
                    .find("-->")
                    .map(|offset| cursor + 4 + offset + 3)
                    .ok_or(DocumentError::InvalidStructure)?;
                tokens.push(HtmlToken {
                    start: cursor,
                    end,
                    translatable: false,
                });
                cursor = end;
                continue;
            }
            let end = html_tag_end(text, cursor)?;
            let raw = &text[cursor..end];
            let (name, closing, self_closing) = html_tag_name(raw)?;
            if let Some(name) = name {
                if closing {
                    if stack.pop().as_deref() != Some(name.as_str()) {
                        return Err(DocumentError::InvalidStructure);
                    }
                } else if !self_closing && !HTML_VOID_TAGS.contains(&name.as_str()) {
                    stack.push(name);
                }
            }
            tokens.push(HtmlToken {
                start: cursor,
                end,
                translatable: false,
            });
            cursor = end;
            continue;
        }
        let end = text[cursor..]
            .find('<')
            .map_or(text.len(), |offset| cursor + offset);
        let content = &text[cursor..end];
        let inside_raw_text = stack
            .iter()
            .any(|tag| HTML_RAW_TEXT_TAGS.contains(&tag.as_str()));
        tokens.push(HtmlToken {
            start: cursor,
            end,
            translatable: !inside_raw_text
                && content.chars().any(|character| !character.is_whitespace()),
        });
        cursor = end;
        if tokens.len() > MAX_HTML_SEGMENTS {
            return Err(DocumentError::InvalidStructure);
        }
    }
    if !stack.is_empty() {
        return Err(DocumentError::InvalidStructure);
    }
    Ok(tokens)
}

fn encode_html_text(translated: &str) -> String {
    translated
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

// 将 HTML token 转成保留标签、脚本、样式和空白的文档段。
fn from_html_text(source_name: String, text: &str) -> Result<DocumentJob, DocumentError> {
    let tokens = parse_html_tokens(text)?;
    let segments = tokens
        .into_iter()
        .enumerate()
        .map(|(index, token)| DocumentSegment {
            index,
            kind: if token.translatable {
                DocumentSegmentKind::Prose
            } else {
                DocumentSegmentKind::Verbatim
            },
            source_text: text[token.start..token.end].to_owned(),
            translated_text: None,
            line_ending: String::new(),
        })
        .collect::<Vec<_>>();
    Ok(DocumentJob {
        format: DocumentFormat::Html,
        source_name,
        segments,
    })
}

const MAX_JSON_TOKENS: usize = 5_000;
const MAX_JSON_SEGMENTS: usize = 10_000;

#[derive(Clone, Debug, Eq, PartialEq)]
enum JsonPathSegment {
    Key(String),
    Index(usize),
}

struct JsonStringToken {
    start: usize,
    end: usize,
    path: Vec<JsonPathSegment>,
    is_key: bool,
}

struct JsonParser<'a> {
    text: &'a str,
    bytes: &'a [u8],
    index: usize,
    tokens: Vec<JsonStringToken>,
}

impl<'a> JsonParser<'a> {
    fn new(text: &'a str) -> Self {
        Self {
            text,
            bytes: text.as_bytes(),
            index: 0,
            tokens: Vec::new(),
        }
    }

    fn parse(mut self) -> Result<Vec<JsonStringToken>, DocumentError> {
        self.skip_whitespace();
        self.parse_value(&[])?;
        self.skip_whitespace();
        if self.index != self.bytes.len() {
            return Err(DocumentError::InvalidStructure);
        }
        Ok(self.tokens)
    }

    fn parse_value(&mut self, path: &[JsonPathSegment]) -> Result<(), DocumentError> {
        self.skip_whitespace();
        let Some(byte) = self.bytes.get(self.index).copied() else {
            return Err(DocumentError::InvalidStructure);
        };
        match byte {
            b'{' => self.parse_object(path),
            b'[' => self.parse_array(path),
            b'"' => {
                self.parse_string(path, false)?;
                Ok(())
            }
            b't' | b'f' | b'n' | b'-' | b'0'..=b'9' => self.parse_primitive(),
            _ => Err(DocumentError::InvalidStructure),
        }
    }

    fn parse_object(&mut self, path: &[JsonPathSegment]) -> Result<(), DocumentError> {
        self.expect_byte(b'{')?;
        self.skip_whitespace();
        if self.consume_byte(b'}') {
            return Ok(());
        }
        loop {
            self.skip_whitespace();
            if self.bytes.get(self.index) != Some(&b'"') {
                return Err(DocumentError::InvalidStructure);
            }
            let (start, end) = self.parse_string(path, true)?;
            let key = serde_json::from_str::<String>(&self.text[start..end])
                .map_err(|_| DocumentError::InvalidStructure)?;
            self.skip_whitespace();
            self.expect_byte(b':')?;
            let mut child_path = path.to_vec();
            child_path.push(JsonPathSegment::Key(key));
            self.parse_value(&child_path)?;
            self.skip_whitespace();
            if self.consume_byte(b'}') {
                return Ok(());
            }
            self.expect_byte(b',')?;
        }
    }

    fn parse_array(&mut self, path: &[JsonPathSegment]) -> Result<(), DocumentError> {
        self.expect_byte(b'[')?;
        self.skip_whitespace();
        if self.consume_byte(b']') {
            return Ok(());
        }
        let mut item_index = 0usize;
        loop {
            let mut child_path = path.to_vec();
            child_path.push(JsonPathSegment::Index(item_index));
            self.parse_value(&child_path)?;
            item_index = item_index
                .checked_add(1)
                .ok_or(DocumentError::InvalidStructure)?;
            self.skip_whitespace();
            if self.consume_byte(b']') {
                return Ok(());
            }
            self.expect_byte(b',')?;
        }
    }

    fn parse_string(
        &mut self,
        path: &[JsonPathSegment],
        is_key: bool,
    ) -> Result<(usize, usize), DocumentError> {
        let start = self.index;
        self.expect_byte(b'"')?;
        loop {
            let Some(byte) = self.bytes.get(self.index).copied() else {
                return Err(DocumentError::InvalidStructure);
            };
            match byte {
                b'"' => {
                    self.index += 1;
                    if self.tokens.len() >= MAX_JSON_TOKENS {
                        return Err(DocumentError::InvalidStructure);
                    }
                    let end = self.index;
                    serde_json::from_str::<String>(&self.text[start..end])
                        .map_err(|_| DocumentError::InvalidStructure)?;
                    self.tokens.push(JsonStringToken {
                        start,
                        end,
                        path: path.to_vec(),
                        is_key,
                    });
                    return Ok((start, end));
                }
                b'\\' => {
                    self.index = self
                        .index
                        .checked_add(2)
                        .ok_or(DocumentError::InvalidStructure)?;
                }
                byte if byte < 0x20 => return Err(DocumentError::InvalidStructure),
                _ => self.index += 1,
            }
        }
    }

    fn parse_primitive(&mut self) -> Result<(), DocumentError> {
        let start = self.index;
        while let Some(byte) = self.bytes.get(self.index).copied() {
            if matches!(byte, b',' | b']' | b'}' | b' ' | b'\n' | b'\r' | b'\t') {
                break;
            }
            self.index += 1;
        }
        if start == self.index
            || serde_json::from_str::<serde_json::Value>(&self.text[start..self.index]).is_err()
        {
            return Err(DocumentError::InvalidStructure);
        }
        Ok(())
    }

    fn skip_whitespace(&mut self) {
        while self
            .bytes
            .get(self.index)
            .is_some_and(|byte| matches!(byte, b' ' | b'\n' | b'\r' | b'\t'))
        {
            self.index += 1;
        }
    }

    fn consume_byte(&mut self, expected: u8) -> bool {
        if self.bytes.get(self.index) == Some(&expected) {
            self.index += 1;
            true
        } else {
            false
        }
    }

    fn expect_byte(&mut self, expected: u8) -> Result<(), DocumentError> {
        self.consume_byte(expected)
            .then_some(())
            .ok_or(DocumentError::InvalidStructure)
    }
}

fn parse_json_tokens(text: &str) -> Result<Vec<JsonStringToken>, DocumentError> {
    JsonParser::new(text).parse()
}

fn json_pointer(path: &[JsonPathSegment]) -> String {
    let mut pointer = String::new();
    for segment in path {
        pointer.push('/');
        match segment {
            JsonPathSegment::Key(key) => {
                pointer.push_str(&key.replace('~', "~0").replace('/', "~1"));
            }
            JsonPathSegment::Index(index) => pointer.push_str(&index.to_string()),
        }
    }
    pointer
}

fn json_path_selected(
    path: &[JsonPathSegment],
    include_paths: Option<&[String]>,
    exclude_paths: Option<&[String]>,
) -> bool {
    let pointer = json_pointer(path);
    if exclude_paths.is_some_and(|paths| paths.iter().any(|candidate| candidate == &pointer)) {
        return false;
    }
    include_paths.is_none_or(|paths| paths.iter().any(|candidate| candidate == &pointer))
}

fn decode_json_string(raw: &str) -> Result<String, DocumentError> {
    serde_json::from_str(raw).map_err(|_| DocumentError::InvalidStructure)
}

fn encode_json_string(translated: &str) -> String {
    serde_json::to_string(translated).unwrap_or_else(|_| "\"\"".to_owned())
}

// 将 JSON 字符串值切成可翻译段，同时保留所有非字符串字节和原始转义。
fn from_json_text(
    source_name: String,
    text: &str,
    include_paths: Option<&[String]>,
    exclude_paths: Option<&[String]>,
) -> Result<DocumentJob, DocumentError> {
    let tokens = parse_json_tokens(text)?;
    let mut segments = Vec::with_capacity(tokens.len().saturating_mul(2).saturating_add(1));
    let mut cursor = 0usize;
    for token in tokens {
        if cursor < token.start {
            segments.push(DocumentSegment {
                index: segments.len(),
                kind: DocumentSegmentKind::Verbatim,
                source_text: text[cursor..token.start].to_owned(),
                translated_text: None,
                line_ending: String::new(),
            });
        }
        let kind = if !token.is_key && json_path_selected(&token.path, include_paths, exclude_paths)
        {
            DocumentSegmentKind::Prose
        } else {
            DocumentSegmentKind::Verbatim
        };
        segments.push(DocumentSegment {
            index: segments.len(),
            kind,
            source_text: text[token.start..token.end].to_owned(),
            translated_text: None,
            line_ending: String::new(),
        });
        cursor = token.end;
        if segments.len() > MAX_JSON_SEGMENTS {
            return Err(DocumentError::InvalidStructure);
        }
    }
    if cursor < text.len() {
        segments.push(DocumentSegment {
            index: segments.len(),
            kind: DocumentSegmentKind::Verbatim,
            source_text: text[cursor..].to_owned(),
            translated_text: None,
            line_ending: String::new(),
        });
    }
    Ok(DocumentJob {
        format: DocumentFormat::Json,
        source_name,
        segments,
    })
}

const CSV_DELIMITER_CANDIDATES: [u8; 4] = [b',', b';', b'\t', b'|'];
const MAX_CSV_RECORDS: usize = 10_000;
const MAX_CSV_FIELDS: usize = 1_024;
const MAX_CSV_SEGMENTS: usize = 10_000;

struct CsvRecord {
    fields: Vec<String>,
    line_ending: String,
}

// 依据首条记录中未加引号的分隔符选择稳定的 CSV 分隔符。
fn detect_csv_delimiter(text: &str) -> char {
    let mut counts = [0usize; CSV_DELIMITER_CANDIDATES.len()];
    let bytes = text.as_bytes();
    let mut in_quotes = false;
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'"' if in_quotes && bytes.get(index + 1) == Some(&b'"') => index += 2,
            b'"' => {
                in_quotes = !in_quotes;
                index += 1;
            }
            b'\r' | b'\n' if !in_quotes => break,
            byte if !in_quotes => {
                if let Some(position) = CSV_DELIMITER_CANDIDATES
                    .iter()
                    .position(|candidate| *candidate == byte)
                {
                    counts[position] += 1;
                }
                index += 1;
            }
            _ => index += 1,
        }
    }
    let position = counts
        .iter()
        .enumerate()
        .max_by_key(|(position, count)| (**count, usize::MAX - *position))
        .map_or(0, |(position, _)| position);
    char::from(CSV_DELIMITER_CANDIDATES[position])
}

// 解析带引号、转义引号和跨行字段的受限 CSV 记录。
fn parse_csv_records(text: &str, delimiter: char) -> Result<Vec<CsvRecord>, DocumentError> {
    if text.is_empty() {
        return Err(DocumentError::InvalidStructure);
    }
    let delimiter = u8::try_from(delimiter).map_err(|_| DocumentError::InvalidStructure)?;
    let bytes = text.as_bytes();
    let mut records = Vec::new();
    let mut fields = Vec::new();
    let mut field_start = 0usize;
    let mut in_quotes = false;
    let mut after_quote = false;
    let mut index = 0usize;
    while index < bytes.len() {
        match bytes[index] {
            b'"' if in_quotes && bytes.get(index + 1) == Some(&b'"') => index += 2,
            b'"' if in_quotes => {
                in_quotes = false;
                after_quote = true;
                index += 1;
            }
            b'"' if index == field_start => {
                in_quotes = true;
                index += 1;
            }
            b'"' => return Err(DocumentError::InvalidStructure),
            byte if byte == delimiter && !in_quotes => {
                if after_quote || !fields.is_empty() || field_start < index {
                    fields.push(text[field_start..index].to_owned());
                } else {
                    fields.push(String::new());
                }
                field_start = index + 1;
                after_quote = false;
                index += 1;
            }
            b'\r' | b'\n' if !in_quotes => {
                if after_quote || !fields.is_empty() || field_start < index {
                    fields.push(text[field_start..index].to_owned());
                } else {
                    fields.push(String::new());
                }
                let line_ending = if bytes[index] == b'\r' && bytes.get(index + 1) == Some(&b'\n') {
                    index += 2;
                    "\r\n"
                } else if bytes[index] == b'\r' {
                    index += 1;
                    "\r"
                } else {
                    index += 1;
                    "\n"
                };
                if fields.len() > MAX_CSV_FIELDS || records.len() >= MAX_CSV_RECORDS {
                    return Err(DocumentError::InvalidStructure);
                }
                records.push(CsvRecord {
                    fields,
                    line_ending: line_ending.to_owned(),
                });
                fields = Vec::new();
                field_start = index;
                after_quote = false;
            }
            _ if after_quote => return Err(DocumentError::InvalidStructure),
            _ => index += 1,
        }
    }
    if in_quotes || after_quote && field_start > bytes.len() {
        return Err(DocumentError::InvalidStructure);
    }
    if !fields.is_empty() || field_start < bytes.len() || records.is_empty() {
        fields.push(text[field_start..].to_owned());
        if fields.len() > MAX_CSV_FIELDS || records.len() >= MAX_CSV_RECORDS {
            return Err(DocumentError::InvalidStructure);
        }
        records.push(CsvRecord {
            fields,
            line_ending: String::new(),
        });
    }
    Ok(records)
}

// 解码 CSV 字段供翻译器使用，同时拒绝不完整的引号结构。
fn decode_csv_field(raw: &str, delimiter: char) -> Result<String, DocumentError> {
    if raw.starts_with('"') {
        if !raw.ends_with('"') || raw.len() < 2 {
            return Err(DocumentError::InvalidStructure);
        }
        let inner = &raw[1..raw.len() - 1];
        let mut decoded = String::with_capacity(inner.len());
        let mut characters = inner.chars().peekable();
        while let Some(character) = characters.next() {
            if character == '"' {
                if characters.next() != Some('"') {
                    return Err(DocumentError::InvalidStructure);
                }
                decoded.push('"');
            } else {
                decoded.push(character);
            }
        }
        Ok(decoded)
    } else if raw.contains('"') || raw.contains(delimiter) {
        Err(DocumentError::InvalidStructure)
    } else {
        Ok(raw.to_owned())
    }
}

// 按源字段的引号风格编码译文，并为新增结构字符补上必要的引号。
fn encode_csv_field(raw: &str, translated: &str, delimiter: char) -> String {
    let quoted = raw.starts_with('"')
        || translated.contains(delimiter)
        || translated.contains('"')
        || translated.contains('\r')
        || translated.contains('\n');
    if quoted {
        format!("\"{}\"", translated.replace('"', "\"\""))
    } else {
        translated.to_owned()
    }
}

// 将 CSV 记录转换为保留分隔符、引号和行尾的文档段。
fn from_csv_text(
    source_name: String,
    text: &str,
    selected_columns: Option<&[usize]>,
) -> Result<DocumentJob, DocumentError> {
    let delimiter = detect_csv_delimiter(text);
    let records = parse_csv_records(text, delimiter)?;
    let max_columns = records
        .iter()
        .map(|record| record.fields.len())
        .max()
        .unwrap_or(0);
    let selected = match selected_columns {
        Some(columns) => {
            let mut sorted = columns.to_vec();
            sorted.sort_unstable();
            if sorted.windows(2).any(|window| window[0] == window[1])
                || sorted.iter().any(|column| *column >= max_columns)
            {
                return Err(DocumentError::InvalidStructure);
            }
            sorted
        }
        None => default_csv_columns(&records, delimiter),
    };
    let mut segments = Vec::new();
    for record in records {
        let field_count = record.fields.len();
        for (column, field) in record.fields.into_iter().enumerate() {
            let kind = if selected.binary_search(&column).is_ok() {
                DocumentSegmentKind::Prose
            } else {
                DocumentSegmentKind::Verbatim
            };
            segments.push(DocumentSegment {
                index: segments.len(),
                kind,
                source_text: field,
                translated_text: None,
                line_ending: String::new(),
            });
            if column + 1 < field_count {
                segments.push(DocumentSegment {
                    index: segments.len(),
                    kind: DocumentSegmentKind::Verbatim,
                    source_text: delimiter.to_string(),
                    translated_text: None,
                    line_ending: String::new(),
                });
            }
        }
        if let Some(segment) = segments.last_mut() {
            segment.line_ending = record.line_ending;
        }
        if segments.len() > MAX_CSV_SEGMENTS {
            return Err(DocumentError::InvalidStructure);
        }
    }
    Ok(DocumentJob {
        format: DocumentFormat::Csv,
        source_name,
        segments,
    })
}

// 默认跳过标识符和纯数字列，避免在没有列选择器的宿主中误译结构字段。
fn default_csv_columns(records: &[CsvRecord], delimiter: char) -> Vec<usize> {
    let max_columns = records
        .iter()
        .map(|record| record.fields.len())
        .max()
        .unwrap_or(0);
    (0..max_columns)
        .filter(|column| {
            let header = records
                .first()
                .and_then(|record| record.fields.get(*column))
                .and_then(|field| decode_csv_field(field, delimiter).ok())
                .unwrap_or_default();
            if looks_like_identifier_header(&header) {
                return false;
            }
            let mut has_value = false;
            let all_numeric = records.iter().skip(1).all(|record| {
                let Some(field) = record.fields.get(*column) else {
                    return true;
                };
                let value = decode_csv_field(field, delimiter).unwrap_or_default();
                if value.trim().is_empty() {
                    return true;
                }
                has_value = true;
                value.trim().parse::<f64>().is_ok()
            });
            !has_value || !all_numeric
        })
        .collect()
}

// 识别常见的 CSV 标识符表头。
fn looks_like_identifier_header(header: &str) -> bool {
    let normalized = header.trim().to_ascii_lowercase();
    matches!(
        normalized.as_str(),
        "id" | "key" | "code" | "uuid" | "identifier" | "number" | "no"
    ) || normalized.ends_with("_id")
        || normalized.ends_with("-id")
        || normalized.ends_with("_key")
        || normalized.ends_with("-key")
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
    validate_structure_with_delimiter(format, text, None)
}

// 校验文档结构；重建 CSV 时可传入源文件已选择的分隔符。
fn validate_structure_with_delimiter(
    format: DocumentFormat,
    text: &str,
    delimiter: Option<char>,
) -> Result<(), DocumentError> {
    if matches!(format, DocumentFormat::Html) {
        parse_html_tokens(text)?;
        return Ok(());
    }
    if matches!(format, DocumentFormat::Json) {
        parse_json_tokens(text)?;
        return Ok(());
    }
    if matches!(format, DocumentFormat::Csv) {
        parse_csv_records(
            text,
            delimiter.unwrap_or_else(|| detect_csv_delimiter(text)),
        )?;
        return Ok(());
    }
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
    let mut metadata = false;
    for (line, _) in lines {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            if metadata {
                metadata = false;
                cue_start = true;
                expecting_timestamp = false;
                cue_has_text = false;
                continue;
            }
            if expecting_timestamp || (cue_count > 0 && !cue_has_text) {
                return Err(DocumentError::InvalidStructure);
            }
            cue_start = true;
            expecting_timestamp = false;
            cue_has_text = false;
            continue;
        }
        if matches!(format, DocumentFormat::WebVtt)
            && cue_start
            && matches!(trimmed, "NOTE" | "STYLE" | "REGION")
        {
            metadata = true;
            cue_start = false;
            continue;
        }
        if metadata {
            continue;
        }
        if matches!(format, DocumentFormat::WebVtt)
            && cue_count == 0
            && trimmed.starts_with("WEBVTT")
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
    if expecting_timestamp || cue_count == 0 || (!metadata && !cue_start && !cue_has_text) {
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
            DocumentFormat::from_name("table.CSV"),
            Ok(DocumentFormat::Csv)
        );
        assert_eq!(
            DocumentFormat::from_name("payload.JSON"),
            Ok(DocumentFormat::Json)
        );
        assert_eq!(
            DocumentFormat::from_name("page.HTML"),
            Ok(DocumentFormat::Html)
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
        let with_note = "WEBVTT\n\n00:00.000 --> 00:01.000\nOne\n\nNOTE\nbetween cues\n\n00:02.000 --> 00:03.000\nTwo\n";
        let mut job = DocumentJob::from_utf8("captions.vtt", with_note.as_bytes()).expect("note");
        let prose = job
            .segments
            .iter()
            .enumerate()
            .filter_map(|(index, segment)| {
                (segment.kind == DocumentSegmentKind::Prose).then_some(index)
            })
            .collect::<Vec<_>>();
        assert_eq!(prose.len(), 2);
        job.apply_translation(prose[0], "一").expect("first cue");
        job.apply_translation(prose[1], "二").expect("second cue");
        assert_eq!(
            job.reconstruct().expect("note reconstruct"),
            with_note.replace("One", "一").replace("Two", "二")
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

    #[test]
    fn csv_selected_columns_decode_and_reconstruct_with_original_shape() {
        let source = "id,name,notes\r\n1,Alice,\"Hello, 世界\"\n2,Bob\n";
        let mut job =
            DocumentJob::from_utf8_with_csv_columns("people.csv", source.as_bytes(), Some(&[1]))
                .expect("csv");
        let prose = job
            .segments
            .iter()
            .enumerate()
            .filter_map(|(index, segment)| {
                (segment.kind == DocumentSegmentKind::Prose).then_some(index)
            })
            .collect::<Vec<_>>();
        assert_eq!(prose.len(), 3);
        assert_eq!(job.translation_source_text(prose[0]).unwrap(), "name");
        assert_eq!(job.translation_source_text(prose[1]).unwrap(), "Alice");
        let notes = job
            .segments
            .iter()
            .position(|segment| segment.source_text == "\"Hello, 世界\"")
            .expect("quoted notes field");
        assert_eq!(
            job.translation_source_text(notes).unwrap(),
            "\"Hello, 世界\""
        );
        for (index, translated) in prose.into_iter().zip(["姓名", "爱丽丝", "鲍勃"]) {
            job.apply_translation(index, translated).unwrap();
        }
        assert_eq!(
            job.reconstruct().unwrap(),
            "id,姓名,notes\r\n1,爱丽丝,\"Hello, 世界\"\n2,鲍勃\n"
        );
        assert_eq!(
            job.segments
                .iter()
                .filter(|segment| segment.kind == DocumentSegmentKind::Verbatim)
                .map(|segment| segment.source_text.as_str())
                .collect::<Vec<_>>(),
            vec![
                "id",
                ",",
                ",",
                "notes",
                "1",
                ",",
                ",",
                "\"Hello, 世界\"",
                "2",
                ",",
            ]
        );
    }

    #[test]
    fn csv_preserves_escaped_quotes_when_translating_quoted_fields() {
        let source = "id,comment\n1,\"你好, \"\"世界\"\"\"\n";
        let mut job =
            DocumentJob::from_utf8_with_csv_columns("comments.csv", source.as_bytes(), Some(&[1]))
                .expect("csv");
        let comment = job
            .segments
            .iter()
            .enumerate()
            .find(|(_, segment)| segment.source_text.starts_with("\"你好"))
            .map(|(index, _)| index)
            .expect("comment");
        let header = job
            .segments
            .iter()
            .enumerate()
            .find(|(_, segment)| segment.source_text == "comment")
            .map(|(index, _)| index)
            .expect("header");
        assert_eq!(
            job.translation_source_text(comment).unwrap(),
            "你好, \"世界\""
        );
        job.apply_translation(header, "comment").unwrap();
        job.apply_translation(comment, "译文, \"世界\"").unwrap();
        assert_eq!(
            job.reconstruct().unwrap(),
            "id,comment\n1,\"译文, \"\"世界\"\"\"\n"
        );
    }

    #[test]
    fn csv_detects_semicolon_delimiters_without_rewriting_rows() {
        let source = "id;value\r\n1;one\r\n";
        let mut job =
            DocumentJob::from_utf8_with_csv_columns("values.csv", source.as_bytes(), Some(&[1]))
                .expect("semicolon csv");
        let prose = job
            .segments
            .iter()
            .enumerate()
            .filter_map(|(index, segment)| {
                (segment.kind == DocumentSegmentKind::Prose).then_some(index)
            })
            .collect::<Vec<_>>();
        for index in prose {
            let source_text = job.translation_source_text(index).unwrap().into_owned();
            job.apply_translation(index, format!("{source_text}-translated"))
                .unwrap();
        }
        assert_eq!(
            job.reconstruct().unwrap(),
            "id;value-translated\r\n1;one-translated\r\n"
        );
    }

    #[test]
    fn csv_default_selection_skips_identifier_and_numeric_columns() {
        let job = DocumentJob::from_utf8("people.csv", b"id,name,amount\n1,Alice,42\n2,Bob,7\n")
            .expect("csv");
        let prose = job
            .segments
            .iter()
            .filter(|segment| segment.kind == DocumentSegmentKind::Prose)
            .map(|segment| segment.source_text.as_str())
            .collect::<Vec<_>>();
        assert_eq!(prose, vec!["name", "Alice", "Bob"]);
    }

    #[test]
    fn rejects_malformed_csv_structure() {
        assert_eq!(
            DocumentJob::from_utf8("table.csv", b"id,name\n1,\"unclosed\n"),
            Err(DocumentError::InvalidStructure)
        );
        assert_eq!(
            DocumentJob::from_utf8("table.csv", b"id,name\n1,\"quoted\"tail\n"),
            Err(DocumentError::InvalidStructure)
        );
    }

    #[test]
    fn json_preserves_shape_and_translates_selected_string_paths() {
        let source = "{\n  \"id\": 7,\n  \"profile\": { \"name\": \"Alice\", \"note\": \"Hello \\\"world\\\"\" },\n  \"items\": [\"one\", {\"label\": \"two\"}],\n  \"active\": true,\n  \"missing\": null\n}\n";
        let include = vec!["/profile/name".to_owned(), "/items/1/label".to_owned()];
        let exclude = vec!["/items/1/label".to_owned()];
        let mut job = DocumentJob::from_utf8_with_json_paths(
            "payload.json",
            source.as_bytes(),
            Some(&include),
            Some(&exclude),
        )
        .expect("json");
        assert_eq!(job.format, DocumentFormat::Json);
        assert_eq!(job.pending_count(), 1);
        let name = job
            .segments
            .iter()
            .enumerate()
            .find(|(_, segment)| segment.source_text == "\"Alice\"")
            .map(|(index, _)| index)
            .expect("name");
        assert_eq!(job.translation_source_text(name).unwrap(), "Alice");
        job.apply_translation(name, "爱丽丝")
            .expect("name translation");
        assert_eq!(
            job.reconstruct().unwrap(),
            source.replace("\"Alice\"", "\"爱丽丝\"")
        );
        assert!(
            job.segments
                .iter()
                .any(|segment| segment.source_text == "\"id\""
                    && segment.kind == DocumentSegmentKind::Verbatim)
        );
        assert!(
            job.segments
                .iter()
                .any(|segment| segment.source_text.contains("true")
                    && segment.kind == DocumentSegmentKind::Verbatim)
        );
    }

    #[test]
    fn json_defaults_to_all_values_and_reencodes_escaped_translation() {
        let source = r#"{"quote":"原文 \\\"引号\\\"","array":["one","two"],"slash/key":"value"}"#;
        let mut job = DocumentJob::from_utf8("payload.json", source.as_bytes()).expect("json");
        let prose = job
            .segments
            .iter()
            .enumerate()
            .filter_map(|(index, segment)| {
                (segment.kind == DocumentSegmentKind::Prose).then_some(index)
            })
            .collect::<Vec<_>>();
        assert_eq!(prose.len(), 4);
        for index in prose {
            let source_text = job.translation_source_text(index).unwrap().into_owned();
            job.apply_translation(index, format!("{source_text}-译"))
                .expect("translate json value");
        }
        let output = job.reconstruct().expect("reconstruct json");
        let value = serde_json::from_str::<serde_json::Value>(&output).expect("valid json");
        assert!(
            value["quote"]
                .as_str()
                .is_some_and(|text| text.ends_with("-译"))
        );
        assert_eq!(value["array"][0], "one-译");
    }

    #[test]
    fn rejects_malformed_json_structure() {
        assert_eq!(
            DocumentJob::from_utf8("payload.json", br#"{"name":"unclosed}"#),
            Err(DocumentError::InvalidStructure)
        );
        assert_eq!(
            DocumentJob::from_utf8("payload.json", br#"{"name":"ok" "other":"bad"}"#),
            Err(DocumentError::InvalidStructure)
        );
        assert_eq!(
            DocumentJob::from_utf8("payload.json", br#"{"name":"bad\q"}"#),
            Err(DocumentError::InvalidStructure)
        );
    }

    #[test]
    fn html_preserves_tags_attributes_scripts_and_styles() {
        let source = "<!DOCTYPE html>\n<html id=\"root\" data-page=\"1\"><head><style>.x { color: red; }</style><script>if (a < b) c();</script></head><body><p>Hello <a href=\"https://example.test\">world</a>!</p></body></html>";
        let mut job = DocumentJob::from_utf8("page.html", source.as_bytes()).expect("html");
        assert_eq!(job.format, DocumentFormat::Html);
        assert_eq!(job.pending_count(), 3);
        let prose = job
            .segments
            .iter()
            .enumerate()
            .filter_map(|(index, segment)| {
                (segment.kind == DocumentSegmentKind::Prose).then_some(index)
            })
            .collect::<Vec<_>>();
        for index in prose {
            let source_text = job.translation_source_text(index).unwrap().into_owned();
            job.apply_translation(index, format!("{source_text}-译<safe>"))
                .expect("translate html text");
        }
        let output = job.reconstruct().expect("reconstruct html");
        assert!(output.contains("id=\"root\" data-page=\"1\""));
        assert!(output.contains("<style>.x { color: red; }</style>"));
        assert!(output.contains("<script>if (a < b) c();</script>"));
        assert!(output.contains("&lt;safe&gt;"));
        assert!(output.contains("href=\"https://example.test\""));
    }

    #[test]
    fn rejects_malformed_html_structure() {
        assert_eq!(
            DocumentJob::from_utf8("page.html", b"<html><body>text</html>"),
            Err(DocumentError::InvalidStructure)
        );
        assert_eq!(
            DocumentJob::from_utf8("page.html", b"<html><script>if (a < b)</html>"),
            Err(DocumentError::InvalidStructure)
        );
        assert_eq!(
            DocumentJob::from_utf8("page.html", b"<html><body><p>text"),
            Err(DocumentError::InvalidStructure)
        );
    }
}
