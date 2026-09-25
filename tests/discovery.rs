// Reuse the compact DCL archive fixture without depending on external installers.
#[path = "support/mod.rs"]
mod support;

use std::fs;
use std::io;
use std::process::Command;
use stix_unpacker::archive::{Archive, Encoding};
use stix_unpacker::discovery::discover;
use support::{fixture, fixture_directory, Workspace, PLAIN};

#[test]
fn scans_headers_and_maps_multiple_archives_independent_of_disk_names() {
    let workspace = Workspace::new();
    let root = &workspace.0;
    let parts = fixture(&root.join("staging"), false, 3);
    for (index, part) in parts.iter().enumerate() {
        let disk = root.join(["DISK99", "misc/deep", "DISK4"][index]);
        fs::create_dir_all(&disk).unwrap();
        fs::rename(part, disk.join(format!("PaYlOaD.{}", index + 1))).unwrap();
    }
    let single = fixture(&root.join("single"), false, 1);
    fs::rename(&single[0], root.join("single/data.Z")).unwrap();
    fs::copy(
        root.join("single/data.Z"),
        root.join("unknown-extension.bin"),
    )
    .unwrap();
    let second = fixture(&root.join("another-set"), false, 2);
    for (index, path) in second.iter().enumerate() {
        fs::rename(path, path.with_file_name(format!("second.{}", index + 1))).unwrap();
    }
    fs::write(root.join("not-an-archive.z"), b"plain text").unwrap();
    fs::write(root.join("not-an-archive.1"), b"plain text").unwrap();
    fs::write(root.join("empty.z"), b"").unwrap();
    let groups = discover(root).unwrap();
    assert_eq!(
        groups.iter().map(|g| g.name.as_str()).collect::<Vec<_>>(),
        ["data.Z", "PaYlOaD", "second", "unknown-extension.bin"]
    );
    assert_eq!(groups[1].volumes.len(), 3);
    assert_eq!(groups[2].volumes.len(), 2);
    for group in groups {
        let archive = Archive::from_volumes(&group.volumes, Encoding::Utf8).unwrap();
        for entry in &archive.entries {
            archive.unpack(entry, io::sink()).unwrap();
        }
    }
}

#[test]
fn rejects_file_directory_conflicts_across_archives() {
    let workspace = Workspace::new();
    let paths = fixture(&workspace.0.join("input"), false, 1);
    let first = Archive::open(&paths[0], Encoding::Utf8).unwrap();
    let mut second = Archive::open(&paths[0], Encoding::Utf8).unwrap();
    for entry in &mut second.entries {
        entry.path = format!("first.txt/{}", entry.path);
    }
    let destination = workspace.0.join("output");
    let result = stix_unpacker::output::preflight_all(
        &[
            (&first, destination.clone()),
            (&second, destination.clone()),
        ],
        true,
    );
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("File/directory conflict"));
    assert!(!destination.exists());
}

#[test]
fn rejects_orphans_gaps_duplicate_parts_and_header_number_mismatch() {
    let workspace = Workspace::new();
    let parts = fixture(&workspace.0, true, 3);
    let original = fs::read(&parts[0]).unwrap();
    fs::remove_file(&parts[0]).unwrap();
    assert!(discover(&workspace.0)
        .unwrap_err()
        .to_string()
        .contains("Missing volume 1"));
    fs::write(&parts[0], original).unwrap();
    fs::rename(&parts[1], parts[1].with_extension("4")).unwrap();
    assert!(discover(&workspace.0)
        .unwrap_err()
        .to_string()
        .contains("Filename/header"));
    fs::rename(parts[1].with_extension("4"), &parts[1]).unwrap();
    let second = fs::read(&parts[1]).unwrap();
    fs::remove_file(&parts[1]).unwrap();
    assert!(discover(&workspace.0)
        .unwrap_err()
        .to_string()
        .contains("Missing volume 2"));
    fs::write(&parts[1], second).unwrap();
    fs::copy(&parts[1], workspace.0.join("STD.2")).unwrap();
    assert!(discover(&workspace.0)
        .unwrap_err()
        .to_string()
        .contains("Multiple candidates"));
}

#[test]
fn cli_maps_all_groups_and_preflights_std_conflicts() {
    let workspace = Workspace::new();
    let input = workspace.0.join("input");
    let parts = fixture(&input, true, 3);
    let single = fixture(&input.join("other"), false, 1);
    let single_path = input.join("other/install.z");
    fs::rename(&single[0], &single_path).unwrap();
    let binary = env!("CARGO_BIN_EXE_stix-unpacker");
    for mode in ["--list", "--test"] {
        let result = Command::new(binary).arg(mode).arg(&input).output().unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let stdout = String::from_utf8_lossy(&result.stdout);
        assert!(stdout.contains("2 archives, 4 volumes, 6 files"));
        assert!(stdout.contains("Archive install.z:"));
        assert!(stdout.contains("Archive StD:"));
    }
    let output = workspace.0.join("grouped");
    let result = Command::new(binary)
        .arg(&input)
        .arg(&output)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    for name in ["StD", "install.z"] {
        assert_eq!(
            fs::read(output.join(name).join("first.txt")).unwrap(),
            PLAIN
        );
    }
    let merged = workspace.0.join("merged");
    let stdcv = single_path.with_file_name("stdcv.z");
    fs::rename(&single_path, &stdcv).unwrap();
    for overwrite in [false, true] {
        let mut command = Command::new(binary);
        command.arg("--std").arg(&input).arg(&merged);
        if overwrite {
            command.arg("--overwrite");
        }
        let result = command.output().unwrap();
        assert!(!result.status.success());
        assert!(String::from_utf8_lossy(&result.stderr).contains("Conflicting output paths"));
        assert!(!merged.exists());
    }
    fs::rename(&stdcv, &single_path).unwrap();
    let removed = Command::new(binary)
        .arg("--flat")
        .arg(&input)
        .arg(&merged)
        .output()
        .unwrap();
    assert!(!removed.status.success());
    assert!(String::from_utf8_lossy(&removed.stderr).contains("Unknown option: --flat"));
    // An explicit file ignores unrelated malformed archives in the scanned tree.
    fs::write(&single_path, [0x13, 0x5d, 0x65, 0x8c]).unwrap();
    let result = Command::new(binary)
        .arg("--test")
        .arg(&parts[0])
        .output()
        .unwrap();
    assert!(result.status.success());
    // Directory extraction fails before writing anything if any recognized header is damaged.
    let result = Command::new(binary)
        .arg(&input)
        .arg(&merged)
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert!(!merged.exists());
}

#[test]
fn std_mode_merges_disjoint_archives_and_preserves_existing_output_on_preflight_failure() {
    let workspace = Workspace::new();
    let input = workspace.0.join("input");
    fixture(&input, true, 3);
    let parts = fixture(&input.join("other"), false, 1);
    let mut bytes = fs::read(&parts[0]).unwrap();
    // Rename the second archive's files without changing record lengths.
    for name in [b"first.txt".as_slice(), b"second.txt", b"third.txt"] {
        let offset = bytes.windows(name.len()).position(|w| w == name).unwrap();
        bytes[offset] = b'x';
    }
    let other = input.join("other/STDcv.z");
    fs::write(&other, bytes).unwrap();
    fs::remove_file(&parts[0]).unwrap();
    let binary = env!("CARGO_BIN_EXE_stix-unpacker");
    let output = workspace.0.join("merged");
    let result = Command::new(binary)
        .arg("--std")
        .arg(&input)
        .arg(&output)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(fs::read(output.join("first.txt")).unwrap(), PLAIN);
    assert_eq!(fs::read(output.join("xirst.txt")).unwrap(), PLAIN);
    let blocked = workspace.0.join("blocked");
    fs::create_dir_all(blocked.join("StD")).unwrap();
    fs::write(blocked.join("StD/third.txt"), b"existing").unwrap();
    let result = Command::new(binary)
        .arg(&input)
        .arg(&blocked)
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert!(!blocked.join("STDcv.z").exists());
    assert_eq!(
        fs::read(blocked.join("StD/third.txt")).unwrap(),
        b"existing"
    );
}

#[test]
fn std_aliases_merge_directory_case_and_skip_unselected_archives() {
    let workspace = Workspace::new();
    let input = workspace.0.join("input");
    fixture_directory(&input, true, 3, "ExtForms\\Calendar");
    let parts = fixture_directory(&input.join("other"), false, 1, "EXTFORMS\\CALENDAR");
    let mut bytes = fs::read(&parts[0]).unwrap();
    for name in [b"first.txt".as_slice(), b"second.txt", b"third.txt"] {
        let offset = bytes.windows(name.len()).position(|w| w == name).unwrap();
        bytes[offset] = b'x';
    }
    let other = parts[0].with_file_name("STDonly.z");
    fs::write(&other, bytes).unwrap();
    fs::remove_file(&parts[0]).unwrap();
    // Neither malformed demo archives nor orphaned non-std volumes affect this selection.
    fs::write(input.join("demo.z"), [0x13, 0x5d, 0x65, 0x8c]).unwrap();
    fs::copy(&other, input.join("other.2")).unwrap();
    let binary = env!("CARGO_BIN_EXE_stix-unpacker");
    for selector in ["--std", "--setup"] {
        let output = workspace.0.join(&selector[2..]);
        let result = Command::new(binary)
            .arg(selector)
            .arg(&input)
            .arg(&output)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert_eq!(
            fs::read(output.join("ExtForms/Calendar/second.txt")).unwrap(),
            PLAIN
        );
        assert_eq!(
            fs::read(output.join("ExtForms/Calendar/xecond.txt")).unwrap(),
            PLAIN
        );
        assert!(!output.join("EXTFORMS").exists());
        assert!(!output.join("ExtForms/CALENDAR").exists());
        assert!(!output.join("StD").exists());
        for mode in ["--list", "--test"] {
            let result = Command::new(binary)
                .args([selector, mode])
                .arg(&input)
                .output()
                .unwrap();
            assert!(result.status.success());
            let text = String::from_utf8_lossy(&result.stdout);
            assert!(text.contains("2 archives, 4 volumes, 6 files"));
            assert!(!text.contains("demo"));
            assert!(!text.contains("EXTFORMS"));
        }
    }
    // A standalone std* archive still uses the canonical ExtForms spelling.
    let output = workspace.0.join("single");
    let result = Command::new(binary)
        .arg("--std")
        .arg(&other)
        .arg(&output)
        .output()
        .unwrap();
    assert!(result.status.success());
    assert_eq!(
        fs::read(output.join("ExtForms/CALENDAR/xecond.txt")).unwrap(),
        PLAIN
    );
}

#[test]
fn std_mode_requires_matching_archives() {
    let workspace = Workspace::new();
    let paths = fixture(&workspace.0.join("input"), false, 1);
    let demo = paths[0].with_file_name("demo.z");
    fs::rename(&paths[0], &demo).unwrap();
    let binary = env!("CARGO_BIN_EXE_stix-unpacker");
    for input in [&demo, demo.parent().unwrap()] {
        let output = workspace.0.join("output");
        let result = Command::new(binary)
            .arg("--std")
            .arg(input)
            .arg(&output)
            .output()
            .unwrap();
        assert!(!result.status.success());
        assert!(String::from_utf8_lossy(&result.stderr).contains("std"));
        assert!(!output.exists());
    }
}
