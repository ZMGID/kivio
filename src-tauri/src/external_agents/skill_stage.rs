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

pub fn stage_active_skill(cwd: &Path, folder_name: &str, source_dir: &Path) -> SkillStagingResult {
    if folder_name.is_empty()
        || folder_name.contains('/')
        || folder_name.contains('\\')
        || folder_name.contains('\0')
    {
        return SkillStagingResult {
            staged: false,
            staged_path: None,
            reason: Some("unsafe folder name".to_string()),
        };
    }

    if !source_dir.is_dir() {
        return SkillStagingResult {
            staged: false,
            staged_path: None,
            reason: Some("source is not a directory".to_string()),
        };
    }

    let alias_root = cwd.join(SKILLS_CWD_ALIAS);
    let staged_path = alias_root.join(folder_name);

    if let Err(err) = fs::create_dir_all(&alias_root) {
        return SkillStagingResult {
            staged: false,
            staged_path: None,
            reason: Some(format!("mkdir alias root: {err}")),
        };
    }

    let _ = fs::remove_dir_all(&staged_path);
    match copy_dir_deref(source_dir, &staged_path) {
        Ok(()) => SkillStagingResult {
            staged: true,
            staged_path: Some(staged_path),
            reason: None,
        },
        Err(err) => SkillStagingResult {
            staged: false,
            staged_path: None,
            reason: Some(err),
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
        let result = stage_active_skill(&cwd, &folder, &src);
        assert!(result.staged);
        let staged = result.staged_path.unwrap();
        assert!(staged.join("SKILL.md").is_file());

        fs::write(staged.join("SKILL.md"), "mutated").unwrap();
        let original = fs::read_to_string(src.join("SKILL.md")).unwrap();
        assert_eq!(original, "hello");

        let _ = fs::remove_dir_all(tmp);
    }
}
