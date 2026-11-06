use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde::Serialize;
use super::spawn::MergeMode;

fn git(repo: &Path, args: &[&str]) -> Result<Output, String> {
    let output = command(repo).args(args).output().map_err(|e| format!("Cannot run Git: {e}"))?;
    if output.status.success() { Ok(output) } else { Err(format!("Git {} failed: {}", args.first().copied().unwrap_or(""), String::from_utf8_lossy(&output.stderr).trim())) }
}

fn command(repo: &Path) -> Command {
    let mut command = Command::new("git");
    command.arg("-C").arg(crate::utils::strip_windows_verbatim_prefix(repo.to_path_buf())).env("GIT_TERMINAL_PROMPT", "0");
    #[cfg(windows)]
    { use std::os::windows::process::CommandExt; command.creation_flags(0x08000000); }
    command
}

fn git_text(repo: &Path, args: &[&str]) -> Result<String, String> {
    String::from_utf8(git(repo, args)?.stdout).map(|text| text.trim().to_string()).map_err(|_| "Git returned non-UTF-8 metadata.".into())
}

fn write_recovery(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut file = fs::OpenOptions::new().write(true).create(true).truncate(true).open(path)
        .map_err(|e| format!("Cannot write recovery artifact {}: {e}", path.display()))?;
    file.write_all(bytes).and_then(|_| file.sync_all()).map_err(|e| format!("Cannot save recovery artifact {}: {e}", path.display()))
}

fn snapshot(repo: &Path, storage: &Path, parent: &str, gitlinks: &[(String, String)]) -> Result<String, String> {
    let index = storage.join(format!("index-{}", uuid::Uuid::new_v4()));
    let result = (|| {
        let run_index = |args: &[&str]| -> Result<Output, String> {
            let output = command(repo).env("GIT_INDEX_FILE", crate::utils::strip_windows_verbatim_prefix(index.clone())).args(args).output().map_err(|e| format!("Cannot snapshot isolated files: {e}"))?;
            if output.status.success() { Ok(output) } else { Err(format!("Cannot snapshot isolated files: {}", String::from_utf8_lossy(&output.stderr))) }
        };
        run_index(&["read-tree", parent])?;
        run_index(&["add", "--all", "--", "."])?;
        for (path, object) in gitlinks { run_index(&["update-index", "--add", "--cacheinfo", &format!("160000,{object},{path}")])?; }
        let tree = String::from_utf8(run_index(&["write-tree"])?.stdout).map_err(|_| "Invalid Git tree metadata.")?;
        let output = command(repo).args(["-c", "user.name=Kivio", "-c", "user.email=kivio@localhost", "commit-tree", tree.trim(), "-p", parent, "-m", "Kivio isolated snapshot"])
            .output().map_err(|e| format!("Cannot commit isolated snapshot: {e}"))?;
        if !output.status.success() { return Err(format!("Cannot commit isolated snapshot: {}", String::from_utf8_lossy(&output.stderr))); }
        String::from_utf8(output.stdout).map(|text| text.trim().to_string()).map_err(|_| "Invalid Git snapshot metadata.".into())
    })();
    let _ = fs::remove_file(index);
    result
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct IsolationOutcome {
    pub applied: bool,
    pub recovery_directory: PathBuf,
    pub patch_file: PathBuf,
    pub branch: Option<String>,
    pub nested: Vec<IsolationOutcome>,
}

pub(crate) struct IsolationLease {
    parent: PathBuf,
    directory: PathBuf,
    lease_root: PathBuf,
    cwd: PathBuf,
    baseline: String,
    gitlinks: Vec<(String, String)>,
    nested: Vec<IsolationLease>,
    cleaned: bool,
}

impl IsolationLease {
    pub(crate) fn prepare(parent: &Path, artifacts: &Path) -> Result<Self, String> {
        let parent = fs::canonicalize(parent).map_err(|e| format!("Cannot resolve isolation directory: {e}"))?;
        let top = PathBuf::from(git_text(&parent, &["rev-parse", "--show-toplevel"])?);
        let top = fs::canonicalize(top).map_err(|e| format!("Cannot resolve Git root: {e}"))?;
        if top != parent { return Err("Isolation requires the repository root as its working directory.".into()); }
        fs::create_dir_all(artifacts).map_err(|e| format!("Cannot create isolation artifacts: {e}"))?;
        let artifacts = fs::canonicalize(artifacts).map_err(|e| format!("Cannot resolve isolation artifacts: {e}"))?;
        if artifacts.starts_with(&parent) { return Err("Isolation recovery artifacts must live outside the source repository.".into()); }
        let directory = artifacts.join(format!("isolation-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&directory).map_err(|e| format!("Cannot create isolation directory: {e}"))?;
        let cwd = directory.join("worktree");
        Self::prepare_at(parent, directory.clone(), cwd, directory)
    }

    fn prepare_at(parent: PathBuf, directory: PathBuf, cwd: PathBuf, lease_root: PathBuf) -> Result<Self, String> {
        let head = git_text(&parent, &["rev-parse", "HEAD"])?;
        let gitlinks = gitlinks(&parent)?;
        let baseline = snapshot(&parent, &directory, &head, &gitlinks)?;
        git(&parent, &["worktree", "add", "--detach", &crate::utils::strip_windows_verbatim_prefix(cwd.clone()).to_string_lossy(), &baseline])?;
        let mut lease = Self { parent, directory, lease_root, cwd, baseline, gitlinks, nested: Vec::new(), cleaned: false };
        write_recovery(&lease.directory.join("recovery.json"), &serde_json::to_vec_pretty(&serde_json::json!({"parent":lease.parent,"worktree":lease.cwd,"baseline":lease.baseline,"state":"prepared"})).map_err(|e| e.to_string())?)?;
        for (relative, _) in &lease.gitlinks {
            let parent_nested = lease.parent.join(relative);
            if !parent_nested.join(".git").exists() { continue; }
            let nested_cwd = lease.cwd.join(relative);
            if nested_cwd.exists() { fs::remove_dir(&nested_cwd).map_err(|e| format!("Cannot prepare empty nested worktree: {e}"))?; }
            if let Some(parent) = nested_cwd.parent() { fs::create_dir_all(parent).map_err(|e| e.to_string())?; }
            let nested_directory = lease.directory.join(format!("nested-{}", uuid::Uuid::new_v4()));
            fs::create_dir(&nested_directory).map_err(|e| e.to_string())?;
            lease.nested.push(Self::prepare_at(parent_nested, nested_directory, nested_cwd, lease.lease_root.clone())?);
        }
        Ok(lease)
    }

    pub(crate) fn cwd(&self) -> &Path { &self.cwd }

    pub(crate) fn recovery_directory(&self) -> &Path { &self.directory }

    pub(crate) fn finish(&mut self, mode: MergeMode, apply: bool, cancelled: bool) -> Result<IsolationOutcome, String> {
        let mut nested = Vec::new();
        for lease in &mut self.nested { nested.push(lease.finish(mode, apply, cancelled)?); }
        let result = self.capture_and_apply(mode, apply && !cancelled);
        let mut outcome = result.map_err(|error| format!("{error} Recovery artifacts retained at {}.", self.directory.display()))?;
        outcome.nested = nested;
        if apply && !cancelled {
            self.cleanup()?;
        }
        Ok(outcome)
    }

    fn capture_and_apply(&self, mode: MergeMode, apply: bool) -> Result<IsolationOutcome, String> {
        let child = snapshot(&self.cwd, &self.directory, &self.baseline, &self.gitlinks)?;
        let patch_file = self.directory.join("changes.patch");
        let patch = git(&self.parent, &["diff", "--binary", "--full-index", &self.baseline, &child])?.stdout;
        write_recovery(&patch_file, &patch)?;
        let branch = if mode == MergeMode::Branch {
            let branch = format!("codex/omp-isolation-{}", uuid::Uuid::new_v4());
            git(&self.parent, &["update-ref", &format!("refs/heads/{branch}"), &child, "0000000000000000000000000000000000000000"])?;
            Some(branch)
        } else { None };
        write_recovery(&self.directory.join("recovery.json"), &serde_json::to_vec_pretty(&serde_json::json!({"parent":self.parent,"worktree":self.cwd,"baseline":self.baseline,"child":child,"patch":patch_file,"branch":branch,"state":"captured"})).map_err(|e| e.to_string())?)?;
        if apply && !patch.is_empty() {
            let common = PathBuf::from(git_text(&self.parent, &["rev-parse", "--path-format=absolute", "--git-common-dir"])?);
            let lock_path = common.join("kivio-isolation-merge.lock");
            let lock = fs::OpenOptions::new().write(true).create_new(true).open(&lock_path)
                .map_err(|e| format!("Cannot acquire isolation merge lease: {e}"))?;
            let result: Result<(), String> = (|| {
                let parent = snapshot(&self.parent, &self.directory, &self.baseline, &self.gitlinks)?;
                let apply_file = if mode == MergeMode::Branch {
                    let merged = git_text(&self.parent, &["merge-tree", "--write-tree", &parent, &child])?;
                    let tree = merged.lines().next().ok_or("Git merge produced no tree.")?;
                    let delta = git(&self.parent, &["diff", "--binary", "--full-index", &parent, tree])?.stdout;
                    let file = self.directory.join("merged.patch");
                    write_recovery(&file, &delta)?;
                    file
                } else { patch_file.clone() };
                if fs::metadata(&apply_file).map_err(|e| e.to_string())?.len() != 0 {
                    git(&self.parent, &["apply", "--check", "--binary", &crate::utils::strip_windows_verbatim_prefix(apply_file.clone()).to_string_lossy()])?;
                    let current = snapshot(&self.parent, &self.directory, &self.baseline, &self.gitlinks)?;
                    let parent_tree = git_text(&self.parent, &["rev-parse", &format!("{parent}^{{tree}}")])?;
                    let current_tree = git_text(&self.parent, &["rev-parse", &format!("{current}^{{tree}}")])?;
                    if parent_tree != current_tree { return Err("Parent files changed while preparing the isolated merge.".into()); }
                    git(&self.parent, &["apply", "--binary", &crate::utils::strip_windows_verbatim_prefix(apply_file.clone()).to_string_lossy()])?;
                }
                Ok(())
            })();
            drop(lock);
            let unlock = fs::remove_file(&lock_path).map_err(|e| format!("Cannot release isolation merge lease: {e}"));
            result?;
            unlock?;
        }
        Ok(IsolationOutcome { applied: apply, recovery_directory: self.directory.clone(), patch_file, branch, nested: Vec::new() })
    }

    pub(crate) fn cleanup(&mut self) -> Result<(), String> {
        if self.cleaned { return Ok(()); }
        if !self.directory.join("changes.patch").is_file() { return Err("Isolation cleanup requires a captured recovery patch.".into()); }
        for nested in &mut self.nested { nested.cleanup()?; }
        let canonical = fs::canonicalize(&self.cwd).map_err(|e| format!("Cannot verify isolation cleanup target: {e}"))?;
        let root = fs::canonicalize(&self.lease_root).map_err(|e| format!("Cannot verify isolation directory: {e}"))?;
        if canonical == self.parent || !canonical.starts_with(&root) || canonical == root {
            return Err("Refusing to remove a worktree outside its isolation lease.".into());
        }
        git(&self.parent, &["worktree", "remove", "--force", &crate::utils::strip_windows_verbatim_prefix(self.cwd.clone()).to_string_lossy()])?;
        self.cleaned = true;
        Ok(())
    }
}

fn gitlinks(repo: &Path) -> Result<Vec<(String, String)>, String> {
    let bytes = git(repo, &["ls-files", "--stage", "-z"])?.stdout;
    let mut links = Vec::new();
    for entry in bytes.split(|byte| *byte == 0).filter(|entry| !entry.is_empty()) {
        let text = std::str::from_utf8(entry).map_err(|_| "Nested Git paths must be valid UTF-8.")?;
        let Some((metadata, path)) = text.split_once('\t') else { return Err("Invalid Git index entry.".into()); };
        let mut fields = metadata.split_whitespace();
        if fields.next() == Some("160000") {
            let object = fields.next().ok_or("Invalid nested Git object.")?;
            if Path::new(path).is_absolute() || Path::new(path).components().any(|part| matches!(part, std::path::Component::ParentDir)) { return Err("Unsafe nested repository path.".into()); }
            links.push((path.to_string(), object.to_string()));
        }
    }
    Ok(links)
}

impl Drop for IsolationLease {
    fn drop(&mut self) {
        if !self.cleaned && self.cwd.exists() && !self.directory.join("changes.patch").is_file() {
            if let Err(error) = self.capture_and_apply(MergeMode::Patch, false) {
                eprintln!("Could not capture interrupted isolation at {}: {error}", self.directory.display());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repository() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        git(dir.path(), &["init", "-q"]).unwrap();
        git(dir.path(), &["config", "user.name", "Test"]).unwrap();
        git(dir.path(), &["config", "user.email", "test@localhost"]).unwrap();
        git(dir.path(), &["config", "core.autocrlf", "false"]).unwrap();
        fs::write(dir.path().join("a.txt"), "one\ntwo\nthree\n").unwrap();
        fs::write(dir.path().join("delete.bin"), [0, 1, 2, 255]).unwrap();
        git(dir.path(), &["add", "."]).unwrap();
        git(dir.path(), &["commit", "-qm", "initial"]).unwrap();
        dir
    }

    #[test]
    fn dirty_parent_binary_new_deleted_and_index_are_preserved() {
        let repo = repository();
        let recovery = tempfile::tempdir().unwrap();
        fs::write(repo.path().join("a.txt"), "dirty parent\ntwo\nthree\n").unwrap();
        git(repo.path(), &["add", "a.txt"]).unwrap();
        fs::write(repo.path().join("untracked.txt"), "parent draft").unwrap();
        let index = git(repo.path(), &["diff", "--cached", "--binary"]).unwrap().stdout;
        let mut lease = IsolationLease::prepare(repo.path(), recovery.path()).unwrap();
        assert_eq!(fs::read_to_string(lease.cwd().join("untracked.txt")).unwrap(), "parent draft");
        fs::write(lease.cwd().join("a.txt"), "dirty parent\nchild edit\nthree\n").unwrap();
        fs::write(lease.cwd().join("new.bin"), [0, 255, 7, 0]).unwrap();
        fs::remove_file(lease.cwd().join("delete.bin")).unwrap();
        let outcome = lease.finish(MergeMode::Patch, true, false).unwrap();
        assert!(outcome.applied);
        assert_eq!(fs::read(repo.path().join("new.bin")).unwrap(), [0, 255, 7, 0]);
        assert!(!repo.path().join("delete.bin").exists());
        assert_eq!(fs::read_to_string(repo.path().join("a.txt")).unwrap(), "dirty parent\nchild edit\nthree\n");
        assert_eq!(git(repo.path(), &["diff", "--cached", "--binary"]).unwrap().stdout, index);
        assert!(!lease.cwd().exists());
        assert!(outcome.patch_file.is_file());
    }

    #[test]
    fn conflict_and_cancel_retain_recovery_without_overwriting_parent() {
        for cancelled in [false, true] {
            let repo = repository();
            let recovery = tempfile::tempdir().unwrap();
            let mut lease = IsolationLease::prepare(repo.path(), recovery.path()).unwrap();
            fs::write(lease.cwd().join("a.txt"), "child\n").unwrap();
            fs::write(repo.path().join("a.txt"), "parent\n").unwrap();
            let result = lease.finish(MergeMode::Patch, true, cancelled);
            assert_eq!(result.is_ok(), cancelled);
            assert_eq!(fs::read_to_string(repo.path().join("a.txt")).unwrap(), "parent\n");
            assert!(lease.cwd().exists());
            assert!(lease.recovery_directory().join("changes.patch").is_file());
            lease.cleanup().unwrap();
        }
    }

    #[test]
    fn branch_mode_merges_nonoverlapping_parent_changes_and_keeps_recovery_branch() {
        let repo = repository();
        let recovery = tempfile::tempdir().unwrap();
        let mut lease = IsolationLease::prepare(repo.path(), recovery.path()).unwrap();
        fs::write(lease.cwd().join("child.txt"), "child").unwrap();
        fs::write(repo.path().join("parent.txt"), "parent").unwrap();
        let head = git_text(repo.path(), &["rev-parse", "HEAD"]).unwrap();
        let outcome = lease.finish(MergeMode::Branch, true, false).unwrap();
        assert_eq!(fs::read_to_string(repo.path().join("parent.txt")).unwrap(), "parent");
        assert_eq!(fs::read_to_string(repo.path().join("child.txt")).unwrap(), "child");
        assert_eq!(git_text(repo.path(), &["rev-parse", "HEAD"]).unwrap(), head);
        git(repo.path(), &["rev-parse", outcome.branch.as_deref().unwrap()]).unwrap();
    }

    #[test]
    fn nested_git_repositories_apply_their_own_dirty_delta() {
        let repo = repository();
        let nested = repository();
        let nested_path = repo.path().join("nested");
        git(repo.path(), &["-c", "protocol.file.allow=always", "submodule", "add", &nested.path().to_string_lossy(), "nested"]).unwrap();
        git(repo.path(), &["commit", "-qam", "nested repository"]).unwrap();
        git(&nested_path, &["config", "core.autocrlf", "false"]).unwrap();
        fs::write(nested_path.join("a.txt"), "nested dirty baseline\n").unwrap();
        let recovery = tempfile::tempdir().unwrap();
        let mut lease = IsolationLease::prepare(repo.path(), recovery.path()).unwrap();
        assert_eq!(fs::read_to_string(lease.cwd().join("nested/a.txt")).unwrap(), "nested dirty baseline\n");
        fs::write(lease.cwd().join("nested/a.txt"), "nested child\n").unwrap();
        let outcome = lease.finish(MergeMode::Patch, true, false).unwrap();
        assert_eq!(outcome.nested.len(), 1);
        assert_eq!(fs::read_to_string(nested_path.join("a.txt")).unwrap(), "nested child\n");
        assert!(!lease.cwd().exists());
    }

    #[test]
    fn retaining_branch_changes_keeps_the_recovery_reference_after_drop() {
        let repo = repository();
        let recovery = tempfile::tempdir().unwrap();
        let mut lease = IsolationLease::prepare(repo.path(), recovery.path()).unwrap();
        fs::write(lease.cwd().join("new.txt"), "child").unwrap();
        let outcome = lease.finish(MergeMode::Branch, false, false).unwrap();
        let metadata = outcome.recovery_directory.join("recovery.json");
        drop(lease);
        let saved: serde_json::Value = serde_json::from_slice(&fs::read(metadata).unwrap()).unwrap();
        assert_eq!(saved["branch"].as_str(), outcome.branch.as_deref());
        git(repo.path(), &["worktree", "remove", "--force", &outcome.recovery_directory.join("worktree").to_string_lossy()]).unwrap();
    }
}
