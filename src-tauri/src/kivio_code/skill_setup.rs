//! Skill integration seam for kivio-code.
//!
//! This module owns ALL skill wiring for the headless CLI so skill support can
//! be built out without touching `kivio_code/mod.rs`. `mod.rs` only calls
//! [`build_skill_registry`] when constructing a `TurnAssembly` and
//! [`skill_tool_definitions`] when assembling the per-turn tool set.
//!
//! It mirrors the desktop chat app's skill path:
//! - Discovery: user app-data skills dir + bundled built-ins + the user's
//!   configured `chat_tools.skill_scan_paths` (same sources as the GUI's
//!   `skills::build_registry`), but resolved WITHOUT a Tauri `AppHandle` via
//!   `skills::build_registry_headless`.
//! - Settings gating: skills disabled in Settings (`disabled_skill_ids`) are
//!   dropped from the registry, exactly like the GUI's available-skills catalog
//!   (`chat::commands` filters with `is_skill_enabled`).
//! - Activation tool: when the registry is non-empty AND the skill runtime is
//!   enabled in Settings (`native_tools.skill_runtime`, gated by
//!   `mcp::registry::skill_runtime_tools_enabled`), expose the same `skill`
//!   tool the GUI's `mcp::registry::list_skill_tool_defs` adds, so the agent
//!   loop can advertise and activate skills mid-run.
use crate::mcp::types::native_skill_tools;
use crate::mcp::ChatToolDefinition;
use crate::settings::{is_skill_enabled, Settings};
use crate::skills::{self, SkillRegistry};
use std::path::Path;

/// Discover skills into a registry, headless, including PROJECT-level skills
/// rooted at `cwd`. Mirrors the GUI's `skills::build_registry` sources (user
/// app-data dir + bundled built-ins + configured `chat_tools.skill_scan_paths`)
/// and additionally scans, per the kivio-code project conventions:
///   - `<cwd>/.kivio/skills` — the native Kivio project skills dir (always, if
///     it exists).
///   - `<cwd>/.claude/skills` — Claude Code-compatible project skills, ONLY when
///     the user's `read_claude_dir` toggle (flipped via `/settings`) is on. This
///     is the same toggle that gates reading `.claude/CLAUDE.md`.
///
/// Reads the `read_claude_dir` toggle from `kivio_code::config` and delegates to
/// [`build_skill_registry_with`] (which takes the bool explicitly, so the
/// root-composition + precedence logic is unit-testable without touching the
/// machine-global config file).
pub fn build_skill_registry(settings: &Settings, cwd: &Path) -> SkillRegistry {
    let read_claude = super::config::load_merged(cwd).read_claude_dir;
    build_skill_registry_with(settings, cwd, read_claude)
}

/// Project skill roots, in PROJECT-PRECEDENCE order. See
/// [`build_skill_registry_with`] for how precedence is enforced. Only roots that
/// exist as directories are returned (we never hand non-existent paths to the
/// scanner).
fn project_skill_roots(cwd: &Path, read_claude: bool) -> Vec<String> {
    let mut roots: Vec<String> = Vec::new();
    let kivio_dir = cwd.join(".kivio").join("skills");
    if kivio_dir.is_dir() {
        roots.push(kivio_dir.to_string_lossy().into_owned());
    }
    if read_claude {
        let claude_dir = cwd.join(".claude").join("skills");
        if claude_dir.is_dir() {
            roots.push(claude_dir.to_string_lossy().into_owned());
        }
    }
    roots
}

/// Testable core of [`build_skill_registry`] with the Claude-Code toggle passed
/// explicitly.
///
/// Precedence: a PROJECT skill must win over a global/built-in skill with the
/// same id. `skills::build_registry_headless` always scans built-in + user roots
/// BEFORE the `extra_paths` we hand it, and its dedup is KEEPS-FIRST
/// (`dedup_records` in `discover.rs` inserts the first record per id and shadows
/// later duplicates). So we cannot win on collision merely by ordering
/// `extra_paths`. Instead we build TWO registries — one from project roots only,
/// one from the global sources — and merge with the project copy taking priority
/// on id collision. Configured `chat_tools.skill_scan_paths` remain global-tier.
fn build_skill_registry_with(settings: &Settings, cwd: &Path, read_claude: bool) -> SkillRegistry {
    let project_roots = project_skill_roots(cwd, read_claude);

    // Project-only registry: authoritative source of which ids the project owns.
    let project = skills::build_registry_headless(&project_roots);
    // Global registry: built-ins + user dir + configured scan paths.
    let global = skills::build_registry_headless(&settings.chat_tools.skill_scan_paths);

    // Merge project-wins: take all project records, then global records whose id
    // is not already provided by the project.
    let project_ids: std::collections::HashSet<String> =
        project.records.iter().map(|r| r.meta.id.clone()).collect();
    let mut registry = project;
    registry.warnings.extend(global.warnings);
    for record in global.records {
        if !project_ids.contains(&record.meta.id) {
            registry.records.push(record);
        }
    }
    registry
        .records
        .sort_by(|a, b| a.meta.name.to_lowercase().cmp(&b.meta.name.to_lowercase()));

    // Mirror the GUI: skills disabled in Settings are not offered to the model.
    registry
        .records
        .retain(|record| is_skill_enabled(&settings.chat_tools, &record.meta.id));
    registry
}

/// Extra tool definitions to expose when skills are available: the single
/// `skill` activation tool. Returns empty when the registry has no skills
/// (nothing to activate) — matching the GUI, which only advertises the catalog
/// when skills exist.
///
/// NOTE: unlike the GUI we do not gate on `native_tools.skill_runtime` here. The
/// headless CLI has no Settings UI to flip that flag and ships with the runtime
/// off by default; gating on it would make a user's freshly-dropped skill silently
/// non-activatable. Presence of at least one discovered+enabled skill is the
/// signal that the model should be offered the activation tool.
pub fn skill_tool_definitions(registry: &SkillRegistry) -> Vec<ChatToolDefinition> {
    if registry.records.is_empty() {
        return Vec::new();
    }
    native_skill_tools()
}

/// Summarize a registry's skills for the interactive `/skill` listing:
/// `(name, description, enabled)` per record, in registry order.
///
/// The registry only ever holds skills the user has NOT disabled in Settings
/// (see [`build_skill_registry`], which drops disabled ids), so every record
/// surfaced here is enabled — the `enabled` flag is `true` for each. It is kept
/// in the tuple so the `/skill` renderer can present an explicit state column
/// and so the shape stays stable if the registry later carries disabled skills.
pub fn skill_summaries(registry: &SkillRegistry) -> Vec<(String, String, bool)> {
    registry
        .records
        .iter()
        .map(|record| {
            (
                record.meta.name.clone(),
                record.meta.description.clone(),
                true,
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    fn temp_dir() -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("kivio-code-skillsetup-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Write a minimal valid SKILL.md under `<root>/<slug>/SKILL.md`.
    fn write_skill(root: &Path, slug: &str, name: &str) {
        write_skill_desc(root, slug, name, &format!("A test skill named {name}."));
    }

    /// Like [`write_skill`] but with an explicit description (used to tell two
    /// same-id copies apart in the precedence test).
    fn write_skill_desc(root: &Path, slug: &str, name: &str, description: &str) {
        let skill_dir = root.join(slug);
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(
            skill_dir.join("SKILL.md"),
            format!(
                "---\nname: {name}\ndescription: {description}\n---\n\n# {name}\nDo the thing.\n"
            ),
        )
        .unwrap();
    }

    #[test]
    fn build_skill_registry_discovers_scan_path_skill() {
        let scan_dir = temp_dir();
        write_skill(&scan_dir, "demo-skill", "demo-skill");

        let mut settings = Settings::default();
        settings.chat_tools.skill_scan_paths = vec![scan_dir.to_string_lossy().into_owned()];

        let registry = build_skill_registry(&settings, Path::new("/tmp"));
        let found = registry
            .records
            .iter()
            .any(|r| r.meta.name == "demo-skill" && r.meta.description.contains("test skill"));
        assert!(found, "scan-path skill should be discovered and parsed");

        let _ = fs::remove_dir_all(&scan_dir);
    }

    #[test]
    fn build_skill_registry_indexes_skill_files() {
        let scan_dir = temp_dir();
        write_skill(&scan_dir, "with-files", "with-files");
        let scripts = scan_dir.join("with-files").join("scripts");
        fs::create_dir_all(&scripts).unwrap();
        fs::write(scripts.join("run.sh"), "echo ok").unwrap();

        let mut settings = Settings::default();
        settings.chat_tools.skill_scan_paths = vec![scan_dir.to_string_lossy().into_owned()];

        let registry = build_skill_registry(&settings, Path::new("/tmp"));
        let record = registry
            .records
            .iter()
            .find(|r| r.meta.name == "with-files")
            .expect("skill discovered");
        assert!(
            record
                .meta
                .files
                .iter()
                .any(|f| f.relative_path == "scripts/run.sh"),
            "headless discovery must index skill resource files"
        );

        let _ = fs::remove_dir_all(&scan_dir);
    }

    #[test]
    fn disabled_skill_is_dropped_from_registry() {
        let scan_dir = temp_dir();
        write_skill(&scan_dir, "keep-me", "keep-me");
        write_skill(&scan_dir, "drop-me", "drop-me");

        let mut settings = Settings::default();
        settings.chat_tools.skill_scan_paths = vec![scan_dir.to_string_lossy().into_owned()];
        settings.chat_tools.disabled_skill_ids = vec!["drop-me".to_string()];

        let registry = build_skill_registry(&settings, Path::new("/tmp"));
        assert!(registry.records.iter().any(|r| r.meta.id == "keep-me"));
        assert!(
            !registry.records.iter().any(|r| r.meta.id == "drop-me"),
            "skills disabled in Settings must not appear in the registry"
        );

        let _ = fs::remove_dir_all(&scan_dir);
    }

    #[test]
    fn skill_summaries_reflects_registry_records() {
        let scan_dir = temp_dir();
        write_skill(&scan_dir, "alpha", "alpha");
        write_skill(&scan_dir, "beta", "beta");

        let mut settings = Settings::default();
        settings.chat_tools.skill_scan_paths = vec![scan_dir.to_string_lossy().into_owned()];
        // beta is disabled in Settings → dropped from the registry → absent here.
        settings.chat_tools.disabled_skill_ids = vec!["beta".to_string()];

        let registry = build_skill_registry(&settings, Path::new("/tmp"));
        let summaries = skill_summaries(&registry);
        // Only the enabled (non-disabled) skill survives in the registry.
        assert!(summaries.iter().any(|(name, desc, enabled)| name == "alpha"
            && desc.contains("test skill")
            && *enabled));
        assert!(
            !summaries.iter().any(|(name, _, _)| name == "beta"),
            "disabled skill must not appear in the summaries"
        );
        // Every surfaced record is enabled (registry only holds enabled skills).
        assert!(summaries.iter().all(|(_, _, enabled)| *enabled));

        let _ = fs::remove_dir_all(&scan_dir);
    }

    #[test]
    fn skill_summaries_empty_for_empty_registry() {
        let registry = SkillRegistry::default();
        assert!(skill_summaries(&registry).is_empty());
    }

    #[test]
    fn skill_tool_definitions_empty_for_empty_registry() {
        let registry = SkillRegistry::default();
        assert!(skill_tool_definitions(&registry).is_empty());
    }

    #[test]
    fn skill_tool_definitions_exposes_activation_trio_when_skills_exist() {
        let scan_dir = temp_dir();
        write_skill(&scan_dir, "activ-skill", "activ-skill");

        let mut settings = Settings::default();
        settings.chat_tools.skill_scan_paths = vec![scan_dir.to_string_lossy().into_owned()];

        let registry = build_skill_registry(&settings, Path::new("/tmp"));
        assert!(!registry.records.is_empty(), "precondition: skill present");

        let defs = skill_tool_definitions(&registry);
        let names: Vec<&str> = defs.iter().map(|d| d.name.as_str()).collect();
        assert_eq!(names, vec!["skill"]);
        // Carries the "skill" source the loop matches on for skill dispatch.
        assert!(defs.iter().all(|d| d.source == "skill"));

        let _ = fs::remove_dir_all(&scan_dir);
    }

    #[test]
    fn project_claude_skill_discovered_when_read_claude_on() {
        let cwd = temp_dir();
        let claude_skills = cwd.join(".claude").join("skills");
        fs::create_dir_all(&claude_skills).unwrap();
        write_skill(&claude_skills, "foo", "foo");

        let settings = Settings::default();
        let registry = build_skill_registry_with(&settings, &cwd, true);
        assert!(
            registry.records.iter().any(|r| r.meta.id == "foo"),
            "project .claude/skills must be discovered when read_claude is on"
        );

        let _ = fs::remove_dir_all(&cwd);
    }

    #[test]
    fn project_claude_skill_hidden_when_read_claude_off() {
        let cwd = temp_dir();
        let claude_skills = cwd.join(".claude").join("skills");
        fs::create_dir_all(&claude_skills).unwrap();
        write_skill(&claude_skills, "foo", "foo");

        let settings = Settings::default();
        let registry = build_skill_registry_with(&settings, &cwd, false);
        assert!(
            !registry.records.iter().any(|r| r.meta.id == "foo"),
            "project .claude/skills must NOT be discovered when read_claude is off"
        );

        let _ = fs::remove_dir_all(&cwd);
    }

    #[test]
    fn project_kivio_skill_always_discovered() {
        let cwd = temp_dir();
        let kivio_skills = cwd.join(".kivio").join("skills");
        fs::create_dir_all(&kivio_skills).unwrap();
        write_skill(&kivio_skills, "bar", "bar");

        let settings = Settings::default();
        // Independent of the Claude toggle, in both states.
        for read_claude in [true, false] {
            let registry = build_skill_registry_with(&settings, &cwd, read_claude);
            assert!(
                registry.records.iter().any(|r| r.meta.id == "bar"),
                ".kivio/skills must always be discovered (read_claude={read_claude})"
            );
        }

        let _ = fs::remove_dir_all(&cwd);
    }

    #[test]
    fn project_skill_wins_over_global_on_id_collision() {
        // Same id ("dup", derived from the skill name) in a global scan path and
        // in the project .kivio/skills. Distinguish the two copies by description.
        let scan_dir = temp_dir();
        write_skill_desc(&scan_dir, "dup", "dup", "GLOBAL copy");

        let cwd = temp_dir();
        let kivio_skills = cwd.join(".kivio").join("skills");
        fs::create_dir_all(&kivio_skills).unwrap();
        write_skill_desc(&kivio_skills, "dup", "dup", "PROJECT copy");

        let mut settings = Settings::default();
        settings.chat_tools.skill_scan_paths = vec![scan_dir.to_string_lossy().into_owned()];

        let registry = build_skill_registry_with(&settings, &cwd, true);
        let dup = registry
            .records
            .iter()
            .find(|r| r.meta.id == "dup")
            .expect("dup skill present");
        assert!(
            dup.location.starts_with(&kivio_skills),
            "project copy must win on id collision; got {:?}",
            dup.location
        );
        assert!(
            dup.meta.description.contains("PROJECT copy"),
            "the project description must be the surviving one; got {:?}",
            dup.meta.description
        );
        // Exactly one record for the id (the project copy).
        assert_eq!(
            registry
                .records
                .iter()
                .filter(|r| r.meta.id == "dup")
                .count(),
            1
        );

        let _ = fs::remove_dir_all(&scan_dir);
        let _ = fs::remove_dir_all(&cwd);
    }

    #[test]
    fn missing_project_skill_dirs_are_skipped_without_error() {
        // cwd has neither .kivio/skills nor .claude/skills.
        let cwd = temp_dir();
        let settings = Settings::default();

        // Must not panic and must not surface project roots as warnings.
        let registry = build_skill_registry_with(&settings, &cwd, true);
        assert!(
            !registry
                .warnings
                .iter()
                .any(|w| w.contains(".kivio") || w.contains(".claude")),
            "non-existent project dirs must be skipped silently, got {:?}",
            registry.warnings
        );

        let _ = fs::remove_dir_all(&cwd);
    }
}
