# stix-unpacker

[Русская версия](README.ru.md)

A Rust console utility for extracting single-volume and multi-volume
InstallShield 3.x / The Stirling Compressor archives. It discovers archives by
header signature, groups numbered volumes by basename, and reconstructs files
across volume boundaries. Standalone archives, including `.z` and `.lib`, are
also recognized.

## Usage

`INPUT` is an archive directory or an explicit first-volume file; `OUTPUT` is the
destination directory. These are placeholders for paths you supply.

```sh
stix-unpacker INPUT OUTPUT
stix-unpacker --std INPUT OUTPUT
stix-unpacker --test INPUT
stix-unpacker --list INPUT
```

Running without arguments prints the version and help. Exit status is 0 on
success and 1 on error.

| Option | Description |
| --- | --- |
| `-x`, `--extract` | Extract files; the default operation. Requires `INPUT OUTPUT`. |
| `-t`, `--test` | Decompress and validate archives without writing files. Requires `INPUT`. |
| `-l`, `--list` | Show archive groups, volumes, file paths, sizes, and DOS timestamps. Requires `INPUT`. |
| `--std`, `--setup` | Select only archive names starting with `std`, ignoring case, and merge their contents into `OUTPUT`. Also works with `--test` and `--list`. |
| `--overwrite` | Replace existing regular output files. |
| `--encoding E` | Filename encoding: `cp1251` (default), `cp437`, or `utf-8`. File contents are never transcoded. |
| `-h`, `--help` | Show the version and help. |
| `-V`, `--version` | Show only the version. |
| `--` | End option parsing, allowing paths that begin with `-`. |

Directory input is scanned recursively. Volume locations and physical disk
numbers do not determine their order. Each discovered archive gets its own
output subdirectory by default. An explicit file selects only its archive and
extracts directly into `OUTPUT`; numbered volumes are searched for nearby,
including sibling disk directories when the first volume is in a `DISKN` folder.

With `--std` or `--setup`, selected archives share the destination directory.
Directory names are merged case-insensitively: `EXTFORMS` and `extforms` become
`ExtForms`, including nested paths. Other directories retain their first spelling.
Existing destination directories are not renamed. Conflicting file paths between
archives are rejected before extraction, even with `--overwrite`.

Missing or incompatible volumes are reported as errors. Each output file is
published only after successful decompression. `--test` validates structure,
stream end markers, and sizes; it does not verify stored per-file checksums.
Timestamps are displayed but not restored. Self-extracting EXE wrappers,
encrypted archives, and later InstallShield CAB formats are not supported.

## Third-party sources

The archive layout was researched using **STIX by Veit Kannegieser**, including
its Pascal sources and Linux adaptation. The archive parser is a new Rust
implementation.

- [Original STIX sources: ZIP mirror](https://ecsoft2.org/system/files/repository/stix_src.zip).
- [Original STIX sources: author's ARJ archive](https://kannegieser.net/veit/quelle/stix_src.arj).
- [STIX Linux port](https://github.com/DeclanHoare/stix).

The DCL decoder in `src/explode.rs` is an altered Rust implementation based on
[Mark Adler's blast 1.3](https://github.com/madler/zlib/tree/master/contrib/blast),
including its canonical Huffman tables and decoding algorithm. It is not the
original blast distribution. See the [blast license](https://github.com/madler/zlib/blob/master/contrib/blast/blast.h#L1-L19).
