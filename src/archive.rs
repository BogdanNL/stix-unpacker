//! InstallShield 3.x headers and records, researched from STIX by Veit Kannegieser.
//! Original sources: https://ecsoft2.org/system/files/repository/stix_src.zip
//! Linux port: https://github.com/DeclanHoare/stix

use crate::{explode::explode, invalid};
use std::collections::HashSet;
use std::fs::{self, File};
use std::io::{self, BufReader, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

const HEADER_SIZE: u64 = 255;
const SIGNATURE: &[u8] = &[0x13, 0x5d, 0x65, 0x8c];

fn u16_at(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([bytes[offset], bytes[offset + 1]])
}

fn u32_at(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap())
}

#[derive(Clone, Copy, Debug)]
pub enum Encoding {
    Windows1251,
    Cp437,
    Utf8,
}

impl Encoding {
    pub fn parse(name: &str) -> io::Result<Self> {
        match name {
            "cp1251" | "windows-1251" => Ok(Self::Windows1251),
            "cp437" => Ok(Self::Cp437),
            "utf-8" | "utf8" => Ok(Self::Utf8),
            _ => Err(invalid(format!("Unsupported filename encoding: {name}"))),
        }
    }

    fn decode(self, bytes: &[u8]) -> io::Result<String> {
        if matches!(self, Self::Utf8) {
            return String::from_utf8(bytes.to_vec())
                .map_err(|_| invalid("Invalid UTF-8 filename"));
        }
        let table = match self {
            Self::Windows1251 => include_str!("cp1251.txt"),
            Self::Cp437 => include_str!("cp437.txt"),
            Self::Utf8 => unreachable!(),
        };
        let high: Vec<_> = table.chars().collect();
        bytes
            .iter()
            .map(|&b| {
                let c = if b < 128 {
                    b as char
                } else {
                    high[b as usize - 128]
                };
                if c == '\u{fffd}' {
                    Err(invalid("Undefined byte in filename encoding"))
                } else {
                    Ok(c)
                }
            })
            .collect()
    }
}

/// Normalize Windows archive paths without allowing traversal or drive paths.
pub fn safe_path(name: &str, allow_empty: bool) -> io::Result<String> {
    let name = name.replace('\\', "/");
    if name.is_empty() && allow_empty {
        return Ok(name);
    }
    for component in name.split('/') {
        let stem = component
            .split('.')
            .next()
            .unwrap_or("")
            .to_ascii_uppercase();
        let reserved = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
            || ((stem.starts_with("COM") || stem.starts_with("LPT"))
                && stem.len() == 4
                && matches!(stem.as_bytes()[3], b'1'..=b'9'));
        if component.is_empty()
            || component == "."
            || component == ".."
            || component.ends_with(['.', ' '])
            || reserved
            || component
                .chars()
                .any(|c| c.is_control() || ":<>\"|?*".contains(c))
        {
            return Err(invalid(format!("Unsafe archive path: {name:?}")));
        }
    }
    Ok(name)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub path: String,
    pub size: u64,
    pub packed_size: u64,
    pub date: u16,
    pub time: u16,
    pub offset: u64,
}

impl Entry {
    pub fn timestamp(&self) -> String {
        format!(
            "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
            1980 + (self.date >> 9),
            (self.date >> 5) & 15,
            self.date & 31,
            self.time >> 11,
            (self.time >> 5) & 63,
            (self.time & 31) * 2
        )
    }
}

#[derive(Debug)]
pub struct Volume {
    pub path: PathBuf,
    pub data_size: u64,
    pub logical_start: u64,
}

pub struct Archive {
    pub entries: Vec<Entry>,
    pub directories: Vec<String>,
    pub volumes: Vec<Volume>,
}

struct Catalog {
    entries: Vec<Entry>,
    directories: Vec<String>,
    data_size: u64,
    number: u8,
    total_volumes: u8,
    archive_size: u32,
}

fn record(file: &mut File, prefix_size: usize, length_offset: usize) -> io::Result<Vec<u8>> {
    let mut bytes = vec![0; prefix_size];
    file.read_exact(&mut bytes)?;
    let length = u16_at(&bytes, length_offset) as usize;
    if length < prefix_size {
        return Err(invalid("Archive record is shorter than its header"));
    }
    bytes.resize(length, 0);
    file.read_exact(&mut bytes[prefix_size..])?;
    Ok(bytes)
}

fn read_catalog(path: &Path, encoding: Encoding) -> io::Result<Catalog> {
    let mut file = File::open(path)?;
    let mut header = [0; HEADER_SIZE as usize];
    file.read_exact(&mut header)?;
    if &header[..4] != SIGNATURE {
        return Err(invalid(
            "Not an InstallShield 3.x archive (signature not found at offset zero)",
        ));
    }
    let names = u32_at(&header, 41) as u64;
    let file_size = file.metadata()?.len();
    if names < HEADER_SIZE || names > file_size {
        return Err(invalid("Invalid directory table offset"));
    }
    file.seek(SeekFrom::Start(names))?;
    let mut directories = Vec::new();
    let mut counts = Vec::new();
    for _ in 0..u16_at(&header, 49) {
        let bytes = record(&mut file, 6, 2)?;
        let length = u16_at(&bytes, 4) as usize;
        if length >= bytes.len() - 6 || bytes[6 + length] != 0 {
            return Err(invalid("Invalid directory name length or terminator"));
        }
        directories.push(safe_path(&encoding.decode(&bytes[6..6 + length])?, true)?);
        counts.push(u16_at(&bytes, 0) as usize);
    }
    let mut entries = Vec::new();
    let mut offset = 0;
    let mut seen = HashSet::new();
    for _ in 0..u16_at(&header, 12) {
        let bytes = record(&mut file, 30, 23)?;
        let index = u16_at(&bytes, 1) as usize;
        let directory = directories
            .get(index)
            .ok_or_else(|| invalid("Invalid file directory index"))?;
        counts[index] = counts[index]
            .checked_sub(1)
            .ok_or_else(|| invalid("Directory file count mismatch"))?;
        let length = bytes[29] as usize;
        if length == 0 || length > bytes.len() - 30 {
            return Err(invalid("Invalid file name length"));
        }
        let name = safe_path(&encoding.decode(&bytes[30..30 + length])?, false)?;
        if name.contains('/') {
            return Err(invalid("File name contains a directory separator"));
        }
        let path = if directory.is_empty() {
            name
        } else {
            format!("{directory}/{name}")
        };
        if !seen.insert(path.to_lowercase()) {
            return Err(invalid(format!("Duplicate archive path: {path}")));
        }
        let packed_size = u32_at(&bytes, 7) as u64;
        entries.push(Entry {
            path,
            size: u32_at(&bytes, 3) as u64,
            packed_size,
            date: u16_at(&bytes, 15),
            time: u16_at(&bytes, 17),
            offset,
        });
        offset += packed_size;
    }
    if counts.iter().any(|&c| c != 0) {
        return Err(invalid("Directory file count mismatch"));
    }
    if file.stream_position()? != file_size {
        return Err(invalid("Unexpected data after the archive catalog"));
    }
    let expected_archive_size = HEADER_SIZE + offset + file_size - names;
    if u32_at(&header, 18) as u64 != expected_archive_size {
        return Err(invalid(
            "Archive size does not match the catalog and compressed data sizes",
        ));
    }
    // A file cannot also serve as a parent directory of another entry.
    for path in entries.iter().map(|e| &e.path).chain(directories.iter()) {
        let parts: Vec<_> = path.split('/').collect();
        for length in 1..parts.len() {
            if seen.contains(&parts[..length].join("/").to_lowercase()) {
                return Err(invalid("Archive file and directory paths conflict"));
            }
        }
    }
    for directory in &directories {
        if seen.contains(&directory.to_lowercase()) {
            return Err(invalid("Archive file and directory paths conflict"));
        }
    }
    Ok(Catalog {
        entries,
        directories,
        data_size: names - HEADER_SIZE,
        number: header[31],
        total_volumes: header[30],
        archive_size: u32_at(&header, 18),
    })
}

fn child_case_insensitive(directory: &Path, name: &str) -> io::Result<Option<PathBuf>> {
    let mut matches = Vec::new();
    for item in fs::read_dir(directory)? {
        let item = item?;
        if item
            .file_name()
            .to_string_lossy()
            .eq_ignore_ascii_case(name)
        {
            matches.push(item.path());
        }
    }
    if matches.len() > 1 {
        return Err(invalid(format!(
            "Ambiguous name {name:?} in {}",
            directory.display()
        )));
    }
    Ok(matches.pop())
}

fn numbered_first(directory: &Path) -> io::Result<Vec<PathBuf>> {
    let mut paths = Vec::new();
    for item in fs::read_dir(directory)? {
        let path = item?.path();
        if path.is_file() && path.extension().is_some_and(|e| e == "1") {
            paths.push(path);
        }
    }
    Ok(paths)
}

pub fn find_first(input: &Path) -> io::Result<PathBuf> {
    if input.is_file() {
        if let Some(extension) = input.extension().and_then(|e| e.to_str()) {
            if let Ok(number) = extension.parse::<u32>() {
                if number != 1 {
                    return Err(invalid("Pass the first archive volume (.1)"));
                }
            }
        }
        return Ok(input.to_path_buf());
    }
    if !input.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("Input not found: {}", input.display()),
        ));
    }
    let mut candidates = numbered_first(input)?;
    if let Some(disk1) = child_case_insensitive(input, "DISK1")? {
        if disk1.is_dir() {
            candidates.extend(numbered_first(&disk1)?);
        }
    }
    candidates.sort();
    match candidates.len() {
        1 => Ok(candidates.remove(0)),
        0 => Err(invalid(format!("No first volume (*.1) found in {} or its DISK1 directory; pass an explicit archive file for .z/.lib archives", input.display()))),
        _ => Err(invalid("Multiple first volumes found; pass the desired .1 file explicitly")),
    }
}

impl Archive {
    pub fn open(input: &Path, encoding: Encoding) -> io::Result<Self> {
        let first = find_first(input)?;
        let groups = crate::discovery::discover(&first)?;
        Self::from_volumes(&groups[0].volumes, encoding)
    }

    /// Validate every mapped volume and catalog before any data is extracted.
    pub fn from_volumes(paths: &[PathBuf], encoding: Encoding) -> io::Result<Self> {
        let first = paths.first().ok_or_else(|| invalid("Empty volume map"))?;
        let catalog = read_catalog(first, encoding)
            .map_err(|e| invalid(format!("{}: {e}", first.display())))?;
        if catalog.number > 1 {
            return Err(invalid(
                "The archive header identifies a continuation volume",
            ));
        }
        let packed: u64 = catalog.entries.iter().map(|e| e.packed_size).sum();
        let mut volumes = Vec::new();
        let mut length = 0;
        for (index, path) in paths.iter().enumerate() {
            let part = if index == 0 {
                None
            } else {
                Some(
                    read_catalog(path, encoding)
                        .map_err(|e| invalid(format!("{}: {e}", path.display())))?,
                )
            };
            let part = part.as_ref().unwrap_or(&catalog);
            let number = index + 1;
            if (part.number != 0 && part.number as usize != number)
                || (part.total_volumes != 0 && part.total_volumes as usize != paths.len())
                || part.entries != catalog.entries
                || part.directories != catalog.directories
                || part.archive_size != catalog.archive_size
            {
                return Err(invalid(format!("Volume {number} does not belong to this archive or has an inconsistent volume count: {}", path.display())));
            }
            if part.data_size == 0 && packed != 0 {
                return Err(invalid(format!("Volume {number} has an empty data area")));
            }
            volumes.push(Volume {
                path: path.clone(),
                logical_start: length,
                data_size: part.data_size,
            });
            length += part.data_size;
        }
        if length != packed {
            return Err(invalid(
                "Archive payload size does not match the file catalog",
            ));
        }
        Ok(Self {
            entries: catalog.entries,
            directories: catalog.directories,
            volumes,
        })
    }

    pub fn unpack<W: Write>(&self, entry: &Entry, output: W) -> io::Result<()> {
        let mut reader = VolumeReader::new(&self.volumes, entry.offset, entry.packed_size)?;
        let consumed = explode(&mut reader, output, entry.size)
            .map_err(|e| invalid(format!("{}: {e}", entry.path)))?;
        if consumed != entry.packed_size {
            return Err(invalid(format!(
                "{}: trailing bytes after the DCL end marker",
                entry.path
            )));
        }
        Ok(())
    }
}

struct VolumeReader<'a> {
    volumes: &'a [Volume],
    index: usize,
    file: BufReader<File>,
    part_left: u64,
    left: u64,
}

impl<'a> VolumeReader<'a> {
    fn new(volumes: &'a [Volume], offset: u64, length: u64) -> io::Result<Self> {
        let index = volumes
            .iter()
            .position(|v| offset >= v.logical_start && offset < v.logical_start + v.data_size)
            .ok_or_else(|| invalid("File offset is outside the volume data"))?;
        let volume = &volumes[index];
        let within = offset - volume.logical_start;
        let mut file = File::open(&volume.path)?;
        file.seek(SeekFrom::Start(HEADER_SIZE + within))?;
        Ok(Self {
            volumes,
            index,
            file: BufReader::new(file),
            part_left: volume.data_size - within,
            left: length,
        })
    }
}

impl Read for VolumeReader<'_> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if self.left == 0 || buffer.is_empty() {
            return Ok(0);
        }
        if self.part_left == 0 {
            self.index += 1;
            let volume = self
                .volumes
                .get(self.index)
                .ok_or_else(|| invalid("Compressed file extends beyond the final volume"))?;
            let mut file = File::open(&volume.path)?;
            file.seek(SeekFrom::Start(HEADER_SIZE))?;
            self.file = BufReader::new(file);
            self.part_left = volume.data_size;
        }
        let length = buffer.len().min(self.left.min(self.part_left) as usize);
        let count = self.file.read(&mut buffer[..length])?;
        if count == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "Truncated volume data",
            ));
        }
        self.left -= count as u64;
        self.part_left -= count as u64;
        Ok(count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_unsafe_paths() {
        for name in [
            "",
            "../evil",
            "a/../evil",
            "/absolute",
            "C:\\evil",
            "\\\\host\\file",
            "a//b",
            "a\0b",
            "a\nb",
            "NUL.txt",
            "a.",
        ] {
            assert!(safe_path(name, false).is_err(), "{name:?}");
        }
        assert_eq!(
            safe_path("ExtForms\\Calendar\\file.txt", false).unwrap(),
            "ExtForms/Calendar/file.txt"
        );
    }

    #[test]
    fn filename_encodings() {
        assert_eq!(
            Encoding::Windows1251
                .decode(&[0xd2, 0xe5, 0xf1, 0xf2])
                .unwrap(),
            "\u{422}\u{435}\u{441}\u{442}"
        );
        assert_eq!(Encoding::Cp437.decode(&[0x82]).unwrap(), "\u{e9}");
        assert!(Encoding::Utf8.decode(&[0xff]).is_err());
    }
}
