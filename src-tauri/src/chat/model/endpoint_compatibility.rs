/// 判断错误是否是"端点拒绝 prompt_cache_key 字段"。仅在本次确实发了该字段时配合调用，
/// 错误体点名该字段即视为命中（NVIDIA NIM / 智谱 GLM 等严格端点的 400 会带上字段名）。
pub(super) fn error_rejects_prompt_cache_key(err: &str) -> bool {
    err.contains("prompt_cache_key")
}

/// 判断错误是否点名拒绝 `prompt_cache_retention`（long 的 24h 字段）。
pub(super) fn error_rejects_prompt_cache_retention(err: &str) -> bool {
    err.contains("prompt_cache_retention")
}
