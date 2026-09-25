// Copyright (c) 2026 BogdanNL (https://github.com/BogdanNL)
// SPDX-License-Identifier: MIT

//! Discover archive signatures first, then construct complete volume maps.

use crate::{archive::safe_path, invalid};
use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Path, PathBuf};

#[derive(Debug)]
pub struct ArchiveGroup {
    pub name: String,
    pub volumes: Vec<PathBuf>,
}

struct Candidate {
    path: PathBuf,
    number: u8,
    count: u8,
}

fn probe(path: &Path) -> io::Result<Option<Candidate>> {
    let mut file = File::open(path)?;
    let mut signature = [0; 4];
    match file.read_exact(&mut signature) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e),
    }
    if signature != [0x13, 0x5d, 0x65, 0x8c] {
        return Ok(None);
    }
    let mut header = [0; 255];
    header[..4].copy_from_slice(&signature);
    file.read_exact(&mut header[4..]).map_err(|e| {
        invalid(format!(
            "{}: incomplete archive header: {e}",
            path.display()
        ))
    })?;
    Ok(Some(Candidate {
        path: path.to_path_buf(),
        number: header[31],
        count: header[30],
    }))
}

fn numbered_name(path: &Path) -> io::Result<Option<(String, usize)>> {
    let Some(extension) = path.extension().and_then(|s| s.to_str()) else {
        return Ok(None);
    };
    if extension.is_empty() || !extension.bytes().all(|b| b.is_ascii_digit()) {
        return Ok(None);
    }
    let number = extension
        .parse::<usize>()
        .map_err(|_| invalid("Volume number is too large"))?;
    if number == 0 {
        return Err(invalid(format!(
            "Volume numbering must start at 1: {}",
            path.display()
        )));
    }
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .ok_or_else(|| invalid("Invalid archive basename"))?;
    Ok(Some((stem.to_owned(), number)))
}

fn collect_files(root: &Path) -> io::Result<Vec<PathBuf>> {
    let mut pending = vec![root.to_path_buf()];
    let mut files = Vec::new();
    while let Some(directory) = pending.pop() {
        for item in fs::read_dir(&directory)? {
            let item = item?;
            let kind = item.file_type()?;
            if kind.is_dir() {
                pending.push(item.path());
            } else if kind.is_file() || (kind.is_symlink() && item.path().is_file()) {
                files.push(item.path());
            }
        }
    }
    files.sort();
    Ok(files)
}

fn disk_directory(path: &Path) -> bool {
    path.file_name()
        .and_then(|s| s.to_str())
        .is_some_and(|name| {
            let upper = name.to_ascii_uppercase();
            upper.strip_prefix("DISK").is_some_and(|suffix| {
                !suffix.is_empty() && suffix.bytes().all(|b| b.is_ascii_digit())
            })
        })
}

/// A directory selects all header-recognized archives in its tree. A file
/// selects only its own group; sibling DISK directories are scanned for parts.
pub fn discover(input: &Path) -> io::Result<Vec<ArchiveGroup>> {
    discover_selected(input, false)
}

/// Select only archive names starting with "std", ignoring ASCII case.
pub fn discover_std(input: &Path) -> io::Result<Vec<ArchiveGroup>> {
    discover_selected(input, true)
}

fn std_name(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.to_ascii_lowercase().starts_with("std"))
}

fn discover_selected(input: &Path, std_only: bool) -> io::Result<Vec<ArchiveGroup>> {
    let input = fs::canonicalize(input)?;
    let (files, selected) = if input.is_dir() {
        (collect_files(&input)?, None)
    } else if input.is_file() {
        if std_only && !std_name(&input) {
            return Err(invalid("The selected archive name does not start with std"));
        }
        if probe(&input)?.is_none() {
            return Err(invalid("Not an InstallShield 3.x archive"));
        }
        if let Some((name, number)) = numbered_name(&input)? {
            if number != 1 {
                return Err(invalid("Pass the first archive volume (.1)"));
            }
            let parent = input.parent().unwrap();
            let root = if disk_directory(parent) {
                parent.parent().unwrap_or(parent)
            } else {
                parent
            };
            (collect_files(root)?, Some(name))
        } else {
            (vec![input.clone()], None)
        }
    } else {
        return Err(invalid("Input must be a regular file or a directory"));
    };

    let mut numbered: BTreeMap<String, BTreeMap<usize, Candidate>> = BTreeMap::new();
    let mut singles = Vec::new();
    for path in files {
        if std_only && !std_name(&path) {
            continue;
        }
        // Explicit-file selection must not depend on unrelated damaged archives.
        if let Some(selected) = &selected {
            if !path
                .file_stem()
                .and_then(|s| s.to_str())
                .is_some_and(|stem| stem.eq_ignore_ascii_case(selected))
                || !path
                    .extension()
                    .and_then(|s| s.to_str())
                    .is_some_and(|ext| !ext.is_empty() && ext.bytes().all(|b| b.is_ascii_digit()))
            {
                continue;
            }
        }
        let Some(candidate) = probe(&path)? else {
            continue;
        };
        if let Some((name, number)) = numbered_name(&path)? {
            if candidate.number != 0 && candidate.number as usize != number {
                return Err(invalid(format!(
                    "Filename/header volume number mismatch: {} (filename {number}, header {})",
                    path.display(),
                    candidate.number
                )));
            }
            let parts = numbered.entry(name.to_ascii_lowercase()).or_default();
            if let Some(previous) = parts.insert(number, candidate) {
                return Err(invalid(format!(
                    "Multiple candidates for volume {number} of {name}: {} and {}",
                    previous.path.display(),
                    path.display()
                )));
            }
        } else {
            if candidate.number > 1 || candidate.count > 1 {
                return Err(invalid(format!(
                    "Multipart archive requires numeric volume filenames: {}",
                    path.display()
                )));
            }
            singles.push(candidate);
        }
    }

    let mut groups = Vec::new();
    for (name, parts) in numbered {
        let first = parts
            .get(&1)
            .ok_or_else(|| invalid(format!("Missing volume 1 of {name}")))?;
        for (index, &number) in parts.keys().enumerate() {
            if number != index + 1 {
                return Err(invalid(format!("Missing volume {} of {name}", index + 1)));
            }
        }
        if first.count != 0 && first.count as usize != parts.len() {
            return Err(invalid(format!(
                "Volume count mismatch for {name}: header declares {}, found {}",
                first.count,
                parts.len()
            )));
        }
        let name = first.path.file_stem().unwrap().to_str().unwrap().to_owned();
        groups.push(ArchiveGroup {
            name,
            volumes: parts.into_values().map(|part| part.path).collect(),
        });
    }
    for candidate in singles {
        let name = candidate
            .path
            .file_name()
            .and_then(|s| s.to_str())
            .ok_or_else(|| invalid("Invalid archive filename"))?
            .to_owned();
        groups.push(ArchiveGroup {
            name,
            volumes: vec![candidate.path],
        });
    }
    if groups.is_empty() {
        return Err(invalid(if std_only {
            "No std* InstallShield 3.x archives found by header signature"
        } else {
            "No InstallShield 3.x archives found by header signature"
        }));
    }
    groups.sort_by_key(|group| group.name.to_ascii_lowercase());
    let mut names = BTreeMap::new();
    for group in &groups {
        if safe_path(&group.name, false)?.contains('/') {
            return Err(invalid(format!(
                "Archive group name contains a path separator: {}",
                group.name
            )));
        }
        if names
            .insert(group.name.to_lowercase(), &group.volumes[0])
            .is_some()
        {
            return Err(invalid(format!(
                "Ambiguous archive group name: {}",
                group.name
            )));
        }
    }
    Ok(groups)
}
