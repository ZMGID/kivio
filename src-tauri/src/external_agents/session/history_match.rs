/// Whether a user prompt recorded by a CLI is the visible Kivio prompt. Kivio may have wrapped it
/// in `# Instructions … # User request` and appended attachment notes, so compare the user
/// section and accept a trailing note. A blank visible prompt (image-only) never matches by text.
pub(crate) fn native_prompt_matches(native_text: &str, visible_text: &str) -> bool {
    fn normalize_space(value: &str) -> String {
        value.split_whitespace().collect::<Vec<_>>().join(" ")
    }
    let expected = normalize_space(visible_text);
    if expected.is_empty() {
        return false;
    }
    let user = native_text
        .rsplit_once("# User request")
        .map(|(_, value)| value.trim_start_matches(['\r', '\n', ' ', '\t']))
        .unwrap_or(native_text);
    let actual = normalize_space(user);
    actual == expected
        || actual
            .strip_prefix(&expected)
            .is_some_and(|tail| tail.is_empty() || tail.starts_with(char::is_whitespace))
}

/// Where a CLI's native history stands relative to the visible Kivio history after a rewind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeRewindPoint {
    /// The native history already ends where the visible history does: nothing to drop.
    Aligned,
    /// Drop the native user turn at this index and every turn after it.
    DropFrom(usize),
    /// The histories cannot be matched safely; the native session must not be trimmed.
    Unmatched,
}

/// `native` holds the CLI's user prompts on its active history, oldest first. `visible` holds
/// Kivio's user prompts, oldest first, ending with the prompt about to be sent. A blank visible
/// prompt (image-only) is matched by position only.
pub fn native_rewind_point(native: &[String], visible: &[String]) -> NativeRewindPoint {
    let Some((_, kept)) = visible.split_last() else {
        return NativeRewindPoint::Unmatched;
    };
    let prefix_matches = native.len() >= kept.len()
        && native.iter().zip(kept).all(|(native, visible)| {
            visible.trim().is_empty() || native_prompt_matches(native, visible)
        });
    if !prefix_matches {
        NativeRewindPoint::Unmatched
    } else if native.len() == kept.len() {
        NativeRewindPoint::Aligned
    } else {
        NativeRewindPoint::DropFrom(kept.len())
    }
}
