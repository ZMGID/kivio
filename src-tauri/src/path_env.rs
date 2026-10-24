//! Legacy process-wide environment initialization; child PATH discovery is shared.
include!("path_snapshot.rs");

#[cfg(target_os = "macos")]
pub fn enrich_path_macos() {
    use std::sync::Once;
    static DONE: Once = Once::new();
    DONE.call_once(|| {
        let current = std::env::var("PATH").unwrap_or_default();
        let login = login_shell_path();
        let defaults = common_dirs_macos(std::env::var_os("HOME").map(std::path::PathBuf::from));

        let merged = merge_paths_unix(&current, login.as_deref(), &defaults);
        if !merged.is_empty() {
            std::env::set_var("PATH", merged);
        }
    });
}

#[cfg(target_os = "windows")]
pub fn enrich_path_windows() {
    use std::sync::Once;
    static DONE: Once = Once::new();
    DONE.call_once(|| {
        let current = std::env::var("PATH").unwrap_or_default();
        let system = read_registry_path(true).map(|p| expand_env_vars(&p));
        let user = read_registry_path(false).map(|p| expand_env_vars(&p));
        let defaults = common_dirs_windows();

        // ① Registry + defaults → process PATH. Must land before the probe so the
        // probe can locate pwsh even when the process PATH was a stale snapshot.
        let merged = merge_paths_windows(
            &current,
            system.as_deref(),
            user.as_deref(),
            None,
            &defaults,
        );
        if !merged.is_empty() {
            std::env::set_var("PATH", &merged);
        }

        // ② Profile probe → merge live $env:PATH additions. On any failure/timeout
        // `profile_shell_path` returns None and we leave the phase-1 PATH untouched
        // (behaviour identical to the pre-profile version). The phase-1 result is
        // the `current` here, so version-manager dirs the registry can't see get
        // folded in; defaults are already present from phase 1.
        if let Some(profile) = profile_shell_path() {
            let remerged = merge_paths_windows(&merged, None, None, Some(&profile), &[]);
            if !remerged.is_empty() {
                std::env::set_var("PATH", remerged);
            }
        }
    });
}

#[cfg(target_os = "windows")]
pub fn refresh_path_now() {
    let current = std::env::var("PATH").unwrap_or_default();
    let system = read_registry_path(true).map(|p| expand_env_vars(&p));
    let user = read_registry_path(false).map(|p| expand_env_vars(&p));
    let defaults = common_dirs_windows();
    let merged = merge_paths_windows(
        &current,
        system.as_deref(),
        user.as_deref(),
        None,
        &defaults,
    );
    if !merged.is_empty() {
        std::env::set_var("PATH", merged);
    }
}

#[cfg(not(target_os = "windows"))]
pub fn refresh_path_now() {
    #[cfg(target_os = "macos")]
    {
        let current = std::env::var("PATH").unwrap_or_default();
        let defaults = common_dirs_macos(std::env::var_os("HOME").map(std::path::PathBuf::from));
        let merged = merge_paths_unix(&current, None, &defaults);
        if !merged.is_empty() {
            std::env::set_var("PATH", merged);
        }
    }
}

pub fn ensure_utf8_locale() {
    use std::sync::Once;
    static DONE: Once = Once::new();
    DONE.call_once(|| {
        let plan = utf8_locale_overrides();
        if plan.remove_lc_all {
            std::env::remove_var("LC_ALL");
        }
        if plan.remove_lc_ctype {
            std::env::remove_var("LC_CTYPE");
        }
        std::env::set_var("LANG", plan.lang);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn capture_stdout_timeout_terminates_and_reaps_child() {
        use std::process::{Command, Stdio};

        let mut command = Command::new("sh");
        command
            .args(["-c", "exec sleep 30"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());

        let started = std::time::Instant::now();
        assert_eq!(
            capture_stdout_with_timeout(command, std::time::Duration::from_millis(40)),
            None
        );
        assert!(
            started.elapsed() < std::time::Duration::from_secs(2),
            "timeout cleanup should kill and reap promptly"
        );
    }

    #[cfg(unix)]
    #[test]
    fn capture_stdout_returns_successful_output() {
        use std::process::{Command, Stdio};

        let mut command = Command::new("sh");
        command
            .args(["-c", "printf 'probe-ok'"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());

        assert_eq!(
            capture_stdout_with_timeout(command, std::time::Duration::from_secs(1)),
            Some("probe-ok".to_string())
        );
    }
    use std::path::PathBuf;

    #[test]
    fn push_unique_skips_empty_and_dupes() {
        let mut seen = HashSet::new();
        let mut out = Vec::new();
        let id = |s: &str| s.to_string();
        push_unique("/a", &mut seen, &mut out, id);
        push_unique("", &mut seen, &mut out, id);
        push_unique("  ", &mut seen, &mut out, id);
        push_unique("/a", &mut seen, &mut out, id);
        push_unique("/b", &mut seen, &mut out, id);
        assert_eq!(out, vec!["/a".to_string(), "/b".to_string()]);
    }

    #[test]
    fn common_dirs_macos_includes_homebrew() {
        let dirs = common_dirs_macos(None);
        assert!(dirs.iter().any(|d| d == "/opt/homebrew/bin"));
        assert!(dirs.iter().any(|d| d == "/usr/local/bin"));
    }

    #[test]
    fn common_dirs_macos_expands_home() {
        let dirs = common_dirs_macos(Some(PathBuf::from("/Users/tester")));
        assert!(dirs.iter().any(|d| d == "/Users/tester/.local/bin"));
        assert!(dirs.iter().any(|d| d == "/Users/tester/.cargo/bin"));
        assert!(dirs.iter().any(|d| d == "/Users/tester/.bun/bin"));
    }

    #[test]
    fn common_dirs_macos_skips_empty_home() {
        let dirs = common_dirs_macos(Some(PathBuf::from("")));
        assert!(!dirs.iter().any(|d| d.contains(".local/bin")));
    }

    /// Simulate the minimal Finder/Dock PATH and confirm merging folds in the
    /// common install dirs without dropping the originals, deduped + in order.
    /// Pure (no env mutation) so it can't pollute sibling tests.
    #[test]
    fn merge_unix_from_minimal_path_adds_common_dirs() {
        let current = "/usr/bin:/bin:/usr/sbin:/sbin";
        let defaults = common_dirs_macos(Some(PathBuf::from("/Users/tester")));

        // No login shell available -> defaults-only fallback path.
        let result = merge_paths_unix(current, None, &defaults);
        let segs: Vec<&str> = result.split(':').collect();

        // Originals preserved and first (order kept).
        assert_eq!(&segs[0..4], &["/usr/bin", "/bin", "/usr/sbin", "/sbin"]);
        // Common dirs folded in.
        assert!(segs.contains(&"/opt/homebrew/bin"));
        assert!(segs.contains(&"/usr/local/bin"));
        assert!(segs.contains(&"/Users/tester/.local/bin"));
        assert!(segs.contains(&"/Users/tester/.cargo/bin"));
        // No duplicates.
        let mut unique = segs.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(unique.len(), segs.len(), "PATH has duplicate segments");
    }

    /// Login-shell PATH entries are merged after the current PATH but before
    /// defaults, and overlap is deduped.
    #[test]
    fn merge_unix_includes_login_shell_path() {
        let current = "/usr/bin:/bin";
        let login = "/opt/homebrew/bin:/usr/bin"; // /usr/bin overlaps
        let defaults = common_dirs_macos(None);

        let result = merge_paths_unix(current, Some(login), &defaults);
        let segs: Vec<&str> = result.split(':').collect();

        assert_eq!(segs[0], "/usr/bin");
        assert_eq!(segs[1], "/bin");
        // login-only entry comes right after current PATH, before /usr/local/bin default.
        assert_eq!(segs[2], "/opt/homebrew/bin");
        // /usr/bin not duplicated.
        assert_eq!(segs.iter().filter(|s| **s == "/usr/bin").count(), 1);
    }

    // ----- Windows merge / expand (pure helpers; compiled & tested on all OSes) -----

    /// A stale process PATH plus the current registry hives folds in the newer
    /// dirs, process PATH stays first, defaults come last, deduped + in order.
    #[test]
    fn merge_windows_adds_registry_and_defaults() {
        let current = "C:\\Windows\\system32;C:\\Windows";
        let system = "C:\\Windows\\system32;C:\\Program Files\\Git\\cmd";
        let user = "C:\\Users\\tester\\AppData\\Roaming\\npm";
        let defaults = vec!["C:\\Users\\tester\\.cargo\\bin".to_string()];

        let result = merge_paths_windows(current, Some(system), Some(user), None, &defaults);
        let segs: Vec<&str> = result.split(';').collect();

        // Process PATH preserved and first.
        assert_eq!(segs[0], "C:\\Windows\\system32");
        assert_eq!(segs[1], "C:\\Windows");
        // System-only entry folded in (system32 deduped).
        assert!(segs.contains(&"C:\\Program Files\\Git\\cmd"));
        // User entry folded in.
        assert!(segs.contains(&"C:\\Users\\tester\\AppData\\Roaming\\npm"));
        // Default folded in last.
        assert_eq!(segs.last(), Some(&"C:\\Users\\tester\\.cargo\\bin"));
        // No case-insensitive duplicates.
        let lowered: Vec<String> = segs.iter().map(|s| s.to_ascii_lowercase()).collect();
        let mut unique = lowered.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(unique.len(), lowered.len(), "PATH has duplicate segments");
    }

    /// Windows path dedup is case-insensitive; first spelling wins.
    #[test]
    fn merge_windows_dedups_case_insensitively() {
        let current = "C:\\Windows\\System32";
        let user = "c:\\windows\\system32"; // same dir, different case
        let result = merge_paths_windows(current, None, Some(user), None, &[]);
        assert_eq!(result, "C:\\Windows\\System32");
    }

    /// Trailing separators / empty segments are dropped, not turned into "".
    #[test]
    fn merge_windows_drops_empty_segments() {
        let current = "C:\\a;;C:\\b;";
        let result = merge_paths_windows(current, None, None, None, &[]);
        assert_eq!(result, "C:\\a;C:\\b");
    }

    /// The profile PATH is merged after user (registry) but before defaults, and
    /// overlap with earlier sources is deduped case-insensitively.
    #[test]
    fn merge_windows_includes_profile_source() {
        let current = "C:\\Windows\\system32";
        let user = "C:\\Users\\tester\\AppData\\Roaming\\npm";
        // fnm's per-shell dir plus an overlap with current (different case).
        let profile =
            "C:\\Users\\tester\\AppData\\Local\\fnm_multishells\\123_456;c:\\windows\\system32";
        let defaults = vec!["C:\\Users\\tester\\.cargo\\bin".to_string()];

        let result = merge_paths_windows(current, None, Some(user), Some(profile), &defaults);
        let segs: Vec<&str> = result.split(';').collect();

        // Order: current, then user, then profile-only entry, then default last.
        assert_eq!(segs[0], "C:\\Windows\\system32");
        assert_eq!(segs[1], "C:\\Users\\tester\\AppData\\Roaming\\npm");
        assert_eq!(
            segs[2],
            "C:\\Users\\tester\\AppData\\Local\\fnm_multishells\\123_456"
        );
        assert_eq!(segs.last(), Some(&"C:\\Users\\tester\\.cargo\\bin"));
        // system32 not duplicated despite the case-different profile entry.
        assert_eq!(
            segs.iter()
                .filter(|s| s.eq_ignore_ascii_case("C:\\Windows\\system32"))
                .count(),
            1
        );
    }

    /// `profile = None` reproduces the pre-profile behaviour exactly (registry +
    /// defaults only).
    #[test]
    fn merge_windows_profile_none_matches_registry_only() {
        let current = "C:\\Windows\\system32";
        let system = "C:\\Program Files\\Git\\cmd";
        let user = "C:\\Users\\tester\\AppData\\Roaming\\npm";
        let defaults = vec!["C:\\Users\\tester\\.cargo\\bin".to_string()];

        let with_none = merge_paths_windows(current, Some(system), Some(user), None, &defaults);
        let with_empty =
            merge_paths_windows(current, Some(system), Some(user), Some(""), &defaults);
        assert_eq!(with_none, with_empty);
        assert_eq!(
            with_none,
            "C:\\Windows\\system32;C:\\Program Files\\Git\\cmd;C:\\Users\\tester\\AppData\\Roaming\\npm;C:\\Users\\tester\\.cargo\\bin"
        );
    }

    // ----- parse_profile_path_output (pure; compiled & tested on all OSes) -----

    #[test]
    fn parse_profile_path_accepts_normal_path() {
        let out = "C:\\Windows\\system32;C:\\Users\\tester\\AppData\\Roaming\\npm\n";
        assert_eq!(
            parse_profile_path_output(out),
            Some("C:\\Windows\\system32;C:\\Users\\tester\\AppData\\Roaming\\npm".to_string())
        );
    }

    #[test]
    fn parse_profile_path_takes_last_line_after_banner() {
        // A profile may print greetings before the PATH is echoed on the last line.
        let out = "Loading fnm...\nWelcome back!\nC:\\a;C:\\b\n\n";
        assert_eq!(
            parse_profile_path_output(out),
            Some("C:\\a;C:\\b".to_string())
        );
    }

    #[test]
    fn parse_profile_path_accepts_single_drive_rooted() {
        // A single-entry PATH has no ';' but is drive-rooted.
        assert_eq!(
            parse_profile_path_output("C:\\Program Files\\nodejs\n"),
            Some("C:\\Program Files\\nodejs".to_string())
        );
    }

    #[test]
    fn parse_profile_path_rejects_plain_text() {
        // Profile chatter with no ';' and not drive-rooted must be rejected.
        assert_eq!(parse_profile_path_output("fnm is ready\n"), None);
    }

    #[test]
    fn parse_profile_path_rejects_empty() {
        assert_eq!(parse_profile_path_output(""), None);
        assert_eq!(parse_profile_path_output("\n\n  \n"), None);
    }

    #[test]
    fn is_drive_rooted_matches_prefix() {
        assert!(is_drive_rooted("C:\\Users"));
        assert!(is_drive_rooted("d:\\x"));
        assert!(!is_drive_rooted("C:/Users")); // forward slash not a Windows root
        assert!(!is_drive_rooted("\\\\server\\share")); // UNC, no drive letter
        assert!(!is_drive_rooted("relative\\path"));
        assert!(!is_drive_rooted("C:"));
    }

    #[test]
    fn expand_env_vars_replaces_known() {
        let lookup = |name: &str| match name {
            "USERPROFILE" => Some("C:\\Users\\tester".to_string()),
            "APPDATA" => Some("C:\\Users\\tester\\AppData\\Roaming".to_string()),
            _ => None,
        };
        let out = expand_env_vars_with("%USERPROFILE%\\.cargo\\bin;%APPDATA%\\npm", &lookup);
        assert_eq!(
            out,
            "C:\\Users\\tester\\.cargo\\bin;C:\\Users\\tester\\AppData\\Roaming\\npm"
        );
    }

    #[test]
    fn expand_env_vars_keeps_unknown_literal() {
        let lookup = |_: &str| None;
        let out = expand_env_vars_with("%NOPE%\\bin", &lookup);
        assert_eq!(out, "%NOPE%\\bin");
    }

    #[test]
    fn expand_env_vars_handles_no_vars_and_double_percent() {
        let lookup = |_: &str| Some("X".to_string());
        assert_eq!(
            expand_env_vars_with("C:\\plain\\path", &lookup),
            "C:\\plain\\path"
        );
        // "%%" is a literal percent, not a lookup.
        assert_eq!(expand_env_vars_with("100%%done", &lookup), "100%done");
    }

    #[test]
    fn expand_env_vars_unterminated_percent_is_literal() {
        let lookup = |_: &str| Some("X".to_string());
        assert_eq!(expand_env_vars_with("C:\\50%off", &lookup), "C:\\50%off");
    }

    // ----- UTF-8 locale plan (pure; compiled & tested on all OSes) -----

    #[test]
    fn locale_is_utf8_accepts_common_spellings() {
        assert!(locale_is_utf8("en_US.UTF-8"));
        assert!(locale_is_utf8("zh_CN.utf8"));
        assert!(locale_is_utf8("C.UTF-8"));
        assert!(locale_is_utf8("UTF-8"));
        assert!(!locale_is_utf8("C"));
        assert!(!locale_is_utf8("POSIX"));
        assert!(!locale_is_utf8("en_US"));
        assert!(!locale_is_utf8("en_US.ISO8859-1"));
        assert!(!locale_is_utf8(""));
    }

    #[test]
    fn plan_empty_env_uses_apple_locale_then_en_us() {
        let from_apple = plan_utf8_locale(None, None, None, Some("zh_CN"));
        assert_eq!(
            from_apple,
            Utf8LocaleOverrides {
                lang: "zh_CN.UTF-8".into(),
                remove_lc_all: false,
                remove_lc_ctype: false,
            }
        );
        let fallback = plan_utf8_locale(None, None, None, None);
        assert_eq!(fallback.lang, "en_US.UTF-8");
    }

    #[test]
    fn plan_upgrades_lang_without_encoding() {
        let plan = plan_utf8_locale(Some("zh_CN"), None, None, None);
        assert_eq!(plan.lang, "zh_CN.UTF-8");
        assert!(!plan.remove_lc_all);
    }

    #[test]
    fn plan_rewrites_legacy_charset() {
        let plan = plan_utf8_locale(Some("en_US.ISO8859-1"), None, None, None);
        assert_eq!(plan.lang, "en_US.UTF-8");
    }

    #[test]
    fn plan_keeps_existing_utf8_lang() {
        let plan = plan_utf8_locale(Some("zh_TW.UTF-8"), None, None, Some("en_US"));
        assert_eq!(plan.lang, "zh_TW.UTF-8");
        assert!(!plan.remove_lc_all);
        assert!(!plan.remove_lc_ctype);
    }

    #[test]
    fn plan_c_lang_falls_through_to_apple() {
        let plan = plan_utf8_locale(Some("C"), None, None, Some("zh_Hans_CN"));
        assert_eq!(plan.lang, "zh_CN.UTF-8");
    }

    #[test]
    fn plan_lifts_blocking_lc_all_and_lc_ctype() {
        let plan = plan_utf8_locale(Some("en_US.UTF-8"), Some("C"), Some("POSIX"), None);
        assert_eq!(plan.lang, "en_US.UTF-8");
        assert!(plan.remove_lc_all);
        assert!(plan.remove_lc_ctype);
    }

    #[test]
    fn plan_leaves_utf8_overrides_in_place() {
        let plan = plan_utf8_locale(None, Some("C.UTF-8"), Some("zh_CN.UTF-8"), None);
        assert!(!plan.remove_lc_all);
        assert!(!plan.remove_lc_ctype);
        assert_eq!(plan.lang, "en_US.UTF-8");
    }

    #[test]
    fn normalize_apple_locale_drops_script_tag() {
        assert_eq!(normalize_apple_locale("zh_Hans_CN"), Some("zh_CN".into()));
        assert_eq!(normalize_apple_locale("zh_Hant_TW"), Some("zh_TW".into()));
        assert_eq!(normalize_apple_locale("en_US"), Some("en_US".into()));
        assert_eq!(normalize_apple_locale("zh-Hans-CN"), Some("zh_CN".into()));
        assert_eq!(normalize_apple_locale("en"), None);
        assert_eq!(normalize_apple_locale(""), None);
    }
}
