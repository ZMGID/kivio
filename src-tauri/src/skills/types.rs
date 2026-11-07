pub use kivio_agent_sdk::skills::{SkillDetail, SkillMeta, SkillRecord, SkillRegistry};
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillListResult {
    pub success: bool,
    pub skills: Vec<SkillMeta>,
    #[serde(default)]
    pub warnings: Vec<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillReadResult {
    pub success: bool,
    pub skill: Option<SkillDetail>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillImportResult {
    pub success: bool,
    pub skill: Option<SkillMeta>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillOpenFolderResult {
    pub success: bool,
    pub path: Option<String>,
    pub error: Option<String>,
}
