/// Locale vars to apply on the process or a child (dock PTY). `LANG` is always
/// a UTF-8 locale; the `remove_*` flags lift a C/POSIX (or other non-UTF-8)
/// override that would otherwise shadow it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Utf8LocaleOverrides {
    pub lang: String,
    pub remove_lc_all: bool,
    pub remove_lc_ctype: bool,
}

/// Current-env snapshot → UTF-8 locale plan for a child. The dock PTY applies
/// this explicitly so a C `LC_ALL` inherited from a terminal-launched `dev`
/// session cannot keep shadowing `LANG`.
pub fn utf8_locale_overrides() -> Utf8LocaleOverrides {
    plan_utf8_locale(
        std::env::var("LANG").ok().as_deref(),
        std::env::var("LC_ALL").ok().as_deref(),
        std::env::var("LC_CTYPE").ok().as_deref(),
        apple_locale().as_deref(),
    )
}

/// Pick a UTF-8 `LANG` (keep / upgrade the existing one, else AppleLocale,
/// else `en_US.UTF-8`). Non-UTF-8 `LC_ALL` / `LC_CTYPE` must be lifted:
/// they outrank `LANG` and would leave BSD `ls` in C.
pub(crate) fn plan_utf8_locale(
    lang: Option<&str>,
    lc_all: Option<&str>,
    lc_ctype: Option<&str>,
    apple_locale: Option<&str>,
) -> Utf8LocaleOverrides {
    Utf8LocaleOverrides {
        lang: pick_utf8_lang(lang, apple_locale),
        remove_lc_all: lc_all.is_some_and(|v| !locale_is_utf8(v)),
        remove_lc_ctype: lc_ctype.is_some_and(|v| !locale_is_utf8(v)),
    }
}

pub(crate) fn locale_is_utf8(value: &str) -> bool {
    let upper = value.trim().to_ascii_uppercase();
    upper.contains("UTF-8") || upper.contains("UTF8")
}

pub(crate) fn is_c_or_posix(value: &str) -> bool {
    let trimmed = value.trim();
    trimmed.eq_ignore_ascii_case("C") || trimmed.eq_ignore_ascii_case("POSIX")
}

/// Upgrade an existing `LANG` to UTF-8 (`zh_CN` → `zh_CN.UTF-8`), or fall
/// through to AppleLocale / `en_US.UTF-8`. C/POSIX is not a real language tag.
pub(crate) fn pick_utf8_lang(lang: Option<&str>, apple_locale: Option<&str>) -> String {
    if let Some(upgraded) = lang.and_then(upgrade_lang) {
        return upgraded;
    }
    if let Some(apple) = apple_locale.and_then(normalize_apple_locale) {
        return format!("{apple}.UTF-8");
    }
    "en_US.UTF-8".to_string()
}

pub(crate) fn upgrade_lang(lang: &str) -> Option<String> {
    let trimmed = lang.trim();
    if trimmed.is_empty() || is_c_or_posix(trimmed) {
        return None;
    }
    if locale_is_utf8(trimmed) {
        return Some(trimmed.to_string());
    }
    match trimmed.rsplit_once('.') {
        Some((base, _)) => {
            let base = base.trim();
            if base.is_empty() || is_c_or_posix(base) {
                None
            } else {
                Some(format!("{base}.UTF-8"))
            }
        }
        None => Some(format!("{trimmed}.UTF-8")),
    }
}

/// `defaults read -g AppleLocale` → `zh_CN` / `zh_Hans_CN` / `en_US`.
/// Newer Apple IDs carry a script tag (`zh_Hans_CN`); libc locales on macOS
/// are the two-part form (`zh_CN.UTF-8`), so drop the script.
pub(crate) fn normalize_apple_locale(apple: &str) -> Option<String> {
    let parts: Vec<&str> = apple
        .trim()
        .split(['_', '-'])
        .filter(|p| !p.is_empty())
        .collect();
    let (lang, region) = match parts.as_slice() {
        [lang, region] if lang.len() == 2 && region.len() == 2 => (*lang, *region),
        [lang, _script, region] if lang.len() == 2 && region.len() == 2 => (*lang, *region),
        _ => return None,
    };
    Some(format!(
        "{}_{}",
        lang.to_ascii_lowercase(),
        region.to_ascii_uppercase()
    ))
}

#[cfg(target_os = "macos")]
pub(crate) fn apple_locale() -> Option<String> {
    let output = std::process::Command::new("defaults")
        .args(["read", "-g", "AppleLocale"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if text.is_empty() { None } else { Some(text) }
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn apple_locale() -> Option<String> {
    None
}
