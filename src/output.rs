// Copyright (c) 2026 BogdanNL (https://github.com/BogdanNL)
// SPDX-License-Identifier: MIT

//! Files are published only after successful decompression and validation.

use crate::{archive::Archive, invalid};
use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufWriter, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static TEMP_ID: AtomicU64 = AtomicU64::new(0);

/// Merge directory spellings across selected archives without changing file names.
/// ExtForms has a fixed spelling; other directories use their first spelling.
pub fn merge_directories(archives: &mut [Archive]) {
    fn normalize(path: &str, known: &mut BTreeMap<String, String>) -> String {
        let mut result = String::new();
        for component in path.split('/').filter(|part| !part.is_empty()) {
            let preferred = if component.eq_ignore_ascii_case("extforms") {
                "ExtForms"
            } else {
                component
            };
            let candidate = if result.is_empty() {
                preferred.to_owned()
            } else {
                format!("{result}/{preferred}")
            };
            result = known
                .entry(candidate.to_lowercase())
                .or_insert(candidate)
                .clone();
        }
        result
    }

    let mut known = BTreeMap::new();
    for archive in archives {
        for directory in &mut archive.directories {
            *directory = normalize(directory, &mut known);
        }
        for entry in &mut archive.entries {
            if let Some((parent, name)) = entry.path.rsplit_once('/') {
                entry.path = format!("{}/{name}", normalize(parent, &mut known));
            }
        }
    }
}

fn check_directory(path: &Path, create: bool) -> io::Result<()> {
    let mut current = PathBuf::new();
    for component in path.components() {
        if component == Component::ParentDir {
            current.push(component);
            continue;
        }
        current.push(component);
        match fs::symlink_metadata(&current) {
            Ok(meta) if meta.is_dir() && !meta.file_type().is_symlink() => {}
            Ok(_) => {
                return Err(invalid(format!(
                    "Output directory is a symlink or is not a directory: {}",
                    current.display()
                )))
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                if create {
                    fs::create_dir(&current)?;
                }
            }
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

fn check_target(path: &Path, overwrite: bool) -> io::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.is_file() && !meta.file_type().is_symlink() && overwrite => Ok(()),
        Ok(_) => Err(invalid(format!(
            "Output path already exists (use --overwrite for regular files): {}",
            path.display()
        ))),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

pub fn preflight(archive: &Archive, root: &Path, overwrite: bool) -> io::Result<()> {
    check_directory(root, false)?;
    for directory in &archive.directories {
        check_directory(&root.join(directory), false)?;
    }
    let inputs: Vec<_> = archive
        .volumes
        .iter()
        .map(|v| fs::canonicalize(&v.path))
        .collect::<io::Result<_>>()?;
    for entry in &archive.entries {
        let target = root.join(&entry.path);
        check_directory(target.parent().unwrap(), false)?;
        check_target(&target, overwrite)?;
        if target.exists() && inputs.contains(&fs::canonicalize(&target)?) {
            return Err(invalid(
                "Extraction would overwrite an input archive volume",
            ));
        }
    }
    Ok(())
}

/// Validate the complete output map, including conflicts between archives.
pub fn preflight_all(jobs: &[(&Archive, PathBuf)], overwrite: bool) -> io::Result<()> {
    let inputs = jobs
        .iter()
        .flat_map(|(archive, _)| &archive.volumes)
        .map(|volume| fs::canonicalize(&volume.path))
        .collect::<io::Result<Vec<_>>>()?;
    let mut files = BTreeMap::new();
    let mut directories = Vec::new();
    for (archive, root) in jobs {
        preflight(archive, root, overwrite)?;
        directories.push(root.clone());
        directories.extend(archive.directories.iter().map(|dir| root.join(dir)));
        for entry in &archive.entries {
            let path = root.join(&entry.path);
            if path.exists() && inputs.contains(&fs::canonicalize(&path)?) {
                return Err(invalid(
                    "Extraction would overwrite an input archive volume",
                ));
            }
            let key = path.to_string_lossy().to_lowercase();
            if let Some(previous) = files.insert(key, path.clone()) {
                return Err(invalid(format!("Conflicting output paths across archives: {} and {}. Use separate archive folders without --std/--setup", previous.display(), path.display())));
            }
            if let Some(parent) = path.parent() {
                directories.push(parent.to_path_buf());
            }
        }
    }
    for directory in directories {
        for ancestor in directory.ancestors() {
            if files.contains_key(&ancestor.to_string_lossy().to_lowercase()) {
                return Err(invalid(format!(
                    "File/directory conflict across archives: {}",
                    ancestor.display()
                )));
            }
        }
    }
    Ok(())
}

pub fn create_directories(archive: &Archive, root: &Path) -> io::Result<()> {
    check_directory(root, true)?;
    for directory in &archive.directories {
        check_directory(&root.join(directory), true)?;
    }
    Ok(())
}

struct Temporary(PathBuf);

impl Drop for Temporary {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

fn temporary(parent: &Path) -> io::Result<(Temporary, File)> {
    for _ in 0..100 {
        let id = TEMP_ID.fetch_add(1, Ordering::Relaxed);
        let path = parent.join(format!(".stix-{}-{id}.tmp", std::process::id()));
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => return Ok((Temporary(path), file)),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    Err(invalid("Cannot create a unique temporary output file"))
}

pub fn extract_entry(
    archive: &Archive,
    index: usize,
    root: &Path,
    overwrite: bool,
) -> io::Result<()> {
    let entry = &archive.entries[index];
    let target = root.join(&entry.path);
    let parent = target.parent().unwrap();
    check_directory(parent, true)?;
    check_target(&target, overwrite)?;
    let (temp, file) = temporary(parent)?;
    let mut output = BufWriter::new(file);
    archive.unpack(entry, &mut output)?;
    output.flush()?;
    drop(output);
    if overwrite {
        check_target(&target, true)?;
        fs::rename(&temp.0, &target)?;
    } else {
        // Linking publishes atomically and fails if the destination already exists.
        fs::hard_link(&temp.0, &target)?;
    }
    Ok(())
}
