use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

pub const SKILLS_CWD_ALIAS: &str = ".kivio/skills-staged";

#[derive(Debug, Clone)]
pub struct SkillStagingResult {
    pub staged: bool,
    pub staged_path: Option<PathBuf>,
    pub reason: Option<String>,
}

pub fn skill_cwd_alias_segment(dir: &str) -> String {
    let folder = Path::new(dir)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("skill");
    let normalized = Path::new(dir)
        .canonicalize()
        .unwrap_or_else(|_| PathBuf::from(dir))
        .to_string_lossy()
        .replace('\\', "/");
    let mut hasher = DefaultHasher::new();
    normalized.hash(&mut hasher);
    let digest = format!("{:016x}", hasher.finish());
    format!("{folder}-{digest}")
}

pub fn stage_active_skill(
    cwd: &Path,
    folder_name: &str,
    source_dir: &Path,
    revision: &str,
) -> SkillStagingResult {
    // First materialization and explicit refresh are rare. Serialize publication
    // so parallel turns never observe or replace each other's partial copy.
    static PUBLISH: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _guard = PUBLISH.lock().unwrap_or_else(|e| e.into_inner());
    let stage = || -> Result<PathBuf, String> {
        if folder_name.is_empty()
            || folder_name == "."
            || folder_name == ".."
            || folder_name.contains('/')
            || folder_name.contains('\\')
            || folder_name.contains('\0')
        {
            return Err("unsafe folder name".into());
        }
        let source = source_dir.canonicalize().map_err(|e| e.to_string())?;
        if !source.is_dir() {
            return Err("source is not a directory".into());
        }
        let alias_root = cwd.join(SKILLS_CWD_ALIAS);
        fs::create_dir_all(&alias_root).map_err(|e| e.to_string())?;
        if alias_root
            .canonicalize()
            .map_err(|e| e.to_string())?
            .starts_with(&source)
        {
            return Err("staging destination is inside the source skill".into());
        }
        let staged = alias_root.join(folder_name);
        let marker = alias_root.join(format!("{folder_name}.revision"));
        let signature = format!("{}\n{revision}", source.display());
        if fs::read_to_string(&marker).ok().as_deref() == Some(&signature)
            && staged.join("SKILL.md").is_file()
        {
            return Ok(staged);
        }
        let temp = alias_root.join(format!(".stage-{}", uuid::Uuid::new_v4()));
        if let Err(error) = copy_dir_deref(&source, &temp) {
            let _ = fs::remove_dir_all(&temp);
            return Err(error);
        }
        let backup = alias_root.join(format!(".old-{}", uuid::Uuid::new_v4()));
        let had_previous = staged.exists();
        if had_previous {
            if let Err(error) = fs::rename(&staged, &backup) {
                let _ = fs::remove_dir_all(&temp);
                return Err(error.to_string());
            }
        }
        if let Err(error) = fs::rename(&temp, &staged) {
            if had_previous {
                let _ = fs::rename(&backup, &staged);
            }
            let _ = fs::remove_dir_all(&temp);
            return Err(error.to_string());
        }
        if let Err(error) =
            crate::chat::storage::atomic_write(&marker, &signature, "skill staging revision")
        {
            fs::remove_dir_all(&staged)
                .map_err(|rollback| format!("{error}; remove uncommitted copy: {rollback}"))?;
            if had_previous {
                fs::rename(&backup, &staged)
                    .map_err(|rollback| format!("{error}; restore previous copy: {rollback}"))?;
            }
            return Err(error);
        }
        let _ = fs::remove_dir_all(backup);
        Ok(staged)
    };
    match stage() {
        Ok(path) => SkillStagingResult {
            staged: true,
            staged_path: Some(path),
            reason: None,
        },
        Err(error) => SkillStagingResult {
            staged: false,
            staged_path: None,
            reason: Some(error),
        },
    }
}

fn copy_dir_deref(src: &Path, dest: &Path) -> Result<(), String> {
    fs::create_dir_all(dest).map_err(|e| e.to_string())?;
    for entry in fs::read_dir(src).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let file_type = entry.file_type().map_err(|e| e.to_string())?;
        let from = entry.path();
        let to = dest.join(entry.file_name());
        if file_type.is_dir() {
            copy_dir_deref(&from, &to)?;
        } else if file_type.is_file() {
            fs::copy(&from, &to).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

pub fn with_skill_root_preamble(body: &str, skill_dir: &str, folder: &str) -> String {
    let rel = format!("{SKILLS_CWD_ALIAS}/{folder}/");
    format!(
        "> **Skill root (relative to project):** `{rel}`\n\
         > **Skill root (absolute fallback):** `{skill_dir}`\n\
         >\n\
         > When the workflow references side files under `assets/` or `references/`, prefer the relative path above.\n\n\
         {body}"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn stage_active_skill_creates_copy_barrier() {
        let tmp = std::env::temp_dir().join(format!("kivio-skill-stage-{}", uuid::Uuid::new_v4()));
        let src = tmp.join("src-skill");
        let cwd = tmp.join("project");
        fs::create_dir_all(&src).unwrap();
        fs::write(src.join("SKILL.md"), "hello").unwrap();
        fs::create_dir_all(&cwd).unwrap();

        let folder = skill_cwd_alias_segment(src.to_str().unwrap());
        let result = stage_active_skill(&cwd, &folder, &src, "v1");
        assert!(result.staged);
        let staged = result.staged_path.unwrap();
        assert!(staged.join("SKILL.md").is_file());

        fs::write(staged.join("SKILL.md"), "mutated").unwrap();
        let original = fs::read_to_string(src.join("SKILL.md")).unwrap();
        assert_eq!(original, "hello");

        let reused = stage_active_skill(&cwd, &folder, &src, "v1");
        assert!(reused.staged);
        assert_eq!(
            fs::read_to_string(staged.join("SKILL.md")).unwrap(),
            "mutated",
            "unchanged staging must preserve the session copy"
        );
        fs::write(src.join("SKILL.md"), "updated source").unwrap();
        let refreshed = stage_active_skill(&cwd, &folder, &src, "v2");
        assert!(refreshed.staged);
        assert_eq!(
            fs::read_to_string(staged.join("SKILL.md")).unwrap(),
            "updated source"
        );
        let failed = stage_active_skill(&cwd, &folder, &tmp.join("missing"), "v3");
        assert!(!failed.staged);
        assert_eq!(
            fs::read_to_string(staged.join("SKILL.md")).unwrap(),
            "updated source"
        );
        assert!(!stage_active_skill(&src, "recursive", &src, "v1").staged);
        assert!(!stage_active_skill(&cwd, "..", &src, "v1").staged);
        // Publication includes its revision: a failed marker commit must leave
        // the last usable copy intact so retry cannot mistake it for another revision.
        let marker = cwd
            .join(SKILLS_CWD_ALIAS)
            .join(format!("{folder}.revision"));
        fs::remove_file(&marker).unwrap();
        fs::create_dir(&marker).unwrap();
        fs::write(src.join("SKILL.md"), "next source").unwrap();
        assert!(!stage_active_skill(&cwd, &folder, &src, "v3").staged);
        assert_eq!(
            fs::read_to_string(staged.join("SKILL.md")).unwrap(),
            "updated source"
        );
        fs::remove_dir(marker).unwrap();
        assert!(stage_active_skill(&cwd, &folder, &src, "v3").staged);
        assert_eq!(
            fs::read_to_string(staged.join("SKILL.md")).unwrap(),
            "next source"
        );
        let _ = fs::remove_dir_all(tmp);
    }
}
