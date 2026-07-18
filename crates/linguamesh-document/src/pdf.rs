use flate2::Compression;
use flate2::read::ZlibDecoder;
use flate2::write::ZlibEncoder;
use std::collections::HashMap;
use std::fmt::Write as FmtWrite;
use std::io::{Read, Write};

use crate::{DocumentError, DocumentJob, DocumentSegment, DocumentSegmentKind, MAX_DOCUMENT_BYTES};

const MAX_PDF_PAGES: usize = 256;
const MAX_PDF_OBJECTS: usize = 4096;
const MAX_PDF_OPERATORS: usize = 100_000;

#[derive(Clone, Debug)]
struct PdfObject {
    id: u32,
    start: usize,
    end: usize,
    body_start: usize,
    body_end: usize,
}

#[derive(Clone, Debug)]
struct PdfTextSpan {
    object_id: u32,
    start: usize,
    end: usize,
    page: usize,
    text: String,
    x: f32,
    y: f32,
}

#[derive(Clone, Debug)]
struct PdfPage {
    width: f32,
    height: f32,
}

#[derive(Clone, Debug)]
struct PdfDocument {
    objects: Vec<PdfObject>,
    pages: Vec<PdfPage>,
    spans: Vec<PdfTextSpan>,
}

fn is_space(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\r' | b'\n' | b'\x0c' | b'\0')
}

fn has_token(data: &[u8], token: &[u8]) -> bool {
    data.windows(token.len()).any(|window| window == token)
}

fn parse_objects(pdf: &[u8]) -> Result<Vec<PdfObject>, DocumentError> {
    let mut objects = Vec::new();
    let mut cursor = 0usize;
    while cursor < pdf.len() {
        if !pdf[cursor].is_ascii_digit() || (cursor > 0 && pdf[cursor - 1].is_ascii_digit()) {
            cursor += 1;
            continue;
        }
        let start = cursor;
        let mut id = 0u32;
        while cursor < pdf.len() && pdf[cursor].is_ascii_digit() {
            id = id
                .checked_mul(10)
                .and_then(|value| value.checked_add(u32::from(pdf[cursor] - b'0')))
                .ok_or(DocumentError::InvalidStructure)?;
            cursor += 1;
        }
        while cursor < pdf.len() && is_space(pdf[cursor]) {
            cursor += 1;
        }
        if !pdf
            .get(cursor..)
            .is_some_and(|tail| tail.starts_with(b"0 obj"))
        {
            continue;
        }
        let body_start = cursor + 5;
        let Some(relative_end) = pdf[body_start..]
            .windows(6)
            .position(|window| window == b"endobj")
        else {
            return Err(DocumentError::InvalidStructure);
        };
        let end = body_start + relative_end + 6;
        objects.push(PdfObject {
            id,
            start,
            end,
            body_start,
            body_end: body_start + relative_end,
        });
        if objects.len() > MAX_PDF_OBJECTS {
            return Err(DocumentError::InvalidStructure);
        }
        cursor = end;
    }
    if objects.is_empty() {
        return Err(DocumentError::InvalidStructure);
    }
    Ok(objects)
}

fn object_body<'a>(pdf: &'a [u8], object: &PdfObject) -> &'a [u8] {
    &pdf[object.body_start..object.body_end]
}

fn object_by_id(objects: &[PdfObject], id: u32) -> Result<&PdfObject, DocumentError> {
    objects
        .iter()
        .find(|object| object.id == id)
        .ok_or(DocumentError::InvalidStructure)
}

fn parse_number(data: &[u8], start: usize) -> Option<(f32, usize)> {
    let mut end = start;
    while end < data.len()
        && (data[end].is_ascii_digit() || matches!(data[end], b'+' | b'-' | b'.'))
    {
        end += 1;
    }
    (end > start)
        .then(|| {
            std::str::from_utf8(&data[start..end])
                .ok()?
                .parse()
                .ok()
                .map(|value| (value, end))
        })
        .flatten()
}

fn media_box(data: &[u8]) -> (f32, f32) {
    let Some(start) = data.windows(9).position(|window| window == b"/MediaBox") else {
        return (612.0, 792.0);
    };
    let mut cursor = start + 9;
    while cursor < data.len() && is_space(data[cursor]) {
        cursor += 1;
    }
    if data.get(cursor) != Some(&b'[') {
        return (612.0, 792.0);
    }
    cursor += 1;
    let mut values = [0.0; 4];
    for value in &mut values {
        while cursor < data.len() && is_space(data[cursor]) {
            cursor += 1;
        }
        let Some((number, next)) = parse_number(data, cursor) else {
            return (612.0, 792.0);
        };
        *value = number;
        cursor = next;
    }
    let width = (values[2] - values[0]).abs();
    let height = (values[3] - values[1]).abs();
    if width > 0.0 && height > 0.0 && width <= 20_000.0 && height <= 20_000.0 {
        (width, height)
    } else {
        (612.0, 792.0)
    }
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn referenced_objects(data: &[u8], marker: &[u8]) -> Vec<u32> {
    let Some(relative) = data
        .windows(marker.len())
        .position(|window| window == marker)
    else {
        return Vec::new();
    };
    let mut cursor = relative + marker.len();
    while cursor < data.len() && is_space(data[cursor]) {
        cursor += 1;
    }
    let mut ids = Vec::new();
    let array = data.get(cursor) == Some(&b'[');
    if array {
        cursor += 1;
    }
    loop {
        while cursor < data.len() && is_space(data[cursor]) {
            cursor += 1;
        }
        let Some((id, next)) = parse_number(data, cursor) else {
            break;
        };
        cursor = next;
        while cursor < data.len() && is_space(data[cursor]) {
            cursor += 1;
        }
        let Some((generation, next)) = parse_number(data, cursor) else {
            break;
        };
        cursor = next;
        while cursor < data.len() && is_space(data[cursor]) {
            cursor += 1;
        }
        if data
            .get(cursor..)
            .is_some_and(|tail| tail.starts_with(b"R"))
            && generation == 0.0
            && (1.0..=4_096.0).contains(&id)
            && id.fract() == 0.0
        {
            ids.push(id as u32);
            cursor += 1;
        } else {
            break;
        }
        if !array || data.get(cursor) == Some(&b']') {
            break;
        }
    }
    ids
}

fn stream_data<'a>(pdf: &'a [u8], object: &PdfObject) -> Result<(&'a [u8], bool), DocumentError> {
    let body = object_body(pdf, object);
    let Some(relative_stream) = body.windows(6).position(|window| window == b"stream") else {
        return Err(DocumentError::InvalidStructure);
    };
    let mut start = relative_stream + 6;
    if body.get(start) == Some(&b'\r') {
        start += 1;
    }
    if body.get(start) == Some(&b'\n') {
        start += 1;
    }
    let end = body[start..]
        .windows(9)
        .position(|window| window == b"endstream")
        .map(|offset| start + offset)
        .ok_or(DocumentError::InvalidStructure)?;
    let compressed = if has_token(&body[..relative_stream], b"/FlateDecode") {
        true
    } else if has_token(&body[..relative_stream], b"/Filter") {
        return Err(DocumentError::InvalidStructure);
    } else {
        false
    };
    Ok((&body[start..end], compressed))
}

fn decode_stream(data: &[u8], compressed: bool) -> Result<Vec<u8>, DocumentError> {
    if data.len() > MAX_DOCUMENT_BYTES {
        return Err(DocumentError::TooLarge);
    }
    if !compressed {
        return Ok(data.to_vec());
    }
    let zlib_decoder = ZlibDecoder::new(data);
    let mut decoded = Vec::new();
    zlib_decoder
        .take((MAX_DOCUMENT_BYTES + 1) as u64)
        .read_to_end(&mut decoded)
        .map_err(|_| DocumentError::InvalidStructure)?;
    if decoded.len() > MAX_DOCUMENT_BYTES {
        return Err(DocumentError::TooLarge);
    }
    Ok(decoded)
}

fn decode_pdf_text(bytes: &[u8]) -> String {
    String::from_utf8(bytes.to_vec())
        .unwrap_or_else(|_| bytes.iter().map(|byte| char::from(*byte)).collect())
}

fn pdf_literal(data: &[u8], start: usize) -> Result<(String, usize, usize), DocumentError> {
    if data.get(start) != Some(&b'(') {
        return Err(DocumentError::InvalidStructure);
    }
    let mut cursor = start + 1;
    let mut depth = 1usize;
    let mut output = Vec::new();
    while cursor < data.len() {
        match data[cursor] {
            b'(' => {
                depth = depth
                    .checked_add(1)
                    .ok_or(DocumentError::InvalidStructure)?;
                output.push(b'(');
                cursor += 1;
            }
            b')' => {
                depth = depth
                    .checked_sub(1)
                    .ok_or(DocumentError::InvalidStructure)?;
                if depth == 0 {
                    let text = decode_pdf_text(&output);
                    return Ok((text, start, cursor + 1));
                }
                output.push(b')');
                cursor += 1;
            }
            b'\\' => {
                cursor += 1;
                let Some(escaped) = data.get(cursor).copied() else {
                    return Err(DocumentError::InvalidStructure);
                };
                match escaped {
                    b'n' => output.push(b'\n'),
                    b'r' => output.push(b'\r'),
                    b't' => output.push(b'\t'),
                    b'b' => output.push(8),
                    b'f' => output.push(12),
                    b'\n' => {}
                    b'\r' => {
                        if data.get(cursor + 1) == Some(&b'\n') {
                            cursor += 1;
                        }
                    }
                    b'0'..=b'7' => {
                        let mut value = u16::from(escaped - b'0');
                        let mut count = 1;
                        while count < 3
                            && data
                                .get(cursor + 1)
                                .is_some_and(|byte| (b'0'..=b'7').contains(byte))
                        {
                            cursor += 1;
                            value = value * 8 + u16::from(data[cursor] - b'0');
                            count += 1;
                        }
                        output.push(
                            u8::try_from(value).map_err(|_| DocumentError::InvalidStructure)?,
                        );
                    }
                    other => output.push(other),
                }
                cursor += 1;
            }
            other => {
                output.push(other);
                cursor += 1;
            }
        }
    }
    Err(DocumentError::InvalidStructure)
}

#[allow(clippy::too_many_lines)]
fn extract_stream_spans(
    data: &[u8],
    object_id: u32,
    page: usize,
) -> Result<Vec<PdfTextSpan>, DocumentError> {
    let mut spans = Vec::new();
    let mut cursor = 0usize;
    let mut in_text = false;
    let mut numbers = Vec::<f32>::new();
    let mut x = 0.0;
    let mut y = 0.0;
    let mut operators = 0usize;
    while cursor < data.len() {
        if is_space(data[cursor]) {
            cursor += 1;
            continue;
        }
        if data[cursor] == b'%' {
            cursor = data[cursor..]
                .iter()
                .position(|byte| *byte == b'\n')
                .map_or(data.len(), |offset| cursor + offset + 1);
            continue;
        }
        if data[cursor] == b'(' {
            let (text, start, end) = pdf_literal(data, cursor)?;
            if in_text && !text.is_empty() {
                spans.push(PdfTextSpan {
                    object_id,
                    start,
                    end,
                    page,
                    text,
                    x,
                    y,
                });
            }
            cursor = end + 1;
            continue;
        }
        if data[cursor] == b'<' && data.get(cursor + 1) != Some(&b'<') {
            let Some(relative_end) = data[cursor + 1..].iter().position(|byte| *byte == b'>')
            else {
                return Err(DocumentError::InvalidStructure);
            };
            let end = cursor + relative_end + 2;
            if in_text {
                let mut bytes = Vec::new();
                let mut high = None;
                for byte in &data[cursor + 1..end - 1] {
                    if is_space(*byte) {
                        continue;
                    }
                    let value = match byte {
                        b'0'..=b'9' => byte - b'0',
                        b'a'..=b'f' => byte - b'a' + 10,
                        b'A'..=b'F' => byte - b'A' + 10,
                        _ => return Err(DocumentError::InvalidStructure),
                    };
                    if let Some(left) = high.take() {
                        bytes.push(left * 16 + value);
                    } else {
                        high = Some(value);
                    }
                }
                if let Some(left) = high {
                    bytes.push(left * 16);
                }
                let text = decode_pdf_text(&bytes);
                if !text.is_empty() {
                    spans.push(PdfTextSpan {
                        object_id,
                        start: cursor,
                        end,
                        page,
                        text,
                        x,
                        y,
                    });
                }
            }
            cursor = end;
            continue;
        }
        if let Some((number, next)) = parse_number(data, cursor) {
            numbers.push(number);
            cursor = next;
            continue;
        }
        let start = cursor;
        while cursor < data.len()
            && !is_space(data[cursor])
            && !matches!(data[cursor], b'[' | b']' | b'(' | b')' | b'<')
        {
            cursor += 1;
        }
        if start == cursor {
            cursor += 1;
            continue;
        }
        let operator = &data[start..cursor];
        operators += 1;
        if operators > MAX_PDF_OPERATORS {
            return Err(DocumentError::InvalidStructure);
        }
        match operator {
            b"BT" => in_text = true,
            b"ET" => {
                in_text = false;
                numbers.clear();
            }
            b"Td" | b"TD" if numbers.len() >= 2 => {
                x += numbers[numbers.len() - 2];
                y += numbers[numbers.len() - 1];
                numbers.clear();
            }
            b"Tm" if numbers.len() >= 6 => {
                x = numbers[numbers.len() - 2];
                y = numbers[numbers.len() - 1];
                numbers.clear();
            }
            b"T*" => {
                y -= 12.0;
                numbers.clear();
            }
            b"Tj" | b"TJ" | b"'" | b"\"" => numbers.clear(),
            _ => {
                if operator.len() <= 3 {
                    numbers.clear();
                }
            }
        }
    }
    Ok(spans)
}

fn parse_pdf(pdf: &[u8]) -> Result<PdfDocument, DocumentError> {
    if pdf.len() > MAX_DOCUMENT_BYTES
        || !pdf.starts_with(b"%PDF-")
        || !pdf.windows(5).any(|window| window == b"%%EOF")
    {
        return Err(DocumentError::InvalidStructure);
    }
    if has_token(pdf, b"/Encrypt") {
        return Err(DocumentError::InvalidStructure);
    }
    let objects = parse_objects(pdf)?;
    let mut pages = Vec::new();
    let mut spans = Vec::new();
    for object in &objects {
        let body = object_body(pdf, object);
        if !has_token(body, b"/Type") || !has_token(body, b"/Page") || has_token(body, b"/Pages") {
            continue;
        }
        if pages.len() >= MAX_PDF_PAGES {
            return Err(DocumentError::InvalidStructure);
        }
        let page_index = pages.len();
        let (width, height) = media_box(body);
        pages.push(PdfPage { width, height });
        for stream_id in referenced_objects(body, b"/Contents") {
            let stream_object = object_by_id(&objects, stream_id)?;
            let (stream, compressed) = stream_data(pdf, stream_object)?;
            let decoded = decode_stream(stream, compressed)?;
            spans.extend(extract_stream_spans(&decoded, stream_id, page_index)?);
        }
    }
    if pages.is_empty() {
        return Err(DocumentError::InvalidStructure);
    }
    Ok(PdfDocument {
        objects,
        pages,
        spans,
    })
}

pub(crate) fn inspect(package: &[u8]) -> Result<Vec<DocumentSegment>, DocumentError> {
    let document = parse_pdf(package)?;
    Ok(document
        .spans
        .iter()
        .enumerate()
        .map(|(index, span)| {
            let next = document.spans.get(index + 1);
            let line_ending = next.is_some_and(|next| {
                next.page != span.page || (span.y - next.y).abs() > 4.0 || next.x + 1.0 < span.x
            });
            DocumentSegment {
                index,
                kind: if span
                    .text
                    .chars()
                    .any(|character| !character.is_whitespace())
                {
                    DocumentSegmentKind::Prose
                } else {
                    DocumentSegmentKind::Verbatim
                },
                source_text: span.text.clone(),
                translated_text: None,
                line_ending: if line_ending {
                    "\n".to_owned()
                } else {
                    String::new()
                },
            }
        })
        .collect())
}

fn pdf_escape(text: &str) -> Result<Vec<u8>, DocumentError> {
    if !text.is_ascii() {
        return Err(DocumentError::PdfTextEncodingUnsupported);
    }
    let mut output = Vec::with_capacity(text.len());
    for byte in text.bytes() {
        match byte {
            b'(' | b')' | b'\\' => {
                output.push(b'\\');
                output.push(byte);
            }
            b'\n' => output.extend_from_slice(br"\n"),
            b'\r' => output.extend_from_slice(br"\r"),
            _ => output.push(byte),
        }
    }
    Ok(output)
}

fn rewrite_stream(
    data: &[u8],
    spans: &[(&PdfTextSpan, &DocumentSegment)],
) -> Result<Vec<u8>, DocumentError> {
    let mut output = Vec::with_capacity(data.len());
    let mut cursor = 0usize;
    for (span, segment) in spans {
        output.extend_from_slice(&data[cursor..span.start]);
        if segment.source_text != span.text {
            return Err(DocumentError::InvalidStructure);
        }
        match segment.kind {
            DocumentSegmentKind::Verbatim => output.extend_from_slice(&data[span.start..span.end]),
            DocumentSegmentKind::Prose => {
                let translated = segment
                    .translated_text
                    .as_deref()
                    .ok_or(DocumentError::SegmentIncomplete(segment.index))?;
                output.push(b'(');
                output.extend_from_slice(&pdf_escape(translated)?);
                output.push(b')');
            }
        }
        cursor = span.end;
    }
    output.extend_from_slice(&data[cursor..]);
    Ok(output)
}

fn rewrite_object(
    pdf: &[u8],
    object: &PdfObject,
    replacements: &[(&PdfTextSpan, &DocumentSegment)],
) -> Result<Vec<u8>, DocumentError> {
    let body = object_body(pdf, object);
    let Some(relative_stream) = body.windows(6).position(|window| window == b"stream") else {
        return Err(DocumentError::InvalidStructure);
    };
    let (old_stream, compressed) = stream_data(pdf, object)?;
    let decoded = decode_stream(old_stream, compressed)?;
    let rewritten = rewrite_stream(&decoded, replacements)?;
    let encoded = if compressed {
        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::fast());
        encoder
            .write_all(&rewritten)
            .map_err(|_| DocumentError::InvalidStructure)?;
        encoder
            .finish()
            .map_err(|_| DocumentError::InvalidStructure)?
    } else {
        rewritten
    };
    let mut dictionary = body[..relative_stream].to_vec();
    let Some(length_start) = dictionary
        .windows(7)
        .position(|window| window == b"/Length")
    else {
        return Err(DocumentError::InvalidStructure);
    };
    let mut length_end = length_start + 7;
    while length_end < dictionary.len() && is_space(dictionary[length_end]) {
        length_end += 1;
    }
    while length_end < dictionary.len() && dictionary[length_end].is_ascii_digit() {
        length_end += 1;
    }
    dictionary.splice(
        length_start + 7..length_end,
        format!(" {}", encoded.len()).bytes(),
    );
    if !compressed
        && let Some(filter_start) = dictionary
            .windows(7)
            .position(|window| window == b"/Filter")
    {
        let mut filter_end = filter_start + 7;
        while filter_end < dictionary.len()
            && dictionary[filter_end] != b'/'
            && dictionary[filter_end] != b'>'
        {
            filter_end += 1;
        }
        dictionary.drain(filter_start..filter_end);
    }
    let mut output = Vec::new();
    output.extend_from_slice(&pdf[object.start..object.body_start]);
    output.extend_from_slice(&dictionary);
    output.extend_from_slice(b"stream\n");
    output.extend_from_slice(&encoded);
    output.extend_from_slice(b"\nendstream\nendobj");
    Ok(output)
}

pub(crate) fn reconstruct(job: &DocumentJob, package: &[u8]) -> Result<Vec<u8>, DocumentError> {
    let document = parse_pdf(package)?;
    if document.spans.len() != job.segments.len() {
        return Err(DocumentError::InvalidStructure);
    }
    if job.segments.is_empty() {
        return Ok(package.to_vec());
    }
    let mut by_object: HashMap<u32, Vec<(&PdfTextSpan, &DocumentSegment)>> = HashMap::new();
    for (span, segment) in document.spans.iter().zip(&job.segments) {
        by_object
            .entry(span.object_id)
            .or_default()
            .push((span, segment));
    }
    let mut replacements = Vec::<(usize, usize, Vec<u8>)>::new();
    for (object_id, spans) in by_object {
        let object = object_by_id(&document.objects, object_id)?;
        replacements.push((
            object.start,
            object.end,
            rewrite_object(package, object, &spans)?,
        ));
    }
    replacements.sort_unstable_by(|left, right| right.0.cmp(&left.0));
    let mut output = package.to_vec();
    for (start, end, replacement) in replacements {
        output.splice(start..end, replacement);
    }
    if output.len() > MAX_DOCUMENT_BYTES {
        return Err(DocumentError::OutputTooLarge);
    }
    parse_pdf(&output).map(|_| output)
}

pub(crate) fn alternative_html(
    job: &DocumentJob,
    package: &[u8],
) -> Result<Vec<u8>, DocumentError> {
    let document = parse_pdf(package)?;
    if document.spans.len() != job.segments.len() {
        return Err(DocumentError::InvalidStructure);
    }
    let mut output = String::from(
        "<!doctype html><html><head><meta charset=\"utf-8\"><title>LinguaMesh PDF translation</title></head><body>",
    );
    let mut current_page = None;
    for (span, segment) in document.spans.iter().zip(&job.segments) {
        if current_page != Some(span.page) {
            if current_page.is_some() {
                output.push_str("</section>");
            }
            let page = document
                .pages
                .get(span.page)
                .ok_or(DocumentError::InvalidStructure)?;
            write!(
                output,
                "<section data-page=\"{}\" data-width=\"{}\" data-height=\"{}\"><h2>Page {}</h2>",
                span.page + 1,
                page.width,
                page.height,
                span.page + 1
            )
            .map_err(|_| DocumentError::InvalidStructure)?;
            current_page = Some(span.page);
        }
        let text = segment.output_text()?;
        output.push_str("<p>");
        output.push_str(&html_escape(text));
        output.push_str("</p>");
    }
    if current_page.is_some() {
        output.push_str("</section>");
    }
    output.push_str("</body></html>");
    if output.len() > MAX_DOCUMENT_BYTES {
        return Err(DocumentError::OutputTooLarge);
    }
    Ok(output.into_bytes())
}

fn html_escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}
