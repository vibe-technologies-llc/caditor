use std::cell::Cell;

use serde_json::Value;

use super::{
    ChunkKind, Codec, JOURNAL_MAGIC, MODEL_MAGIC, Magic, Piece, model::file_from_records, parse,
    push_chunk, push_packed, push_raw, start_file, value,
};
use crate::format::FORMAT_VERSION;

fn json_of(content: &[u8]) -> String {
    let value: Value = value::from_bytes(content).unwrap();
    serde_json::to_string(&value).unwrap()
}

fn content_of(json: &str) -> Vec<u8> {
    match serde_json::from_str::<Value>(json) {
        Ok(value) => value::to_bytes(&value).unwrap(),
        Err(_) => json.as_bytes().to_vec(),
    }
}

pub(crate) fn records_as_json(bytes: &[u8]) -> Vec<String> {
    parse(bytes, &MODEL_MAGIC)
        .unwrap()
        .chunks()
        .filter(|chunk| chunk.kind == Some(ChunkKind::Record))
        .map(|chunk| json_of(&chunk.unpack(None).unwrap()))
        .collect()
}

pub(crate) fn model_from_json(version: u32, records: &[String]) -> Vec<u8> {
    let contents: Vec<Vec<u8>> = records.iter().map(|record| content_of(record)).collect();
    file_from_records(version, &contents).unwrap()
}

pub(crate) fn current_model_from_json(records: &[String]) -> Vec<u8> {
    model_from_json(FORMAT_VERSION, records)
}

pub(crate) fn rewrite_journal(bytes: &[u8], edit: impl Fn(usize, String) -> String) -> Vec<u8> {
    let container = parse(bytes, &JOURNAL_MAGIC).unwrap();
    let mut rewritten = start_file(&JOURNAL_MAGIC, container.version);
    for (index, chunk) in container.chunks().enumerate() {
        let kind = chunk.kind.unwrap();
        let content = chunk.unpack(None).unwrap();
        if content.is_empty() {
            push_packed(&mut rewritten, kind, &content).unwrap();
            continue;
        }
        let json = edit(index, json_of(&content));
        push_packed(&mut rewritten, kind, &content_of(&json)).unwrap();
    }
    rewritten
}

pub(crate) fn corrupt_chunk(bytes: &[u8], magic: &Magic, index: usize) -> Vec<u8> {
    let container = parse(bytes, magic).unwrap();
    let Piece::Chunk(chunk) = container.pieces[index] else {
        panic!("piece {index} is already damaged");
    };
    let start = chunk.whole.as_ptr() as usize - bytes.as_ptr() as usize;
    let mut corrupted = bytes.to_vec();
    corrupted[start + chunk.whole.len() - 1] ^= 0x55;
    corrupted
}

pub(crate) fn with_unknown_codec(bytes: &[u8], record: usize) -> Vec<u8> {
    const UNKNOWN_CODEC: u8 = 200;
    let container = parse(bytes, &MODEL_MAGIC).unwrap();
    let mut rewritten = start_file(&MODEL_MAGIC, container.version);
    let mut records = 0;
    for chunk in container.chunks() {
        if chunk.kind != Some(ChunkKind::Record) {
            rewritten.extend_from_slice(chunk.whole);
            continue;
        }
        if records == record {
            let content = chunk.unpack(None).unwrap();
            push_raw(
                &mut rewritten,
                [ChunkKind::Record as u8, UNKNOWN_CODEC, 0],
                content.len(),
                &content,
            )
            .unwrap();
        } else {
            rewritten.extend_from_slice(chunk.whole);
        }
        records += 1;
    }
    rewritten
}

pub(crate) fn model_chunk_count(bytes: &[u8]) -> usize {
    parse(bytes, &MODEL_MAGIC).unwrap().pieces.len()
}

pub(crate) fn stored_copy(bytes: &[u8], magic: &Magic) -> Vec<u8> {
    let container = parse(bytes, magic).unwrap();
    let mut stored = start_file(magic, container.version);
    for chunk in container.chunks() {
        if chunk.codec == Some(Codec::ZstdAfterNewer) {
            stored.extend_from_slice(chunk.whole);
            continue;
        }
        let content = chunk.unpack(None).unwrap();
        push_chunk(
            &mut stored,
            chunk.kind.unwrap(),
            Codec::Stored,
            0,
            content.len(),
            &content,
        )
        .unwrap();
    }
    stored
}

pub(crate) fn push_foreign(bytes: &mut Vec<u8>, kind: u8, flags: u8, content: &[u8]) {
    push_raw(
        bytes,
        [kind, Codec::Stored as u8, flags],
        content.len(),
        content,
    )
    .unwrap();
}

thread_local! {
    static SLICE_LENGTH: Cell<Option<usize>> = const { Cell::new(None) };
}

pub(crate) fn slice_length() -> Option<usize> {
    SLICE_LENGTH.get()
}

pub(crate) fn with_slices_of<T>(length: usize, work: impl FnOnce() -> T) -> T {
    let before = SLICE_LENGTH.replace(Some(length));
    let result = work();
    SLICE_LENGTH.set(before);
    result
}

thread_local! {
    static WORTH_SHARING: Cell<Option<usize>> = const { Cell::new(None) };
}

pub(crate) fn worth_sharing() -> Option<usize> {
    WORTH_SHARING.get()
}

pub(crate) fn sharing_from<T>(length: usize, work: impl FnOnce() -> T) -> T {
    let before = WORTH_SHARING.replace(Some(length));
    let result = work();
    WORTH_SHARING.set(before);
    result
}

thread_local! {
    static MAX_DECOMPRESSED: Cell<Option<usize>> = const { Cell::new(None) };
}

pub(crate) fn max_decompressed() -> Option<usize> {
    MAX_DECOMPRESSED.get()
}

pub(crate) fn decompressing_at_most<T>(limit: usize, work: impl FnOnce() -> T) -> T {
    let before = MAX_DECOMPRESSED.replace(Some(limit));
    let result = work();
    MAX_DECOMPRESSED.set(before);
    result
}

thread_local! {
    static LARGEST_FILE: Cell<Option<usize>> = const { Cell::new(None) };
}

pub(crate) fn largest_file() -> Option<usize> {
    LARGEST_FILE.get()
}

pub(crate) fn files_of_at_most<T>(largest: usize, work: impl FnOnce() -> T) -> T {
    let before = LARGEST_FILE.replace(Some(largest));
    let result = work();
    LARGEST_FILE.set(before);
    result
}
