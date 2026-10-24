//! Native host adapter for the original Agent tool loop and Rust tool owners.
use std::{path::PathBuf, sync::Arc};
use serde_json::{json, Value};
use crate::{agent::{ToolExecutor, ToolExecutorFuture, ToolExecutionContext}, conversation_data::Conversation,
    mcp::{NativeMcp, types::{self, ChatToolDefinition, McpToolCallResult}},
    native_tools::{self, NativeToolWorkspace, ShellHost, background_registry::BackgroundCommandRegistry},
    resource_sessions::{Catalog, ResourceSessions}, settings::Settings, skills::{SkillRegistry, SkillRunContext}};

static RESOURCE_REVISION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// Explicit installation/settings changes retire cached catalogs before the next run.
pub fn invalidate_resource_catalogs() {
    RESOURCE_REVISION.fetch_add(1, std::sync::atomic::Ordering::AcqRel);
}

pub struct PreparedNativeTools {
    pub tools: Vec<ChatToolDefinition>,
    pub blocked_tools: Vec<ChatToolDefinition>,
    pub skill_registry: Arc<SkillRegistry>,
    pub working_directory: PathBuf,
    pub workflow_hooks: Option<crate::workflow_hooks::Runtime>,
    pub video_analysis: crate::video_analysis::VideoTool,
    pub video_reports: Vec<String>,
}
pub struct NativeToolExecutor {
    root: PathBuf,
    pool: NativeMcp,
    search: crate::web_search_host::NativeSearch,
    http: reqwest::Client,
    background: Arc<BackgroundCommandRegistry>,
    resources: ResourceSessions,
    resource_revision: std::sync::Mutex<u64>,
}
impl NativeToolExecutor {
    pub fn new(root: PathBuf) -> Self {
        Self { pool: NativeMcp::new(root.clone()), search: crate::web_search_host::NativeSearch::new(root.clone()),
            root, http: reqwest::Client::new(), background: Arc::default(), resources: ResourceSessions::default(), resource_revision: std::sync::Mutex::new(0) }
    }
    pub fn reload(&self, conversation_id: &str) { self.resources.forget(conversation_id); }
    pub fn abort_background(&self, conversation_id: &str) -> usize { self.background.kill_for_conversation(conversation_id) }
    pub async fn shutdown(&self) { self.background.kill_all(); self.pool.shutdown().await; }
    fn catalog_dir(&self) -> PathBuf { self.root.join("conversations") }
    fn settings(&self) -> Result<Settings,String> { crate::native_preferences::load_runtime(&self.root) }
    fn workspace(&self, id: &str, settings: &Settings) -> Result<NativeToolWorkspace,String> {
        let dir=self.catalog_dir();
        let conversation=crate::conversation_files::ConversationStore::new(dir.clone()).load(id).map_err(|e|e.to_string())?;
        if let Some(project)=crate::workspace::resolve_project(&dir,&conversation)? {
            Ok(NativeToolWorkspace::project(project.id,project.name,project.root_path))
        } else {
            Ok(NativeToolWorkspace::conversation(crate::workspace::resolve_working_directory(&dir,&conversation,&settings.chat_tools.native_tools.working_directory)?))
        }
    }
    pub async fn prepare(&self, settings: &Settings, conversation: &Conversation, chat_mode: bool) -> Result<PreparedNativeTools,String> {
        let revision = RESOURCE_REVISION.load(std::sync::atomic::Ordering::Acquire);
        {
            let mut applied = self.resource_revision.lock().unwrap_or_else(|error| error.into_inner());
            if *applied != revision {
                self.resources.invalidate();
                *applied = revision;
            }
        }
        let working_directory=crate::workspace::resolve_working_directory(&self.catalog_dir(),conversation,&settings.chat_tools.native_tools.working_directory)?;
        let cwd=working_directory.clone();
        let paths=settings.chat_tools.skill_scan_paths.clone();
        let bundled=std::env::current_exe().ok().and_then(|p|p.parent()?.parent().map(|p|p.join("Resources/skills"))).filter(|p|p.is_dir());
        let catalog=self.resources.load(&conversation.id,false,move || {
            let plugins=crate::plugins::packages::discover_active()?;
            let skills=crate::skills::build_registry_with_plugins_at(bundled.as_deref(),&paths,Some(&cwd),&plugins)?;
            let hooks=plugins.into_iter().flat_map(|p|p.hooks).collect();
            Ok(Catalog { skills:Arc::new(skills), hooks, ..Default::default() })
        }).await?;
        let main_provider = settings.get_provider(&conversation.provider_id).ok_or("主模型供应商不存在")?;
        let video_plan = crate::video_analysis::plan(settings, main_provider, &conversation.model, conversation)?;
        let video_reports = if video_plan.send_video { Vec::new() } else { video_plan.reports.clone() };
        let mut tools=Vec::new();
        if video_plan.model.is_some() { tools.push(crate::video_analysis::tool_definition()); }
        let video_analysis = crate::video_analysis::VideoTool::new(conversation.clone(), video_plan);
        let definitions: &[fn()->ChatToolDefinition]=&[
            types::native_web_search_tool,types::native_web_fetch_tool,types::native_read_file_tool,
            types::native_list_dir_tool,types::native_search_files_tool,types::native_glob_files_tool,
            types::native_write_file_tool,types::native_edit_file_tool,types::native_run_command_tool,
            types::native_bash_output_tool,types::native_kill_background_tool,
            types::native_memory_read_tool,types::native_memory_search_tool,types::native_memory_modify_tool,
        ];
        for definition in definitions {
            let tool=definition();
            if crate::mcp::native_policy::find_entry(&tool.name).is_some_and(|p|(p.enabled)(&settings.chat_tools.native_tools,crate::mcp::search_policy::web_search_configured(settings),settings.chat_memory.enabled)) { tools.push(tool); }
        }
        if settings.chat_tools.native_tools.skill_runtime { tools.extend(types::native_skill_tools()); }
        for server in crate::mcp::server_policy::eligible_mcp_servers(settings) {
            if conversation.assistant_snapshot.as_ref().is_some_and(|a| !a.mcp_server_ids.contains(&server.id)) { continue; }
            let discovered=self.pool.list(server).await.map_err(|e|format!("MCP {} discovery failed: {e}",server.name))?;
            tools.extend(crate::mcp::server_policy::tools_from_mcp(server,discovered));
        }
        crate::ask_user::append_tool_definitions(&mut tools);
        crate::todo::append_tool_definitions(&mut tools);
        crate::agent::prepare::apply_assistant_mcp_restrictions(&mut tools,conversation.assistant_snapshot.as_ref());
        let prompt=conversation.messages.iter().rev().find(|m|m.role=="user").map(|m|m.content.clone());
        crate::agent::tool_policy::apply_inline_code_request_tool_filter(&mut tools,prompt.as_deref());
        let mode=crate::conversation_data::WebSearchMode::resolve(conversation.web_search_mode,settings);
        let mode=settings.get_provider(&conversation.provider_id).map_or(mode,|p|mode.for_provider(p));
        crate::agent::tool_policy::apply_web_search_mode_tool_filter(&mut tools,mode,settings);
        let blocked_tools = if chat_mode {
            crate::agent::tool_policy::apply_chat_mode_tool_filter(&mut tools, true, &settings.chat.chat_mode)
        } else { crate::agent::tool_policy::apply_agent_plan_tool_filter(&mut tools,crate::plan::is_plan_mode(&conversation.agent_plan_state)) };
        let mut hooks=catalog.hooks.clone();
        let plugin_root=self.root.join("plugins");
        if let Ok(bytes)=std::fs::read(plugin_root.join("workflow-hooks.json")) {
            let value:Value=serde_json::from_slice(&bytes).map_err(|e|e.to_string())?;
            if value["enabled"]==true { hooks.extend(crate::workflow_hooks::parse(&value,"user",&plugin_root,&plugin_root.join("hook-data"))?); }
        }
        let workflow_hooks=(!chat_mode).then_some(crate::workflow_hooks::Runtime { hooks,cwd:working_directory.clone(),agent_type:"main".into(),prompt });
        Ok(PreparedNativeTools {tools,blocked_tools,skill_registry:catalog.skills.clone(),working_directory,workflow_hooks,video_analysis,video_reports})
    }
}
impl ShellHost for NativeToolExecutor {
    fn playwright_extension_token(&self)->String { self.settings().map(|s|s.chat_tools.playwright_extension_token).unwrap_or_default() }
    fn background_commands_handle(&self)->Arc<BackgroundCommandRegistry> { self.background.clone() }
}
fn text(content:String)->McpToolCallResult { McpToolCallResult {content,is_error:false,raw:Value::Null,artifacts:vec![],structured_content:None,follow_up_user_messages:vec![]} }
impl ToolExecutor for NativeToolExecutor {
    fn call<'a>(&'a self,ctx:&'a ToolExecutionContext<'a>,tool:&'a ChatToolDefinition,args:Value,skill_context:Option<&'a mut SkillRunContext>)->ToolExecutorFuture<'a> {
        Box::pin(async move {
            let settings=self.settings()?;
            if tool.source=="mcp" {
                let server=settings.chat_tools.servers.iter().find(|s|Some(s.id.as_str())==tool.server_id.as_deref()).ok_or("MCP server no longer exists")?;
                if !settings.chat_tools.enabled || !crate::mcp::server_policy::mcp_server_is_runtime_eligible(server) || (!server.enabled_tools.is_empty() && !server.enabled_tools.contains(&tool.name)) { return Err("MCP tool is disabled".into()); }
                return self.pool.call(server,&tool.name,args).await;
            }
            if tool.source=="skill" {
                if !settings.chat_tools.native_tools.skill_runtime { return Err("Skill runtime is disabled".into()); }
                let context=skill_context.as_deref().ok_or("Skill runtime requires the conversation catalog")?;
                let name=crate::skills::extract_skill_name(&args)?;
                return crate::skills::execute_skill_tool(&settings,context.registry(),tool,&name,Some(context)).await;
            }
            if tool.source!="native" { return Err(format!("Unsupported native host tool source: {}",tool.source)); }
            if tool.name==crate::todo::TODO_WRITE_TOOL_NAME {
                let outcome=crate::todo::apply_todo_write(args)?;
                let store=crate::conversation_files::ConversationStore::new(self.catalog_dir());
                let mut conversation=store.load(ctx.tool_conversation_id).map_err(|e|e.to_string())?;
                let revision=conversation.revision;
                conversation.agent_todo_state=outcome.state.clone();
                store.save(conversation,revision).map_err(|e|e.to_string())?;
                return Ok(crate::todo::tool_result(&outcome));
            }
            let workspace=self.workspace(ctx.tool_conversation_id,&settings)?;
            match tool.name.as_str() {
                "read" => {
                    let workspace=workspace.clone();
                    tokio::task::spawn_blocking(move || {
                        let path=args["path"].as_str().ok_or("read requires path")?;
                        let resolved=native_tools::resolve_tool_read_path(&workspace,path)?;
                        if resolved.is_dir() { return native_tools::list_dir(&workspace,&args).map(text); }
                        crate::mcp::native_results::read_file_tool_result(native_tools::read_file(&workspace,&args)?)
                    }).await.map_err(|e|e.to_string())?
                },
                "ls"=>native_tools::list_dir(&workspace,&args).map(text),
                "grep"=>native_tools::search_files(&workspace,&args).map(text),
                "glob"=>native_tools::glob_files(&workspace,&args).map(text),
                "write"|"edit"=> {
                    let write=tool.name=="write";
                    tokio::task::spawn_blocking(move || {
                        let result=if write {native_tools::write_file(&workspace,&args)?} else {native_tools::edit_file(&workspace,&args)?};
                        crate::mcp::native_results::file_mutation_tool_result(result)
                    }).await.map_err(|e|e.to_string())?
                },
                "bash"=>native_tools::run_command(&workspace,&args,Some(self),Some(ctx.tool_conversation_id)).await.map(text),
                "bash_output"=>native_tools::bash_output(self,&args,Some(ctx.tool_conversation_id)).await.map(text),
                "kill_background"=>native_tools::kill_background(self,&args,Some(ctx.tool_conversation_id)).map(text),
                "memory_read"|"memory_search"|"memory_modify"=> {
                    if !settings.chat_memory.enabled {return Err("Memory is disabled".into());}
                    let dir=crate::memory_files::memory_dir_at(&self.root)?;
                    match tool.name.as_str() {"memory_read"=>crate::memory_tools::tool_read_at(&dir,&args),"memory_search"=>crate::memory_tools::tool_search_at(&dir,&args),_=>crate::memory_tools::tool_modify_at(&dir,&args)}
                },
                "web_search"=> {
                    let query=args["query"].as_str().filter(|s|!s.trim().is_empty()).ok_or("web_search query is empty")?;
                    let results=self.search.search(&settings.lens.web_search,query).await?;
                    let mut result=text(crate::web_search::format_web_context(&results));
                    result.raw=serde_json::to_value(&results).map_err(|e|e.to_string())?;
                    result.structured_content=Some(json!({"type":"third_party_web_search","provider":crate::web_search::provider_label(settings.lens.web_search.provider),"queries":[query],"citations":results.iter().map(|r|json!({"title":r.title,"url":r.url,"snippet":r.content.chars().take(400).collect::<String>(),"published_date":r.published_date})).collect::<Vec<_>>()}));
                    Ok(result)
                },
                "web_fetch"=> {
                    let url=args["url"].as_str().unwrap_or("");
                    if url.starts_with("https://") && crate::mcp::search_policy::web_fetch_configured(&settings) {
                        if let Ok(page)=self.search.fetch(&settings.lens.web_search,url).await { if !page.text.trim().is_empty(){return Ok(text(crate::web_search::format_web_fetch(&page)));} }
                    }
                    native_tools::web_fetch(&self.http,&args).await.map(text)
                },
                other=>Err(format!("Native tool execution owner not connected: {other}")),
            }
        })
    }
}


/// Video cache belongs to one reply, while MCP and background processes belong to the app.
pub struct NativeRunTools<'a> {
    pub executor: &'a NativeToolExecutor,
    pub video: tokio::sync::Mutex<crate::video_analysis::VideoTool>,
}
impl ToolExecutor for NativeRunTools<'_> {
    fn call<'a>(&'a self, ctx: &'a ToolExecutionContext<'a>, tool: &'a ChatToolDefinition, args: Value, skill: Option<&'a mut SkillRunContext>) -> ToolExecutorFuture<'a> {
        Box::pin(async move {
            if tool.source != "mixer" || tool.name != "mixer_video_analysis" {
                return self.executor.call(ctx, tool, args, skill).await;
            }
            let settings = self.executor.settings()?;
            let host = crate::RuntimeHost::for_directory(&self.executor.catalog_dir().join("runtime"));
            for provider in &settings.providers {
                crate::RuntimeHost::shared(provider, &self.executor.catalog_dir().join("runtime"));
            }
            host.set_request_debug_enabled(settings.chat_tools.request_debug_enabled);
            let directory = self.executor.catalog_dir().join(format!("{}_attachments", ctx.conversation_id));
            self.video.lock().await.call_with_host(&settings, &*host, |conversation| {
                let start = crate::agent::context_projection::context_replay_start_index(conversation);
                let mut budget = crate::video::MAX_VIDEO_BYTES;
                conversation.messages.iter().skip(start).filter(|message| message.role == "user")
                    .map(|message| crate::attachment_files::user_message_parts(&message.content, &message.attachments, &directory, &mut budget)
                        .map(|parts| json!({"role":"user", "content":parts}))).collect()
            }, ctx, &args).await
        })
    }
}
