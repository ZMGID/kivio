use std::sync::Arc;

use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;

use super::types::{SkillRecord, SkillRegistry};
use crate::{
    model::{ModelArtifact, ModelTool},
    tools::{input_schema, StaticTool, ToolDefinition, ToolFuture, ToolOutput},
};

#[derive(Debug, Clone, Default)]
pub struct SkillAccess {
    pub disabled_ids: Vec<String>,
    /// None permits all enabled skills; an empty list permits none.
    pub allowed_ids: Option<Vec<String>>,
}

pub trait SkillBodyReader: Send + Sync {
    fn read_body(&self, record: &SkillRecord) -> Result<String, String>;
}

struct FileSkillBodyReader;

impl SkillBodyReader for FileSkillBodyReader {
    fn read_body(&self, record: &SkillRecord) -> Result<String, String> {
        Ok(super::read_skill_record(record)?.body)
    }
}

#[derive(Clone)]
pub struct SkillRunContext {
    registry: Arc<SkillRegistry>,
    access: SkillAccess,
    body_reader: Arc<dyn SkillBodyReader>,
}

impl Default for SkillRunContext {
    fn default() -> Self {
        Self::new(Arc::new(SkillRegistry::default()), SkillAccess::default())
    }
}

impl SkillRunContext {
    pub fn new(registry: Arc<SkillRegistry>, access: SkillAccess) -> Self {
        Self {
            registry,
            access,
            body_reader: Arc::new(FileSkillBodyReader),
        }
    }

    pub fn with_body_reader(mut self, reader: Arc<dyn SkillBodyReader>) -> Self {
        self.body_reader = reader;
        self
    }

    pub fn registry(&self) -> &SkillRegistry {
        &self.registry
    }

    pub fn skill_id_allowed(&self, skill_id: &str) -> bool {
        !self.access.disabled_ids.iter().any(|id| id == skill_id)
            && self
                .access
                .allowed_ids
                .as_ref()
                .is_none_or(|ids| ids.iter().any(|id| id == skill_id))
    }

    /// Read fresh instructions from the frozen catalog without granting any tools.
    /// disable_model_invocation controls catalog visibility; host authorization and access govern loading.
    pub fn load(&self, name: &str, arguments: Option<&str>) -> Result<String, String> {
        let name = name.trim();
        if name.is_empty() {
            return Err("Skill name is required".into());
        }
        let record = self
            .registry
            .find(name)
            .ok_or_else(|| format!("Skill not found: {name}"))?;
        if self
            .access
            .disabled_ids
            .iter()
            .any(|id| id == &record.meta.id)
        {
            return Err(format!("Skill is disabled: {name}"));
        }
        if !self.skill_id_allowed(&record.meta.id) {
            return Err(format!("Skill is not allowed: {name}"));
        }
        let mut loaded = record.clone();
        loaded.body = self.body_reader.read_body(record)?;
        loaded.body = substitute_arguments(
            &loaded.body,
            arguments.unwrap_or_default(),
            &record.meta.arguments,
        );
        Ok(activate_skill(&loaded))
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SkillArgs {
    /// Skill name from available_skills.
    pub name: String,
    /// Optional trailing arguments for the skill's declared placeholders.
    #[serde(default)]
    pub arguments: Option<String>,
}

pub struct SkillTool {
    context: SkillRunContext,
}

impl SkillTool {
    pub fn new(context: SkillRunContext) -> Self {
        Self { context }
    }

    pub fn definition() -> ToolDefinition {
        let mut definition = ToolDefinition::new(ModelTool {
            name: "skill".into(),
            description: "Load the instructions for a listed skill according to the session's skill activation policy. Injects the skill's instructions and resources into the current conversation. The output may contain workflow guidance and references to scripts and files in the skill directory. Read files or execute scripts only through separately authorized tools. A previous activation is not proof that its instructions remain in context. Reload when needed after compaction or history changes, or when skill files change. If the output is truncated, read the skill file in sections instead of repeatedly loading the same truncated output.".into(),
            input_schema: input_schema::<Self>(),
        });
        definition.label.source = "skill".into();
        definition.parallel_safe = true;
        definition
    }
}

impl StaticTool for SkillTool {
    type Args = SkillArgs;
    type Output = Value;
    type Error = String;
    type HostData = Vec<ModelArtifact>;

    fn execute<'a>(
        &'a self,
        args: Self::Args,
    ) -> ToolFuture<'a, Result<ToolOutput<Self::HostData, Self::Output>, Self::Error>> {
        let context = self.context.clone();
        Box::pin(async move {
            let content = tokio::task::spawn_blocking(move || {
                context.load(&args.name, args.arguments.as_deref())
            })
            .await
            .map_err(|error| format!("Load skill failed: {error}"))??;
            Ok(ToolOutput::text(content))
        })
    }
}

/// Substitute argument placeholders in a skill body.
///
/// - `$ARGUMENTS` → the full trailing argument string (everything after the
///   slash command), verbatim.
/// - `$ARG_NAME` → the positional word at the index of `ARG_NAME` in
///   `arg_names`. Words are whitespace-split from `args_raw`. A declared name
///   with no corresponding word substitutes to empty (never panics).
///
/// Unknown `$NAME` placeholders (not `ARGUMENTS`, not a declared arg) are left
/// untouched so skill bodies can mention literal `$` text safely.
pub fn substitute_arguments(body: &str, args_raw: &str, arg_names: &[String]) -> String {
    let trimmed = args_raw.trim();
    let words: Vec<&str> = trimmed.split_whitespace().collect();

    // Map of UPPERCASE declared name -> positional value (missing word => "").
    let mut name_values: std::collections::HashMap<String, &str> = std::collections::HashMap::new();
    for (index, name) in arg_names.iter().enumerate() {
        let key = name.trim();
        if key.is_empty() {
            continue;
        }
        name_values
            .entry(key.to_ascii_uppercase())
            .or_insert_with(|| words.get(index).copied().unwrap_or(""));
    }

    // Single left-to-right scan: every `$TOKEN` is resolved exactly once against the
    // original body. Substituted values are emitted verbatim and never re-scanned, so
    // a value containing a `$ARG_...`-like token is not re-substituted, and no token is
    // a prefix-collision victim of another (e.g. `$A` vs `$AB`).
    let bytes = body.as_bytes();
    let mut out = String::with_capacity(body.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'$' {
            // Copy a full UTF-8 char (find next char boundary).
            let mut end = i + 1;
            while end < bytes.len() && !body.is_char_boundary(end) {
                end += 1;
            }
            out.push_str(&body[i..end]);
            i = end;
            continue;
        }
        // Read the identifier after `$`: [A-Za-z0-9_]+ (ASCII only).
        let start = i + 1;
        let mut end = start;
        while end < bytes.len() {
            let c = bytes[end];
            if c.is_ascii_alphanumeric() || c == b'_' {
                end += 1;
            } else {
                break;
            }
        }
        if end == start {
            // Lone `$` (no identifier) → literal.
            out.push('$');
            i += 1;
            continue;
        }
        let ident = &body[start..end];
        let upper = ident.to_ascii_uppercase();
        if upper == "ARGUMENTS" {
            out.push_str(trimmed);
        } else if let Some(value) = name_values.get(&upper) {
            out.push_str(value);
        } else if let Some(stripped) = upper.strip_prefix("ARG_") {
            // `$ARG_NAME` convention: resolve the stripped name, else empty string.
            out.push_str(name_values.get(stripped).copied().unwrap_or(""));
        } else {
            // Unknown `$NAME` → leave literal so bodies can mention `$` text safely.
            out.push('$');
            out.push_str(ident);
        }
        i = end;
    }
    out
}

pub fn activate_skill(record: &SkillRecord) -> String {
    let mut out = format!(
        "<skill_content name=\"{}\">\n",
        xml_escape(&record.meta.name)
    );
    out.push_str("Skill file: ");
    out.push_str(&record.location.display().to_string());
    out.push_str("\nIf this output is truncated, use read on the skill file to retrieve the missing instructions.\n\n");
    out.push_str(&record.body);
    out.push_str("\n\nSkill directory: ");
    out.push_str(&record.base_dir.display().to_string());
    out.push_str("\nRelative paths in this skill are relative to the skill directory.\n");

    out.push_str("</skill_content>");
    out
}

pub fn extract_skill_name(arguments: &Value) -> Result<String, String> {
    arguments
        .get("name")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
        .ok_or_else(|| "Skill name is required".to_string())
}

fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::super::types::{SkillMeta, SkillRecord};
    use super::*;
    use std::path::PathBuf;

    fn demo_meta() -> SkillMeta {
        SkillMeta {
            id: "demo".to_string(),
            name: "demo".to_string(),
            description: "Demo".to_string(),
            source: "user".to_string(),
            path: None,
            recommended_tools: vec![],
            disable_model_invocation: false,
            files: vec![],
            triggers: vec![],
            argument_hint: None,
            arguments: vec![],
        }
    }

    #[test]
    fn skill_access_denies_before_body_io_and_manual_visibility_is_not_authority() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        struct Reader(Arc<AtomicUsize>);
        impl SkillBodyReader for Reader {
            fn read_body(&self, _record: &SkillRecord) -> Result<String, String> {
                self.0.fetch_add(1, Ordering::SeqCst);
                Ok("Hello $ARG_WHO. $ARGUMENTS".into())
            }
        }

        let mut meta = demo_meta();
        meta.disable_model_invocation = true;
        meta.arguments = vec!["who".into()];
        let registry = Arc::new(SkillRegistry {
            records: vec![SkillRecord {
                meta,
                location: PathBuf::from("/skills/demo/SKILL.md"),
                base_dir: PathBuf::from("/skills/demo"),
                body: String::new(),
            }],
            ..Default::default()
        });
        let reads = Arc::new(AtomicUsize::new(0));
        for access in [
            SkillAccess {
                disabled_ids: vec!["demo".into()],
                allowed_ids: None,
            },
            SkillAccess {
                disabled_ids: vec![],
                allowed_ids: Some(vec![]),
            },
            SkillAccess {
                disabled_ids: vec![],
                allowed_ids: Some(vec!["other".into()]),
            },
        ] {
            let context = SkillRunContext::new(registry.clone(), access)
                .with_body_reader(Arc::new(Reader(reads.clone())));
            assert!(context.load("demo", None).is_err());
        }
        assert_eq!(reads.load(Ordering::SeqCst), 0);
        let context = SkillRunContext::new(registry, SkillAccess::default())
            .with_body_reader(Arc::new(Reader(reads.clone())));
        assert!(context.load("missing", None).is_err());
        assert_eq!(reads.load(Ordering::SeqCst), 0);
        assert!(context
            .load("demo", Some("World now"))
            .unwrap()
            .contains("Hello World. World now"));
        assert_eq!(reads.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn repeated_activation_returns_complete_instructions() {
        let record = SkillRecord {
            meta: demo_meta(),
            location: PathBuf::from("/skills/demo/SKILL.md"),
            base_dir: PathBuf::from("/skills/demo"),
            body: "Skill body".to_string(),
        };
        let first = activate_skill(&record);
        assert!(first.contains("Skill body"));
        let second = activate_skill(&record);
        assert!(second.contains("Skill body"));
        assert!(second.contains("</skill_content>"));
    }

    #[test]
    fn substitute_arguments_replaces_full_and_positional() {
        let body = "Commit with message: $ARGUMENTS\nFirst: $TITLE Second: $SCOPE";
        let out = substitute_arguments(
            body,
            "  fix login regression  ",
            &["title".to_string(), "scope".to_string()],
        );
        assert!(out.contains("Commit with message: fix login regression"));
        assert!(out.contains("First: fix Second: login"));
    }

    #[test]
    fn substitute_arguments_missing_positional_is_empty() {
        let body = "A=$FIRST B=$SECOND end";
        let out = substitute_arguments(body, "only", &["first".to_string(), "second".to_string()]);
        assert_eq!(out, "A=only B= end");
    }

    #[test]
    fn substitute_arguments_no_prefix_collision_between_a_and_ab() {
        // FIX 6 (1): `$ARG_A` must resolve to the `a` value and `$ARG_AB` to the `ab`
        // value; the old multi-pass impl corrupted `$ARG_AB` while replacing `$ARG_A`.
        let out = substitute_arguments(
            "$ARG_A|$ARG_AB",
            "x y",
            &["a".to_string(), "ab".to_string()],
        );
        assert_eq!(out, "x|y");
    }

    #[test]
    fn substitute_arguments_does_not_re_substitute_value_tokens() {
        // FIX 6 (2): a positional value that itself contains a `$ARG_...` token must be
        // emitted verbatim and never re-scanned/substituted by a later pass.
        let out = substitute_arguments(
            "first=$ARG_A second=$ARG_B",
            r#"$ARG_B payload"#,
            &["a".to_string(), "b".to_string()],
        );
        // word[0] == "$ARG_B", word[1] == "payload"; $ARG_A -> "$ARG_B" (literal),
        // $ARG_B -> "payload". The injected "$ARG_B" must survive unchanged.
        assert_eq!(out, "first=$ARG_B second=payload");
    }
}
