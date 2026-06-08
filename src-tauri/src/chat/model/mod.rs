//! Provider-agnostic Chat model contracts and provider adapters.
//!
//! Runtime code should exchange `GenerateRequest`, `GenerateOutput`, and `StreamPart`.
//! Provider-specific JSON belongs inside this module's adapters.

pub mod anthropic;
pub mod openai;
pub mod types;

pub use anthropic::AnthropicMessagesProvider;
pub use openai::OpenAiChatProvider;
pub use types::*;
