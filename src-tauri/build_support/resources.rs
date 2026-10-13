use std::{fs, io, path::Path};

/// Tauri copies current resources but leaves deleted files behind. Only prune
/// the app-owned resource directory, after its replacement files were copied.
pub fn prune_stale(source: &Path, destination: &Path) -> io::Result<()> {
    if !destination.exists() {
        return Ok(());
    }
    // Validate the entire trees before deleting anything; never follow links
    // or allow an output directory to contain the source (or vice versa).
    check_tree(source)?;
    check_tree(destination)?;
    let source = source.canonicalize()?;
    let destination = destination.canonicalize()?;
    if source.starts_with(&destination) || destination.starts_with(&source) {
        return Err(io::Error::other("resource source and output overlap"));
    }
    prune_directory(&source, &destination)
}

fn check_tree(path: &Path) -> io::Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    #[cfg(windows)]
    let is_link = {
        use std::os::windows::fs::MetadataExt;
        metadata.file_attributes() & 0x400 != 0 // FILE_ATTRIBUTE_REPARSE_POINT
    };
    #[cfg(not(windows))]
    let is_link = metadata.file_type().is_symlink();
    if is_link || !metadata.is_dir() {
        return Err(io::Error::other(format!(
            "expected an ordinary resource directory: {}",
            path.display()
        )));
    }
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let metadata = fs::symlink_metadata(entry.path())?;
        if metadata.is_dir() || metadata.file_type().is_symlink() {
            check_tree(&entry.path())?;
        }
    }
    Ok(())
}

fn prune_directory(source: &Path, destination: &Path) -> io::Result<()> {
    for entry in fs::read_dir(destination)? {
        let entry = entry?;
        let original = source.join(entry.file_name());
        if !original.exists() {
            if entry.file_type()?.is_dir() {
                fs::remove_dir_all(entry.path())?;
            } else {
                fs::remove_file(entry.path())?;
            }
        } else if entry.file_type()?.is_dir() {
            prune_directory(&original, &entry.path())?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> tempfile::TempDir {
        // Atomic directory creation avoids collisions between parallel tests;
        // wall-clock timestamps are not unique, even at nanosecond precision.
        let root = tempfile::Builder::new()
            .prefix("kivio-resource-test-")
            .tempdir()
            .unwrap();
        fs::create_dir_all(root.path().join("source/current")).unwrap();
        fs::create_dir_all(root.path().join("output/skills/current")).unwrap();
        root
    }

    #[test]
    fn removes_retired_skills_and_nested_files_but_keeps_current_and_siblings() {
        let fixture = fixture();
        let source = fixture.path().join("source");
        let output = fixture.path().join("output/skills");
        fs::write(source.join("current/SKILL.md"), "current").unwrap();
        fs::write(output.join("current/SKILL.md"), "current").unwrap();
        fs::write(output.join("current/obsolete.md"), "old").unwrap();
        fs::create_dir_all(output.join("retired/references")).unwrap();
        fs::write(output.join("retired/references/old.md"), "old").unwrap();
        fs::write(fixture.path().join("output/kivio.exe"), "keep").unwrap();
        prune_stale(&source, &output).unwrap();
        assert_eq!(
            fs::read_to_string(output.join("current/SKILL.md")).unwrap(),
            "current"
        );
        assert!(!output.join("current/obsolete.md").exists());
        assert!(!output.join("retired").exists());
        assert!(fixture.path().join("output/kivio.exe").exists());
        prune_stale(&source, &output).unwrap();
    }

    #[test]
    fn missing_source_and_overlapping_roots_leave_output_intact() {
        let fixture = fixture();
        let output = fixture.path().join("output/skills");
        fs::write(output.join("keep.md"), "keep").unwrap();
        assert!(prune_stale(&fixture.path().join("missing"), &output).is_err());
        assert!(prune_stale(&output, &output).is_err());
        assert!(prune_stale(&output.join("current"), &output).is_err());
        assert!(prune_stale(&output, &output.join("current")).is_err());
        assert!(output.join("keep.md").exists());
    }
}
