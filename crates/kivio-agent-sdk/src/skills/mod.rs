//! Skill catalogs and read-only activation. Hosts choose sources and access policy.
//! Recommended tools are descriptive metadata and never grant tool authorization.

mod catalog;
mod discover;
pub mod parse;
mod runtime;
pub mod types;

pub use catalog::format_catalog;
pub use discover::{read_skill_record, read_skill_record_with};
pub use parse::{parse_skill_markdown, parse_skill_record};
pub use runtime::{
    activate_skill, extract_skill_name, substitute_arguments, SkillAccess, SkillArgs,
    SkillBodyReader, SkillRunContext, SkillTool,
};
pub use types::{
    normalize_trigger, record_triggers, slugify, SkillDetail, SkillFileEntry, SkillFileKind,
    SkillMeta, SkillRecord, SkillRegistry, SkillSource,
};
