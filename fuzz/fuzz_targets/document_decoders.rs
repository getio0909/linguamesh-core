#![no_main]

use libfuzzer_sys::fuzz_target;
use linguamesh_document::{DocumentJob, MAX_DOCUMENT_BYTES};

const EXTENSIONS: [&str; 12] = [
    "txt", "md", "srt", "vtt", "csv", "html", "json", "docx", "pptx", "xlsx", "epub", "pdf",
];

fuzz_target!(|input: &[u8]| {
    let selector = input.first().copied().unwrap_or_default() as usize % EXTENSIONS.len();
    let contents = input.get(1..).unwrap_or_default();
    let bounded = &contents[..contents.len().min(MAX_DOCUMENT_BYTES)];
    let source_name = format!("fuzz-input.{}", EXTENSIONS[selector]);
    let _ = DocumentJob::from_utf8(source_name, bounded);
});
