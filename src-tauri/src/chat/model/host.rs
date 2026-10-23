//! Platform-independent capabilities consumed by model adapters.
use crate::chat::{provider_runtime::ProviderRequestRuntime, request_debug::RequestDebugRecord};
use crate::{settings::ModelProvider, usage::UsageRecordInput};
pub trait ModelHost: Send + Sync {
    fn client_for(&self, provider: &ModelProvider) -> &reqwest::Client;
    fn provider_runtime(&self) -> &ProviderRequestRuntime;
    fn request_debug_enabled(&self) -> bool;
    fn record_usage(&self, input: UsageRecordInput<'_>);
    fn record_debug(&self, record: RequestDebugRecord);
}
