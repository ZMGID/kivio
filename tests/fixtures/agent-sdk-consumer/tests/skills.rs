mod support;

use std::{
    fs,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::{SystemTime, UNIX_EPOCH},
};

use kivio_agent_sdk::{
    agent::{
        content::AgentToolStatus, execute::ToolProgress, AgentSession, AgentSessionConfig,
        RuntimeMessage, ToolExecutionContext,
    },
    skills::{
        format_catalog, SkillAccess, SkillBodyReader, SkillRecord, SkillRegistry, SkillRunContext,
        SkillSource, SkillTool,
    },
    tools::{ToolAuthorization, ToolDefinition, ToolError, ToolFuture, ToolRegistry},
};
use serde_json::{json, Value};

struct SkillFiles(PathBuf);

impl SkillFiles {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "kivio-sdk-skills-{}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            NEXT.fetch_add(1, Ordering::Relaxed),
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }

    fn write(&self, directory: &str, name: &str, fields: &str, body: &str) -> PathBuf {
        let path = self.0.join(directory).join("SKILL.md");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(
            &path,
            format!("---\nname: {name}\ndescription: {name} instructions\n{fields}---\n{body}\n"),
        )
        .unwrap();
        path
    }

    fn source(&self, directory: &str, source: &str) -> SkillSource {
        SkillSource {
            path: self.0.join(directory),
            source: source.into(),
        }
    }
}

impl Drop for SkillFiles {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

struct Authorization(bool);

impl ToolAuthorization for Authorization {
    fn authorize<'a>(
        &'a self,
        _: &'a ToolProgress<'a>,
        _: &'a ToolExecutionContext<'a>,
        _: &'a ToolDefinition,
        _: &'a Value,
    ) -> ToolFuture<'a, Result<(), ToolError>> {
        Box::pin(async move {
            if self.0 {
                Ok(())
            } else {
                Err(ToolError::Execution("host denied skill".into()))
            }
        })
    }
}

fn tools(context: SkillRunContext, allowed: bool) -> ToolRegistry {
    let mut tools = ToolRegistry::new();
    tools
        .register_static(
            SkillTool::definition(),
            SkillTool::new(context),
            Arc::new(Authorization(allowed)),
        )
        .unwrap();
    tools
}

fn session(server: &support::Server, tools: ToolRegistry, catalog: String) -> AgentSession {
    let mut config = AgentSessionConfig::new(Arc::new(server.provider()), "test-model");
    config.tools = tools;
    config.history = vec![RuntimeMessage::system(catalog)].into();
    config.options.thinking_enabled = false;
    AgentSession::new(config)
}

#[test]
fn explicit_sources_keep_first_id_and_report_partial_discovery() {
    let files = SkillFiles::new();
    let first = files.write("project/demo", "demo", "", "project body");
    files.write("personal/demo", "demo", "", "personal body");
    files.write("duplicate/demo", "demo", "", "duplicate body");
    let invalid = files.write("invalid/broken", "broken", "", "broken body");
    fs::write(invalid, "---\nname: broken\n---\nmissing description").unwrap();
    let registry = SkillRegistry::discover(&[
        files.source("project", "project"),
        files.source("personal", "personal"),
        files.source("duplicate", "project"),
        files.source("invalid", "extra"),
    ]);
    assert!(registry.scan_failed);
    assert_eq!(registry.metas().len(), 1);
    assert_eq!(registry.find("demo").unwrap().location, first);
    assert_eq!(registry.find("demo").unwrap().meta.source, "project");
    assert!(registry.records.iter().all(|record| record.body.is_empty()));
    assert!(registry
        .warnings
        .iter()
        .any(|warning| warning.contains("duplicate")));
    assert!(registry
        .warnings
        .iter()
        .any(|warning| warning.contains("broken")));
    let catalog = format_catalog(&registry, None, |_| true);
    assert!(catalog.contains("demo instructions"));
    assert!(catalog.contains("SKILL.md"));
    assert!(!catalog.contains("project body"));
    assert!(!catalog.contains("personal body"));
}

#[test]
fn reload_updates_metadata_and_retains_the_previous_catalog_on_failure() {
    let files = SkillFiles::new();
    let path = files.write("demo", "demo", "", "original body");
    let sources = [files.source("", "host")];
    let mut registry = SkillRegistry::discover(&sources);
    let frozen = SkillRunContext::new(Arc::new(registry.clone()), SkillAccess::default());
    files.write("demo", "renamed", "", "edited body");
    files.write("new", "new", "", "new instructions");
    assert_eq!(frozen.registry().metas().len(), 1);
    assert!(frozen.registry().find("renamed").is_none());
    assert!(frozen.registry().find("new").is_none());
    let loaded = frozen.load("demo", None).unwrap();
    assert!(loaded.contains("<skill_content name=\"demo\">"));
    assert!(loaded.contains("edited body"));
    assert!(loaded.contains(&path.display().to_string()));
    registry.reload(&sources).unwrap();
    assert!(registry.find("demo").is_none());
    assert!(registry.find("renamed").is_some());
    assert!(registry.find("new").is_some());
    assert_eq!(frozen.registry().metas().len(), 1);
    fs::write(&path, "invalid skill markdown").unwrap();
    assert!(registry.reload(&sources).is_err());
    assert!(registry.find("renamed").is_some());
    assert!(registry.find("new").is_some());
    assert_eq!(registry.metas().len(), 2);
}

#[tokio::test]
async fn session_loads_fresh_instructions_without_adding_tools_or_running_scripts() {
    let files = SkillFiles::new();
    files.write(
        "demo",
        "demo",
        "arguments: [first, second]\nallowed-tools: shell secret_tool\n",
        "initial body",
    );
    let marker = files.0.join("script-ran");
    let script_dir = files.0.join("demo/scripts");
    fs::create_dir(&script_dir).unwrap();
    fs::write(
        script_dir.join("run.ps1"),
        format!("Set-Content -LiteralPath '{}' -Value ran", marker.display()),
    )
    .unwrap();
    fs::write(
        script_dir.join("run.sh"),
        format!("touch '{}'", marker.display()),
    )
    .unwrap();
    fs::create_dir(files.0.join("demo/references")).unwrap();
    fs::write(
        files.0.join("demo/references/private.md"),
        "RESOURCE_ONLY_SECRET",
    )
    .unwrap();
    let registry = Arc::new(SkillRegistry::discover(&[files.source("", "host")]));
    assert_eq!(
        registry.find("demo").unwrap().meta.recommended_tools,
        ["secret_tool", "shell"]
    );
    let catalog = format_catalog(&registry, None, |_| true);
    let context = SkillRunContext::new(registry.clone(), SkillAccess::default());
    let tool_name = SkillTool::definition().model.name;
    let arguments = json!({"name":"demo","arguments":"$ARG_SECOND value"}).to_string();
    let server = support::Server::start(vec![
        support::tool(&tool_name, &arguments),
        support::answer("first answer"),
        support::tool(&tool_name, r#"{"name":"demo"}"#),
        support::answer("second answer"),
    ])
    .await;
    let registered = tools(context, true);
    assert_eq!(registered.definitions().len(), 1);
    let session = session(&server, registered, catalog);
    files.write(
        "demo",
        "demo",
        "arguments: [first, second]\nallowed-tools: shell secret_tool\n",
        "fresh body\nfirst=$ARG_FIRST\nsecond=$ARG_SECOND\nall=$ARGUMENTS",
    );
    let first = tokio::time::timeout(support::LIMIT, session.prompt("activate demo"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(first.content, "first answer");
    assert_eq!(first.tool_records[0].status, AgentToolStatus::Success);
    files.write("demo", "demo", "", "second fresh body");
    let second = tokio::time::timeout(support::LIMIT, session.prompt("activate again"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(second.content, "second answer");
    assert!(!marker.exists());
    assert!(registry.records.iter().all(|record| record.body.is_empty()));
    let requests = server.requests.lock().unwrap();
    assert_eq!(requests.len(), 4);
    assert!(!requests[0].body["messages"]
        .to_string()
        .contains("initial body"));
    let first_messages = requests[1].body["messages"].as_array().unwrap();
    let loaded = first_messages.last().unwrap()["content"].as_str().unwrap();
    assert!(loaded.contains("fresh body"));
    assert!(loaded.contains("first=$ARG_SECOND\nsecond=value\nall=$ARG_SECOND value"));
    assert!(!loaded.contains("RESOURCE_ONLY_SECRET"));
    assert!(!loaded.contains("Set-Content"));
    let second_messages = requests[3].body["messages"].as_array().unwrap();
    assert!(second_messages.last().unwrap()["content"]
        .as_str()
        .unwrap()
        .contains("second fresh body"));
    for request in requests.iter() {
        assert_eq!(request.body["tools"], requests[0].body["tools"]);
        let definitions = request.body["tools"].as_array().unwrap();
        assert_eq!(definitions.len(), 1);
        assert_eq!(definitions[0]["function"]["name"], tool_name);
    }
}

struct ReadProbe(Arc<AtomicUsize>);

impl SkillBodyReader for ReadProbe {
    fn read_body(&self, _: &SkillRecord) -> Result<String, String> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Err("body read reached".into())
    }
}

#[tokio::test]
async fn host_denial_and_skill_access_stop_before_reading_instructions() {
    let files = SkillFiles::new();
    files.write("demo", "demo", "", "instructions");
    let registry = Arc::new(SkillRegistry::discover(&[files.source("", "host")]));
    for (allowed, access, reads) in [
        (false, SkillAccess::default(), 0),
        (
            true,
            SkillAccess {
                disabled_ids: vec!["demo".into()],
                allowed_ids: None,
            },
            0,
        ),
        (
            true,
            SkillAccess {
                disabled_ids: vec![],
                allowed_ids: Some(vec![]),
            },
            0,
        ),
        (
            true,
            SkillAccess {
                disabled_ids: vec![],
                allowed_ids: Some(vec!["other".into()]),
            },
            0,
        ),
        (
            true,
            SkillAccess {
                disabled_ids: vec![],
                allowed_ids: Some(vec!["demo".into()]),
            },
            1,
        ),
    ] {
        let count = Arc::new(AtomicUsize::new(0));
        let context = SkillRunContext::new(registry.clone(), access)
            .with_body_reader(Arc::new(ReadProbe(count.clone())));
        let catalog = format_catalog(context.registry(), None, |id| context.skill_id_allowed(id));
        assert_eq!(catalog.is_empty(), allowed && reads == 0);
        let server = support::Server::start(vec![
            support::tool(&SkillTool::definition().model.name, r#"{"name":"demo"}"#),
            support::answer("handled error"),
        ])
        .await;
        let session = session(&server, tools(context, allowed), catalog);
        let result = tokio::time::timeout(support::LIMIT, session.prompt("activate"))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(result.content, "handled error");
        assert_eq!(result.tool_records.len(), 1);
        assert_eq!(
            result.tool_records[0].status,
            if allowed {
                AgentToolStatus::Error
            } else {
                AgentToolStatus::Skipped
            }
        );
        assert_eq!(count.load(Ordering::SeqCst), reads);
        let requests = server.requests.lock().unwrap();
        let messages = requests[1].body["messages"].as_array().unwrap();
        let error = messages.last().unwrap()["content"].as_str().unwrap();
        if !allowed {
            assert!(error.contains("host denied skill"));
        } else if reads == 1 {
            assert!(error.contains("body read reached"));
        } else {
            assert!(!error.contains("body read reached"));
        }
    }
}

#[test]
fn manual_only_visibility_is_separate_from_explicit_host_access() {
    let files = SkillFiles::new();
    files.write(
        "manual",
        "manual",
        "disable-model-invocation: true\n",
        "manual body",
    );
    let registry = Arc::new(SkillRegistry::discover(&[files.source("", "host")]));
    assert!(format_catalog(&registry, None, |_| true).is_empty());
    assert!(format_catalog(&registry, Some("manual"), |_| true).contains("manual instructions"));
    let context = SkillRunContext::new(registry, SkillAccess::default());
    assert!(context
        .load("manual", None)
        .unwrap()
        .contains("manual body"));
}

#[test]
fn missing_invalid_and_non_utf8_instruction_files_fail_activation() {
    let files = SkillFiles::new();
    let path = files.write("demo", "demo", "", "valid body");
    let registry = Arc::new(SkillRegistry::discover(&[files.source("", "host")]));
    let context = SkillRunContext::new(registry, SkillAccess::default());
    for bytes in [b"invalid markdown".as_slice(), &[0xff, 0xfe]] {
        fs::write(&path, bytes).unwrap();
        assert!(context.load("demo", None).is_err());
    }
    fs::remove_file(path).unwrap();
    assert!(context.load("demo", None).is_err());
    assert!(context.load("unknown", None).is_err());
}

#[test]
fn resources_resolve_only_existing_paths_inside_the_skill_directory() {
    let files = SkillFiles::new();
    files.write("demo", "demo", "", "instructions");
    let reference = files.0.join("demo/reference.txt");
    fs::write(&reference, "reference").unwrap();
    let outside = files.0.join("outside.txt");
    fs::write(&outside, "outside").unwrap();
    let registry = SkillRegistry::discover(&[files.source("demo", "host")]);
    let record = registry.find("demo").unwrap();
    assert_eq!(
        record.resolve_resource(Path::new("reference.txt")).unwrap(),
        fs::canonicalize(&reference).unwrap()
    );
    assert!(record.resolve_resource(Path::new("missing.txt")).is_err());
    assert!(record
        .resolve_resource(Path::new("../outside.txt"))
        .is_err());
    assert!(record.resolve_resource(&outside).is_err());
    let escape = files.0.join("demo/escape.txt");
    #[cfg(windows)]
    let linked = std::os::windows::fs::symlink_file(&outside, &escape);
    #[cfg(unix)]
    let linked = std::os::unix::fs::symlink(&outside, &escape);
    #[cfg(any(windows, unix))]
    match linked {
        Ok(()) => assert!(record.resolve_resource(Path::new("escape.txt")).is_err()),
        Err(error) => eprintln!("symlink escape check unavailable: {error}"),
    }
}
