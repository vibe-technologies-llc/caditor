use serde_json::Value;

use super::{
    ChunkKind, JOURNAL_MAGIC, MODEL_MAGIC, Magic, Piece, model::file_from_records, parse,
    push_packed, start_file, value,
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
        let json = edit(index, json_of(&chunk.unpack(None).unwrap()));
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

pub(crate) fn model_chunk_count(bytes: &[u8]) -> usize {
    parse(bytes, &MODEL_MAGIC).unwrap().pieces.len()
}
