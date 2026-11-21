use super::AnthropicThinkingMode;

/// Claude 思考线格式。对照官方 per-model 表：
/// https://platform.claude.com/docs/en/build-with-claude/thinking-troubleshooting
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClaudeThinkingKind {
    /// Claude 3 / 3.5：没有 thinking，发任何 type 都会 400。
    Unsupported,
    /// 4.5 及更早（含 3.7 / Opus 4 / Sonnet 4）：`type: "enabled"` + `budget_tokens`。
    Extended,
    /// 4.6+：`type: "adaptive"` + `output_config.effort`。4.7+ 发 `enabled` 会 400。
    Adaptive,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClaudeThinkingProfile {
    pub kind: ClaudeThinkingKind,
    /// 官方 `output_config.effort`。Opus 4.5 是 extended-only 里唯一认 effort 的模型。
    supports_output_effort: bool,
    supports_xhigh: bool,
    supports_max: bool,
    /// Off 的 wire 类型：Sonnet 5.5 用 between_tools，5 用 disabled；始终思考的模型省略。
    pub off_thinking_type: Option<&'static str>,
    /// These models reject non-default temperature, top_p and top_k.
    pub(super) forbid_sampling_parameters: bool,
}

impl ClaudeThinkingProfile {
    pub(super) fn thinking_mode(self) -> AnthropicThinkingMode {
        match self.kind {
            ClaudeThinkingKind::Unsupported => AnthropicThinkingMode::Unsupported,
            ClaudeThinkingKind::Extended => AnthropicThinkingMode::Extended {
                output_effort: self.supports_output_effort,
            },
            ClaudeThinkingKind::Adaptive => AnthropicThinkingMode::Adaptive,
        }
    }

    pub fn effort_levels(self) -> Vec<String> {
        if self.kind == ClaudeThinkingKind::Unsupported {
            return Vec::new();
        }
        let mut out = vec!["low".into(), "medium".into(), "high".into()];
        if self.supports_xhigh {
            out.push("xhigh".into());
        }
        if self.supports_max {
            out.push("max".into());
        }
        out
    }

    pub fn clamp_effort(self, level: &str) -> &str {
        match level {
            "xhigh" if !self.supports_xhigh => {
                if self.supports_max {
                    "max"
                } else {
                    "high"
                }
            }
            "max" if !self.supports_max => "high",
            "low" | "medium" | "high" | "xhigh" | "max" => level,
            "minimal" => "low",
            _ => "high",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ClaudeFamily {
    Opus,
    Sonnet,
    Haiku,
    Fable,
    Mythos,
}

impl ClaudeFamily {
    fn as_str(self) -> &'static str {
        match self {
            Self::Opus => "opus",
            Self::Sonnet => "sonnet",
            Self::Haiku => "haiku",
            Self::Fable => "fable",
            Self::Mythos => "mythos",
        }
    }
}

/// 从模型 id 识别 Claude 代际。覆盖官方 id、日期快照、Bedrock 点分 id、2api 短名（`opus-4.5`）。
/// 认不出则返回 None（适配器按 adaptive 兜底；UI 档位走安全子集）。
pub fn claude_thinking_profile(model: &str) -> Option<ClaudeThinkingProfile> {
    let id = normalize_claude_model_id(model);
    if id.contains("mythos-preview") {
        return Some(ClaudeThinkingProfile {
            kind: ClaudeThinkingKind::Adaptive,
            supports_output_effort: true,
            supports_xhigh: false,
            supports_max: true,
            off_thinking_type: None,
            forbid_sampling_parameters: true,
        });
    }
    if id.contains("3-7-sonnet") || id.contains("sonnet-3-7") {
        return Some(extended_budget_only());
    }
    if is_legacy_claude3(&id) {
        return Some(unsupported_thinking());
    }
    let (family, major, minor) = parse_claude_family_version(&id)?;
    if major < 4 {
        return Some(unsupported_thinking());
    }
    Some(profile_for_claude_version(family, major, minor))
}

fn is_legacy_claude3(id: &str) -> bool {
    id.contains("3-5-")
        || id.contains("sonnet-3-5")
        || id.contains("haiku-3-5")
        || id.contains("3-haiku")
        || id.contains("3-opus")
        || id.contains("3-sonnet")
}

fn unsupported_thinking() -> ClaudeThinkingProfile {
    ClaudeThinkingProfile {
        kind: ClaudeThinkingKind::Unsupported,
        supports_output_effort: false,
        supports_xhigh: false,
        supports_max: false,
        off_thinking_type: None,
        forbid_sampling_parameters: false,
    }
}

fn extended_budget_only() -> ClaudeThinkingProfile {
    ClaudeThinkingProfile {
        kind: ClaudeThinkingKind::Extended,
        supports_output_effort: false,
        supports_xhigh: false,
        supports_max: false,
        off_thinking_type: None,
        forbid_sampling_parameters: false,
    }
}

fn normalize_claude_model_id(model: &str) -> String {
    let name = model.trim().to_ascii_lowercase();
    let stripped = name.rsplit('/').next().unwrap_or(name.as_str());
    let stripped = stripped
        .strip_prefix("anthropic.")
        .unwrap_or(stripped)
        .strip_prefix("anthropic-")
        .unwrap_or(stripped);
    stripped.trim().replace(['.', ':'], "-")
}

fn parse_claude_family_version(id: &str) -> Option<(ClaudeFamily, u32, u32)> {
    for family in [
        ClaudeFamily::Opus,
        ClaudeFamily::Sonnet,
        ClaudeFamily::Haiku,
        ClaudeFamily::Fable,
        ClaudeFamily::Mythos,
    ] {
        let key = family.as_str();
        let rest = if let Some(stripped) = id.strip_prefix(&format!("{key}-")) {
            stripped
        } else if let Some(idx) = id.find(&format!("-{key}-")) {
            &id[idx + key.len() + 2..]
        } else {
            continue;
        };
        let mut parts = rest.split('-');
        let major = version_segment(parts.next()?)?;
        let minor = parts.next().and_then(version_segment).unwrap_or(0);
        return Some((family, major, minor));
    }
    None
}

fn version_segment(part: &str) -> Option<u32> {
    if (1..=2).contains(&part.len()) && part.bytes().all(|b| b.is_ascii_digit()) {
        part.parse().ok()
    } else {
        None
    }
}

fn version_at_least(major: u32, minor: u32, req_major: u32, req_minor: u32) -> bool {
    major > req_major || (major == req_major && minor >= req_minor)
}

/// 档位名单来自官方 Effort 页：
/// xhigh = Fable 5 / Mythos 5 / Opus 5 / Opus 4.8 / Opus 4.7 / Sonnet 5
/// max = 上述 + Mythos Preview / Opus 4.6 / Sonnet 4.6
/// effort 本体 = 上述 + Opus 4.5
fn profile_for_claude_version(
    family: ClaudeFamily,
    major: u32,
    minor: u32,
) -> ClaudeThinkingProfile {
    // Haiku 5.5 adds adaptive thinking and output effort; older Haiku uses budgets.
    if family == ClaudeFamily::Haiku && !version_at_least(major, minor, 5, 5) {
        return extended_budget_only();
    }
    let adaptive = match family {
        ClaudeFamily::Fable | ClaudeFamily::Mythos => major >= 5,
        ClaudeFamily::Opus | ClaudeFamily::Sonnet => version_at_least(major, minor, 4, 6),
        ClaudeFamily::Haiku => version_at_least(major, minor, 5, 5),
    };
    let kind = if adaptive {
        ClaudeThinkingKind::Adaptive
    } else {
        ClaudeThinkingKind::Extended
    };
    let supports_output_effort = match family {
        ClaudeFamily::Opus => version_at_least(major, minor, 4, 5),
        ClaudeFamily::Sonnet => version_at_least(major, minor, 4, 6),
        ClaudeFamily::Haiku => version_at_least(major, minor, 5, 5),
        ClaudeFamily::Fable | ClaudeFamily::Mythos => major >= 5,
    };
    let supports_xhigh = match family {
        ClaudeFamily::Fable | ClaudeFamily::Mythos => major >= 5,
        ClaudeFamily::Opus => version_at_least(major, minor, 4, 7),
        ClaudeFamily::Sonnet => major >= 5,
        ClaudeFamily::Haiku => version_at_least(major, minor, 5, 5),
    };
    let supports_max = match family {
        ClaudeFamily::Fable | ClaudeFamily::Mythos => major >= 5,
        ClaudeFamily::Opus | ClaudeFamily::Sonnet => version_at_least(major, minor, 4, 6),
        ClaudeFamily::Haiku => version_at_least(major, minor, 5, 5),
    };
    ClaudeThinkingProfile {
        kind,
        supports_output_effort,
        supports_xhigh,
        supports_max,
        off_thinking_type: match family {
            ClaudeFamily::Haiku if version_at_least(major, minor, 5, 5) => Some("disabled"),
            ClaudeFamily::Sonnet if version_at_least(major, minor, 5, 5) => Some("between_tools"),
            ClaudeFamily::Sonnet if major >= 5 => Some("disabled"),
            ClaudeFamily::Opus if major >= 5 && !version_at_least(major, minor, 5, 5) => {
                Some("disabled")
            }
            _ => None,
        },
        forbid_sampling_parameters: match family {
            ClaudeFamily::Fable | ClaudeFamily::Mythos => major >= 5,
            ClaudeFamily::Opus => version_at_least(major, minor, 4, 7),
            ClaudeFamily::Sonnet => major >= 5,
            ClaudeFamily::Haiku => version_at_least(major, minor, 5, 5),
        },
    }
}
