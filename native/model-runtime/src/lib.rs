//! Assembly of Kivio's original Rust model implementation for native desktop.
//! Wire encoding, parsing, retry policy and model parameters have one source.
#![allow(dead_code, unused_imports)]
#[path = "../../../src-tauri/src/chat/conversation_data.rs"]
pub mod conversation_data;
#[path = "../../../src-tauri/src/chat/model/endpoint_compatibility.rs"]
pub mod endpoint_compatibility;
#[path = "../../../src-tauri/src/chat/file_io.rs"]
pub mod file_io;
#[path = "../../../src-tauri/src/chat/model/host.rs"]
pub mod host;
#[path = "../../../src-tauri/src/chat/model/invocation.rs"]
pub mod invocation;
#[path = "../../../src-tauri/src/chat/model_database_data.rs"]
pub mod model_database;
#[path = "../../../src-tauri/src/chat/model_parameters.rs"]
pub mod model_metadata;
#[path = "../../../src-tauri/src/opencode_free.rs"]
pub mod opencode_free;
#[path = "../../../src-tauri/src/chat/retry_status.rs"]
pub mod protocol;
#[path = "../../../src-tauri/src/provider_request.rs"]
pub mod provider_request;
#[path = "../../../src-tauri/src/chat/provider_request_runtime.rs"]
pub mod provider_runtime;
#[path = "../../../src-tauri/src/provider_settings.rs"]
pub mod provider_settings;
#[path = "../../../src-tauri/src/chat/request_debug_data.rs"]
pub mod request_debug;
#[path = "../../../src-tauri/src/chat/model/responses.rs"]
pub mod responses;
#[path = "../../../src-tauri/src/api_runtime.rs"]
pub mod runtime;
#[path = "../../../src-tauri/src/tool_settings.rs"]
pub mod tool_settings;
#[path = "../../../src-tauri/src/settings_config.rs"]
pub mod settings_config;
pub mod native_preferences;
pub mod settings {
    pub use crate::provider_settings::*;
    pub use crate::tool_settings::*;
}
#[path = "../../../src-tauri/src/chat/model/sse.rs"]
pub mod sse;
#[path = "../../../src-tauri/src/mcp/tool_contract.rs"]
pub mod tool_contract;
#[path = "../../../src-tauri/src/api_transport.rs"]
pub mod transport;
#[path = "../../../src-tauri/src/chat/model/types.rs"]
pub mod types;
#[path = "../../../src-tauri/src/model_usage.rs"]
pub mod usage;
#[path = "../../../src-tauri/src/utils.rs"]
pub mod utils;
#[path = "../../../src-tauri/src/chat/video.rs"]
pub mod video;

#[path = "../../../src-tauri/src/chat/model/anthropic.rs"]
pub mod anthropic;
#[path = "../../../src-tauri/src/chat/model/dispatch.rs"]
pub mod dispatch;
#[path = "../../../src-tauri/src/chat/model/gemini.rs"]
pub mod gemini;
#[path = "../../../src-tauri/src/chat/model/openai.rs"]
pub mod openai;
pub use anthropic::AnthropicMessagesProvider;
pub use gemini::GeminiProvider;
pub use openai::OpenAiChatProvider;
pub use responses::OpenAiResponsesProvider;
pub use types::*;
pub mod api {
    pub use crate::transport::*;
    pub use crate::{runtime, transport};
}
pub mod mcp {
    pub use crate::tool_contract::ChatToolDefinition;
    pub mod types {
        pub use crate::tool_contract::*;
    }
}
pub mod chat {
    pub use crate::conversation_data as types;
    pub use crate::file_io as storage;
    pub use crate::{
        attachment_files, conversation_mutations, conversation_resources, image_prep,
        message_content, todo_history,
    };
    pub use crate::{
        conversation_data, model_database, model_metadata, protocol, provider_runtime,
        request_debug, video,
    };
    pub use crate::{hooks, todo};
    pub mod model {
        pub use crate::invocation::{generate_with_chat_provider, stream_with_chat_provider};
        pub use crate::types::*;
        pub use crate::{anthropic, gemini, host, invocation, openai, responses, types};
    }
}
// This host resolves only its explicitly isolated native preview identity.
#[path = "../../../src-tauri/src/app_identity.rs"]
pub mod app_identity;
pub mod app_data {
    const IDENTITY: crate::app_identity::AppIdentity =
        crate::app_identity::AppIdentity::NativePreview;
    pub fn app_data_dir() -> Option<std::path::PathBuf> {
        directories::BaseDirs::new().map(|base| base.data_dir().join(IDENTITY.identifier()))
    }
    pub fn is_dev() -> bool {
        IDENTITY.is_development()
    }
    pub fn oauth_service() -> &'static str {
        IDENTITY.oauth_service()
    }
    pub fn user_home_name() -> &'static str {
        IDENTITY.user_home_name()
    }
    pub fn product_name() -> &'static str {
        IDENTITY.product_name()
    }
}
#[path = "../../../src-tauri/src/provider_oauth_runtime.rs"]
pub mod provider_oauth;

#[path = "../../../src-tauri/src/chat/request_debug_owner.rs"]
mod request_debug_owner;
#[path = "../../../src-tauri/src/usage_ledger.rs"]
pub mod usage_ledger;

pub struct RuntimeHost {
    client: reqwest::Client,
    direct_client: reqwest::Client,
    runtime: provider_runtime::ProviderRequestRuntime,
    configured: std::sync::Mutex<Vec<settings::ModelProvider>>,
    usage_dir: std::path::PathBuf,
    usage_ledger: usage_ledger::UsageLedger,
    request_debug: request_debug_owner::RequestDebugState,
    debug_enabled: std::sync::atomic::AtomicBool,
}
impl RuntimeHost {
    pub fn for_directory(data_root: &std::path::Path) -> std::sync::Arc<Self> {
        static HOSTS: std::sync::OnceLock<
            std::sync::Mutex<
                std::collections::HashMap<std::path::PathBuf, std::sync::Arc<RuntimeHost>>,
            >,
        > = std::sync::OnceLock::new();
        let mut hosts = HOSTS
            .get_or_init(Default::default)
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        hosts
            .entry(data_root.to_path_buf())
            .or_insert_with(|| {
                let usage_dir = data_root.join("usage");
                std::sync::Arc::new(Self {
                    client: transport::build_http_client(),
                    direct_client: transport::build_direct_http_client(),
                    runtime: Default::default(),
                    configured: Default::default(),
                    request_debug: request_debug_owner::RequestDebugState::new(&usage_dir),
                    usage_ledger: Default::default(),
                    usage_dir,
                    debug_enabled: Default::default(),
                })
            })
            .clone()
    }
    pub fn shared(
        provider: &settings::ModelProvider,
        data_root: &std::path::Path,
    ) -> std::sync::Arc<Self> {
        let host = Self::for_directory(data_root);
        // Match the existing owner's update rule: another conversation does not reset
        // learned endpoints, key failover, or cooldowns for this application profile.
        let mut previous = host.configured.lock().unwrap_or_else(|e| e.into_inner());
        let mut next = previous.clone();
        if let Some(slot) = next.iter_mut().find(|p| p.id == provider.id) {
            *slot = provider.clone();
        } else {
            next.push(provider.clone());
        }
        host.runtime.sync_preferred_api_keys(&previous, &next);
        *previous = next;
        drop(previous);
        host
    }
    pub fn usage_stats(
        &self,
        query: usage_ledger::UsageStatsQuery,
    ) -> Result<usage_ledger::UsageStatsResponse, String> {
        self.usage_ledger.stats(&self.usage_dir, query)
    }
    pub fn clear_usage(&self) -> Result<(), String> {
        self.usage_ledger.clear(&self.usage_dir)
    }
    pub fn set_request_debug_enabled(&self, enabled: bool) {
        self.debug_enabled
            .store(enabled, std::sync::atomic::Ordering::Relaxed);
    }
    pub fn debug_snapshot(&self) -> Vec<request_debug::RequestDebugRecord> {
        self.request_debug.snapshot()
    }
    pub fn clear_debug(&self) {
        self.request_debug.clear();
    }
}
impl host::ModelHost for RuntimeHost {
    fn client_for(&self, provider: &settings::ModelProvider) -> &reqwest::Client {
        if provider.request.oauth.is_some() {
            return provider_oauth::inference_client(provider.request.use_system_proxy);
        }
        if provider.request.use_system_proxy {
            &self.client
        } else {
            &self.direct_client
        }
    }
    fn provider_runtime(&self) -> &provider_runtime::ProviderRequestRuntime {
        &self.runtime
    }
    fn request_debug_enabled(&self) -> bool {
        self.debug_enabled
            .load(std::sync::atomic::Ordering::Relaxed)
    }
    fn record_usage(&self, input: usage::UsageRecordInput<'_>) {
        usage_ledger::record_model_call_in(&self.usage_ledger, &self.usage_dir, input);
    }
    fn record_debug(&self, record: request_debug::RequestDebugRecord) {
        self.request_debug.record(record);
    }
}

pub fn request_max_tokens(provider: &settings::ModelProvider, model: &str) -> u32 {
    model_metadata::chat_max_output_tokens_on_wire(Some(provider), model, 16384)
}

#[path = "../../../src-tauri/src/chat/attachment_files.rs"]
pub mod attachment_files;
#[path = "../../../src-tauri/src/chat/conversation_mutations.rs"]
pub mod conversation_mutations;
#[path = "../../../src-tauri/src/chat/conversation_resources.rs"]
pub mod conversation_resources;
#[path = "../../../src-tauri/src/chat/image_prep.rs"]
pub mod image_prep;
#[path = "../../../src-tauri/src/chat/message_content.rs"]
pub mod message_content;
#[path = "../../../src-tauri/src/chat/todo_history.rs"]
pub mod todo_history;

#[path = "../../../src-tauri/src/chat/todo_runtime.rs"]
pub mod todo;

#[path = "../../../src-tauri/src/chat/hooks_runtime.rs"]
pub mod hooks;
#[path = "../../../src-tauri/src/proc.rs"]
pub mod proc;
pub mod native_tools {
    use std::path::{Path, PathBuf};
    use tokio::process::Command;
    #[cfg(target_os = "windows")]
    const CREATE_NO_WINDOW: u32 = 0x08000000;
    include!("../../../src-tauri/src/native_tools/shell_process.rs");
}

#[path = "../../../src-tauri/src/dock/terminal_runtime.rs"]
pub mod terminal;
#[path = "../../../src-tauri/src/utf8_locale.rs"]
pub mod utf8_locale;

#[path = "../../../src-tauri/src/chat/conversation_files.rs"]
pub mod conversation_files;
#[path = "../../../src-tauri/src/chat/group_catalog.rs"]
pub mod group_catalog;

#[path = "../../../src-tauri/src/chat/conversation_library.rs"]
pub mod conversation_library;
#[path = "../../../src-tauri/src/chat/conversation_search.rs"]
pub mod conversation_search;

#[path = "../../../src-tauri/src/chat/memory_files.rs"]
pub mod memory_files;

#[path = "../../../src-tauri/src/prompts.rs"]
pub mod prompts;
