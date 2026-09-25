// Copyright (c) 2026 BogdanNL (https://github.com/BogdanNL)
// SPDX-License-Identifier: MIT

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);
const STREAM: &[u8] = &[0, 4, 0x82, 0x24, 0x25, 0x8f, 0x80, 0x7f];
pub const PLAIN: &[u8] = b"AIAIAIAIAIAIA";

pub struct Workspace(pub PathBuf);

impl Workspace {
    pub fn new() -> Self {
        let id = NEXT.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("stix-test-{}-{id}", std::process::id()));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}

impl Drop for Workspace {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn put16(bytes: &mut [u8], offset: usize, value: u16) {
    bytes[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
}

fn put32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn catalog(directory: &str) -> Vec<u8> {
    let mut catalog = Vec::new();
    for (count, name) in [(2, ""), (1, directory)] {
        let mut record = vec![0; 7 + name.len()];
        let length = record.len() as u16;
        put16(&mut record, 0, count);
        put16(&mut record, 2, length);
        put16(&mut record, 4, name.len() as u16);
        record[6..6 + name.len()].copy_from_slice(name.as_bytes());
        catalog.extend(record);
    }
    for (index, name) in [(0, "first.txt"), (1, "second.txt"), (0, "third.txt")] {
        let mut record = vec![0; 30 + name.len()];
        let length = record.len() as u16;
        record[0] = 8;
        put16(&mut record, 1, index);
        put32(&mut record, 3, PLAIN.len() as u32);
        put32(&mut record, 7, STREAM.len() as u32);
        put16(&mut record, 23, length);
        record[29] = name.len() as u8;
        record[30..].copy_from_slice(name.as_bytes());
        catalog.extend(record);
    }
    catalog
}

pub fn fixture(root: &Path, nested: bool, count: usize) -> Vec<PathBuf> {
    fixture_directory(root, nested, count, "Nested\\Folder")
}

pub fn fixture_directory(root: &Path, nested: bool, count: usize, directory: &str) -> Vec<PathBuf> {
    let catalog = catalog(directory);
    let data = STREAM.repeat(3);
    let chunk = data.len() / count;
    let mut paths = Vec::new();
    for number in 1..=count {
        let parent = if nested {
            root.join(format!("dIsK{number}"))
        } else {
            root.to_path_buf()
        };
        fs::create_dir_all(&parent).unwrap();
        let path = parent.join(format!("StD.{number}"));
        let mut bytes = vec![0; 255];
        bytes[..4].copy_from_slice(&[0x13, 0x5d, 0x65, 0x8c]);
        put16(&mut bytes, 12, 3);
        put32(&mut bytes, 18, (255 + data.len() + catalog.len()) as u32);
        bytes[30] = if number == 1 { count as u8 } else { 0 };
        bytes[31] = number as u8;
        put32(&mut bytes, 41, (255 + chunk) as u32);
        put16(&mut bytes, 49, 2);
        bytes.extend_from_slice(&data[(number - 1) * chunk..number * chunk]);
        bytes.extend_from_slice(&catalog);
        fs::write(&path, bytes).unwrap();
        paths.push(path);
    }
    paths
}
