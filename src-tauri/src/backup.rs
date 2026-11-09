pub mod webdav;

use std::{fs, path::Path};

pub(crate) fn recover_at_startup(root: &Path) -> Result<(), String> {
    if root.exists() {
        return Ok(());
    }
    let name = root
        .file_name()
        .ok_or("Invalid app data directory")?
        .to_string_lossy();
    let rollback = root.with_file_name(format!("{name}.before-restore"));
    if rollback.exists() {
        return fs::rename(rollback, root).map_err(|error| error.to_string());
    }
    if root
        .with_file_name(format!("{name}.restore"))
        .join("app")
        .exists()
    {
        return Err("旧版完全恢复已停用，当前数据目录缺失。请手动找回原数据目录。 / Legacy full restore is disabled and the current data directory is missing. Recover the original data directory manually.".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn startup_only_rolls_back_missing_current_data_and_is_repeatable() {
        for current in [false, true] {
            for rollback in [false, true] {
                for staged in [false, true] {
                    for ready in [false, true] {
                        let temp = tempfile::tempdir().unwrap();
                        let root = temp.path().join("kivio");
                        let old = temp.path().join("kivio.before-restore");
                        let stage = temp.path().join("kivio.restore");
                        fs::create_dir(&stage).unwrap();
                        fs::write(stage.join("marker"), b"preserve-stage").unwrap();
                        if current {
                            fs::create_dir(&root).unwrap();
                            fs::write(root.join("data"), b"current").unwrap();
                        }
                        if rollback {
                            fs::create_dir(&old).unwrap();
                            fs::write(old.join("data"), b"original").unwrap();
                        }
                        if staged {
                            fs::create_dir(stage.join("app")).unwrap();
                            fs::write(stage.join("app/data"), b"foreign").unwrap();
                        }
                        if ready {
                            fs::write(stage.join("ready.json"), b"true").unwrap();
                        }
                        for _ in 0..2 {
                            let result = recover_at_startup(&root);
                            assert_eq!(result.is_err(), !current && !rollback && staged);
                            assert_eq!(root.exists(), current || rollback);
                            if current || rollback {
                                assert_eq!(
                                    fs::read(root.join("data")).unwrap(),
                                    if current {
                                        b"current".as_slice()
                                    } else {
                                        b"original".as_slice()
                                    }
                                );
                            }
                            assert_eq!(old.exists(), current && rollback);
                            if old.exists() {
                                assert_eq!(fs::read(old.join("data")).unwrap(), b"original");
                            }
                            assert_eq!(fs::read(stage.join("marker")).unwrap(), b"preserve-stage");
                            assert_eq!(stage.join("ready.json").exists(), ready);
                            assert_eq!(stage.join("app").exists(), staged);
                            if staged {
                                assert_eq!(fs::read(stage.join("app/data")).unwrap(), b"foreign");
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn fresh_install_stays_missing_until_normal_initialization() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("kivio");
        recover_at_startup(&root).unwrap();
        recover_at_startup(&root).unwrap();
        assert!(!root.exists());
        assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 0);
    }
}
