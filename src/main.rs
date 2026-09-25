use std::env;
use std::io::{self, Write};
use std::path::PathBuf;
use stix_unpacker::archive::{Archive, Encoding};
use stix_unpacker::{discovery, output};

const VERSION: &str = concat!(env!("CARGO_PKG_NAME"), " ", env!("CARGO_PKG_VERSION"));

const HELP: &str = "InstallShield 3.x archive unpacker

Usage:
  stix-unpacker [--extract] <input> <output-directory>
  stix-unpacker --test <input>
  stix-unpacker --list <input>

Options:
  -x, --extract       Extract files (the default mode)
  -t, --test          Decompress and validate every file without writing output
  -l, --list          List file paths, sizes, DOS timestamps, and volume paths
      --overwrite    Replace existing regular files when extracting
      --std, --setup Select std* archives and merge their contents in the output
      --encoding E   Filename encoding: cp1251 (default), cp437, or utf-8
  -h, --help          Show the version and this help
  -V, --version       Show the version
  --                 End option parsing

Directory input is scanned recursively for archive headers, regardless of file
extension. Numeric volumes are grouped by basename and sorted by volume number;
disk directory names and physical volume locations do not determine ordering.
Selected groups and catalogs are validated before extraction. Directory input
uses one output subdirectory per archive by default. --std (alias --setup)
selects only archive names starting with std, ignoring case, and uses one shared
output directory. Directory names are merged without regard to case, with
EXTFORMS normalized to ExtForms. File name conflicts are rejected even with
--overwrite. --std/--setup also restrict --list and --test to the selected groups.
An explicit archive file selects only that archive and extracts directly into
the output directory. File symlinks are read; directory symlinks are not followed.
Test mode checks structure, DCL end markers, and sizes; no stored checksum
is known for this format, so it cannot detect every possible byte alteration.
";

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Extract,
    Test,
    List,
}

fn run() -> io::Result<()> {
    let mut args = env::args_os().skip(1).peekable();
    if args.peek().is_none() {
        return show_help();
    }
    let mut mode = None;
    let mut encoding = Encoding::Windows1251;
    let mut overwrite = false;
    let mut std_only = false;
    let mut positional = Vec::new();
    let mut options = true;
    while let Some(arg) = args.next() {
        let text = arg.to_string_lossy();
        if options && text == "--" {
            options = false;
            continue;
        }
        if options && text.starts_with('-') {
            let selected = match text.as_ref() {
                "-h" | "--help" => {
                    return show_help();
                }
                "-V" | "--version" => {
                    return writeln!(io::stdout().lock(), "{VERSION}");
                }
                "-x" | "--extract" => Some(Mode::Extract),
                "-t" | "--test" => Some(Mode::Test),
                "-l" | "--list" => Some(Mode::List),
                "--overwrite" => {
                    overwrite = true;
                    None
                }
                "--std" | "--setup" => {
                    std_only = true;
                    None
                }
                "--encoding" => {
                    let value = args
                        .next()
                        .ok_or_else(|| usage("--encoding requires a value"))?;
                    encoding = Encoding::parse(&value.to_string_lossy())?;
                    None
                }
                _ => return Err(usage(&format!("Unknown option: {text}"))),
            };
            if let Some(selected) = selected {
                if mode.is_some() {
                    return Err(usage("Select only one operation"));
                }
                mode = Some(selected);
            }
        } else {
            positional.push(PathBuf::from(arg));
        }
    }
    let mode = mode.unwrap_or(Mode::Extract);
    let expected = if mode == Mode::Extract { 2 } else { 1 };
    if positional.len() != expected {
        return Err(usage("Incorrect number of positional arguments"));
    }
    if overwrite && mode != Mode::Extract {
        return Err(usage("--overwrite requires extraction mode"));
    }
    let groups = if std_only {
        discovery::discover_std(&positional[0])?
    } else {
        discovery::discover(&positional[0])?
    };
    let mut archives: Vec<_> = groups
        .iter()
        .map(|group| Archive::from_volumes(&group.volumes, encoding))
        .collect::<io::Result<_>>()?;
    if std_only {
        output::merge_directories(&mut archives);
    }
    let total: u64 = archives
        .iter()
        .flat_map(|a| &a.entries)
        .map(|e| e.size)
        .sum();
    let count: usize = archives.iter().map(|a| a.entries.len()).sum();
    let stdout = io::stdout();
    let mut stdout = stdout.lock();
    writeln!(
        stdout,
        "Archive map: {} archives, {} volumes, {count} files, {total} uncompressed bytes",
        archives.len(),
        archives.iter().map(|a| a.volumes.len()).sum::<usize>()
    )?;
    for (group, archive) in groups.iter().zip(&archives) {
        writeln!(
            stdout,
            "Archive {}: {} files, {} volumes, {} bytes compressed, {} bytes uncompressed",
            group.name,
            archive.entries.len(),
            archive.volumes.len(),
            archive.entries.iter().map(|e| e.packed_size).sum::<u64>(),
            archive.entries.iter().map(|e| e.size).sum::<u64>()
        )?;
        for (index, volume) in archive.volumes.iter().enumerate() {
            writeln!(
                stdout,
                "Volume {}: {} ({} data bytes)",
                index + 1,
                volume.path.display(),
                volume.data_size
            )?;
        }
        if mode == Mode::List {
            writeln!(
                stdout,
                "{:>12} {:>12}  {:19}  Path",
                "Size", "Compressed", "Modified (DOS)"
            )?;
            for entry in &archive.entries {
                writeln!(
                    stdout,
                    "{:>12} {:>12}  {}  {}",
                    entry.size,
                    entry.packed_size,
                    entry.timestamp(),
                    entry.path
                )?;
            }
        }
    }
    if mode == Mode::List {
        return Ok(());
    }
    let jobs: Vec<_> = archives
        .iter()
        .zip(&groups)
        .map(|(archive, group)| {
            let root = if mode == Mode::Extract {
                if positional[0].is_dir() && !std_only {
                    positional[1].join(&group.name)
                } else {
                    positional[1].clone()
                }
            } else {
                PathBuf::new()
            };
            (archive, root)
        })
        .collect();
    if mode == Mode::Extract {
        output::preflight_all(&jobs, overwrite)?;
    }
    for ((archive, root), group) in jobs.iter().zip(&groups) {
        if mode == Mode::Extract {
            output::create_directories(archive, root)?;
        }
        for (index, entry) in archive.entries.iter().enumerate() {
            if mode == Mode::Test {
                archive.unpack(entry, io::sink())?;
            } else {
                output::extract_entry(archive, index, root, overwrite)?;
            }
            writeln!(
                stdout,
                "{} {} [{}/{}] {}",
                if mode == Mode::Test {
                    "OK"
                } else {
                    "Extracted"
                },
                group.name,
                index + 1,
                archive.entries.len(),
                entry.path
            )?;
        }
    }
    writeln!(
        stdout,
        "{}: {} archives, {} files, {} bytes",
        if mode == Mode::Test {
            "Test passed"
        } else {
            "Extraction complete"
        },
        archives.len(),
        count,
        total
    )?;
    Ok(())
}

fn show_help() -> io::Result<()> {
    write!(io::stdout().lock(), "{VERSION}\n{HELP}")
}

fn usage(message: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidInput,
        format!("{message}. Run with --help for usage."),
    )
}

fn main() {
    if let Err(error) = run() {
        if error.kind() == io::ErrorKind::BrokenPipe {
            return;
        }
        eprintln!("Error: {error}");
        std::process::exit(1);
    }
}
