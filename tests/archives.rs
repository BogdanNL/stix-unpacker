use std::fs;
use std::io;
use std::process::Command;
use stix_unpacker::archive::{Archive, Encoding};
use stix_unpacker::output;

#[path = "support/mod.rs"]
mod support;
use support::{fixture, Workspace, PLAIN};

#[test]
fn discovers_all_layouts_and_crosses_volume_ten() {
    for nested in [true, false] {
        let workspace = Workspace::new();
        let paths = fixture(&workspace.0, nested, 12);
        for input in [
            &workspace.0,
            &paths[0],
            &paths[0].parent().unwrap().to_path_buf(),
        ] {
            let archive = Archive::open(input, Encoding::Utf8).unwrap();
            assert_eq!(archive.volumes.len(), 12);
            assert_eq!(archive.entries[1].path, "Nested/Folder/second.txt");
            assert_eq!(archive.entries[2].path, "third.txt");
            for entry in &archive.entries {
                let mut bytes = Vec::new();
                archive.unpack(entry, &mut bytes).unwrap();
                assert_eq!(bytes, PLAIN);
            }
        }
    }
}

#[test]
fn discovers_archives_starting_on_later_physical_disks() {
    let workspace = Workspace::new();
    fixture(&workspace.0, true, 3);
    for number in 1..=3 {
        fs::rename(
            workspace.0.join(format!("dIsK{number}")),
            workspace.0.join(format!("dIsK{}", number + 11)),
        )
        .unwrap();
    }
    let first = workspace.0.join("dIsK12/StD.1");
    for input in [&first, first.parent().unwrap()] {
        let archive = Archive::open(input, Encoding::Utf8).unwrap();
        assert_eq!(archive.volumes.len(), 3);
        for entry in &archive.entries {
            let mut output = Vec::new();
            archive.unpack(entry, &mut output).unwrap();
            assert_eq!(output, PLAIN);
        }
    }
    // Do not silently choose between conventional and shifted candidates.
    fs::create_dir(workspace.0.join("DISK2")).unwrap();
    fs::copy(
        workspace.0.join("dIsK13/StD.2"),
        workspace.0.join("DISK2/StD.2"),
    )
    .unwrap();
    let error = Archive::open(&first, Encoding::Utf8).err().unwrap();
    assert!(error
        .to_string()
        .contains("Multiple candidates for volume 2"));
}

#[test]
fn rejects_missing_wrong_and_truncated_volumes() {
    let workspace = Workspace::new();
    let paths = fixture(&workspace.0, true, 12);
    let original = fs::read(&paths[9]).unwrap();
    fs::remove_file(&paths[9]).unwrap();
    let error = Archive::open(&workspace.0, Encoding::Utf8).err().unwrap();
    assert!(error.to_string().contains("Missing volume 10"));
    let mut wrong = original.clone();
    wrong[31] = 9;
    fs::write(&paths[9], wrong).unwrap();
    assert!(Archive::open(&workspace.0, Encoding::Utf8).is_err());
    fs::write(&paths[9], &original[..original.len() - 1]).unwrap();
    assert!(Archive::open(&workspace.0, Encoding::Utf8).is_err());
}

#[test]
fn rejects_inconsistent_catalog_and_ambiguous_first_volume() {
    let workspace = Workspace::new();
    let paths = fixture(&workspace.0, false, 12);
    let mut bytes = fs::read(&paths[1]).unwrap();
    *bytes.last_mut().unwrap() = b'X';
    fs::write(&paths[1], bytes).unwrap();
    assert!(Archive::open(&paths[0], Encoding::Utf8).is_err());
    fs::copy(&paths[0], workspace.0.join("other.1")).unwrap();
    assert!(Archive::open(&workspace.0, Encoding::Utf8).is_err());
}

#[test]
fn extracts_atomically_and_requires_explicit_overwrite() {
    let workspace = Workspace::new();
    let paths = fixture(&workspace.0.join("input"), false, 1);
    let archive = Archive::open(&paths[0], Encoding::Utf8).unwrap();
    let destination = workspace.0.join("output");
    output::preflight(&archive, &destination, false).unwrap();
    output::create_directories(&archive, &destination).unwrap();
    for index in 0..archive.entries.len() {
        output::extract_entry(&archive, index, &destination, false).unwrap();
    }
    assert_eq!(
        fs::read(destination.join("Nested/Folder/second.txt")).unwrap(),
        PLAIN
    );
    assert!(output::preflight(&archive, &destination, false).is_err());
    fs::write(destination.join("first.txt"), b"old").unwrap();
    output::preflight(&archive, &destination, true).unwrap();
    output::extract_entry(&archive, 0, &destination, true).unwrap();
    assert_eq!(fs::read(destination.join("first.txt")).unwrap(), PLAIN);
    // A bad stream must neither replace a good file nor leave a temporary file.
    let mut bytes = fs::read(&paths[0]).unwrap();
    bytes[255] = 2;
    fs::write(&paths[0], bytes).unwrap();
    assert!(output::extract_entry(&archive, 0, &destination, true).is_err());
    assert_eq!(fs::read(destination.join("first.txt")).unwrap(), PLAIN);
    assert!(fs::read_dir(&destination).unwrap().all(|e| !e
        .unwrap()
        .file_name()
        .to_string_lossy()
        .starts_with(".stix-")));
}

#[cfg(unix)]
#[test]
fn refuses_output_symlinks() {
    use std::os::unix::fs::symlink;
    let workspace = Workspace::new();
    let paths = fixture(&workspace.0.join("input"), false, 1);
    let archive = Archive::open(&paths[0], Encoding::Utf8).unwrap();
    let destination = workspace.0.join("output");
    fs::create_dir(&destination).unwrap();
    symlink(&workspace.0, destination.join("Nested")).unwrap();
    assert!(output::preflight(&archive, &destination, true).is_err());
    fs::remove_file(destination.join("Nested")).unwrap();
    symlink(&paths[0], destination.join("first.txt")).unwrap();
    assert!(output::preflight(&archive, &destination, true).is_err());
}

#[test]
fn cli_without_arguments_shows_version_and_help() {
    let binary = env!("CARGO_BIN_EXE_stix-unpacker");
    let result = Command::new(binary).output().unwrap();
    assert!(result.status.success());
    assert!(result.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&result.stdout);
    assert!(stdout.starts_with(&format!("stix-unpacker {}\n", env!("CARGO_PKG_VERSION"))));
    for expected in [
        "Usage:",
        "--extract",
        "--test",
        "--list",
        "--std",
        "--setup",
    ] {
        assert!(stdout.contains(expected));
    }
    for flag in ["--help", "-h"] {
        let help = Command::new(binary).arg(flag).output().unwrap();
        assert!(help.status.success());
        assert!(help.stderr.is_empty());
        assert_eq!(result.stdout, help.stdout);
    }
    let version = Command::new(binary).arg("--version").output().unwrap();
    assert!(version.status.success());
    assert_eq!(
        String::from_utf8_lossy(&version.stdout),
        format!("stix-unpacker {}\n", env!("CARGO_PKG_VERSION"))
    );
}

#[test]
fn cli_modes_and_errors() {
    let workspace = Workspace::new();
    let paths = fixture(&workspace.0.join("input"), true, 12);
    let binary = env!("CARGO_BIN_EXE_stix-unpacker");
    let relative = Command::new(binary)
        .current_dir(paths[0].parent().unwrap())
        .args(["--test", "StD.1"])
        .output()
        .unwrap();
    assert!(
        relative.status.success(),
        "{}",
        String::from_utf8_lossy(&relative.stderr)
    );
    for mode in ["--list", "--test"] {
        let result = Command::new(binary)
            .arg(mode)
            .arg(&paths[0])
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert!(String::from_utf8_lossy(&result.stdout).contains("Nested/Folder/second.txt"));
    }
    let destination = workspace.0.join("output");
    assert!(!destination.exists());
    let result = Command::new(binary)
        .arg("--extract")
        .arg(workspace.0.join("input"))
        .arg(&destination)
        .output()
        .unwrap();
    assert!(result.status.success());
    assert_eq!(fs::read(destination.join("StD/third.txt")).unwrap(), PLAIN);
    for args in [
        vec!["--bad-option"],
        vec!["--list", "--test"],
        vec!["--extract"],
        vec!["--encoding", "unknown"],
    ] {
        let result = Command::new(binary).args(args).output().unwrap();
        assert!(!result.status.success());
        assert!(String::from_utf8_lossy(&result.stderr).starts_with("Error:"));
    }
    let archive = Archive::open(&paths[0], Encoding::Utf8).unwrap();
    archive.unpack(&archive.entries[0], io::sink()).unwrap();
}
