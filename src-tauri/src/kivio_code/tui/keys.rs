//! 按键解码 —— PI `keys.ts` 端口。
//!
//! 把终端送来的字节序列解码成按键标识（如 `"ctrl+c"`、`"up"`、`"shift+enter"`），并提供
//! [`matches_key`]（输入是否匹配某 key id）。支持 Kitty CSI-u、xterm modifyOtherKeys、legacy
//! 序列三套，以及修饰位掩码、numpad 归一、shifted-letter identity、非拉丁布局 base-layout-key
//! 匹配、ctrl-char 公式（`code & 0x1f`）等。
//!
//! 设计上避免 PI 的模块级全局可变状态：Kitty 协议是否激活作为显式参数 `kitty_active` 传入；
//! 事件类型（press/repeat/release）由独立的 [`is_key_release`] / [`is_key_repeat`] 探测。

// 修饰位掩码（与 Kitty 协议一致）
const MOD_SHIFT: u32 = 1;
const MOD_ALT: u32 = 2;
const MOD_CTRL: u32 = 4;
const MOD_SUPER: u32 = 8;
const LOCK_MASK: u32 = 64 + 128; // Caps Lock + Num Lock

// 关键码点
const CP_ESCAPE: i64 = 27;
const CP_TAB: i64 = 9;
const CP_ENTER: i64 = 13;
const CP_SPACE: i64 = 32;
const CP_BACKSPACE: i64 = 127;
const CP_KP_ENTER: i64 = 57414;

// 方向键 / 功能键用负数码点（与 PI 一致）
const ARROW_UP: i64 = -1;
const ARROW_DOWN: i64 = -2;
const ARROW_RIGHT: i64 = -3;
const ARROW_LEFT: i64 = -4;
const FN_DELETE: i64 = -10;
const FN_INSERT: i64 = -11;
const FN_PAGE_UP: i64 = -12;
const FN_PAGE_DOWN: i64 = -13;
const FN_HOME: i64 = -14;
const FN_END: i64 = -15;

const SYMBOL_KEYS: &[char] = &[
    '`', '-', '=', '[', ']', '\\', ';', '\'', ',', '.', '/', '!', '@', '#', '$', '%', '^', '&', '*', '(', ')', '_', '+',
    '|', '~', '{', '}', ':', '<', '>', '?',
];

fn is_symbol_key(c: char) -> bool {
    SYMBOL_KEYS.contains(&c)
}

/// Kitty 协议下 numpad / functional 码点 → 等价基础码点。
fn normalize_kitty_functional(cp: i64) -> i64 {
    match cp {
        57399 => 48,
        57400 => 49,
        57401 => 50,
        57402 => 51,
        57403 => 52,
        57404 => 53,
        57405 => 54,
        57406 => 55,
        57407 => 56,
        57408 => 57,
        57409 => 46,
        57410 => 47,
        57411 => 42,
        57412 => 45,
        57413 => 43,
        57415 => 61,
        57416 => 44,
        57417 => ARROW_LEFT,
        57418 => ARROW_RIGHT,
        57419 => ARROW_UP,
        57420 => ARROW_DOWN,
        57421 => FN_PAGE_UP,
        57422 => FN_PAGE_DOWN,
        57423 => FN_HOME,
        57424 => FN_END,
        57425 => FN_INSERT,
        57426 => FN_DELETE,
        other => other,
    }
}

/// shift + 大写字母 → 小写字母 identity（A-Z + shift ≡ a-z）。
fn normalize_shifted_letter_identity(cp: i64, modifier: u32) -> i64 {
    let eff = modifier & !LOCK_MASK;
    if (eff & MOD_SHIFT) != 0 && (65..=90).contains(&cp) {
        return cp + 32;
    }
    cp
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyEventType {
    Press,
    Repeat,
    Release,
}

struct ParsedKitty {
    codepoint: i64,
    base_layout_key: Option<i64>,
    modifier: u32,
}

struct ParsedMok {
    codepoint: i64,
    modifier: u32,
}

/// 探测是否 Kitty 释放事件（含 `:3`）。粘贴内容不算。
pub fn is_key_release(data: &str) -> bool {
    if data.contains("\x1b[200~") {
        return false;
    }
    const PATS: &[&str] = &[":3u", ":3~", ":3A", ":3B", ":3C", ":3D", ":3H", ":3F"];
    PATS.iter().any(|p| data.contains(p))
}

/// 探测是否 Kitty 重复事件（含 `:2`）。
pub fn is_key_repeat(data: &str) -> bool {
    if data.contains("\x1b[200~") {
        return false;
    }
    const PATS: &[&str] = &[":2u", ":2~", ":2A", ":2B", ":2C", ":2D", ":2H", ":2F"];
    PATS.iter().any(|p| data.contains(p))
}

/// 探测最后事件类型（press/repeat/release）。
pub fn event_type(data: &str) -> KeyEventType {
    if is_key_release(data) {
        KeyEventType::Release
    } else if is_key_repeat(data) {
        KeyEventType::Repeat
    } else {
        KeyEventType::Press
    }
}

// ---- 手写的小型序列解析（替代 PI 的正则） ----

/// 解析数字串，返回 (value, bytes_consumed)。
fn take_number(chars: &[char], start: usize) -> (Option<i64>, usize) {
    let mut i = start;
    while i < chars.len() && chars[i].is_ascii_digit() {
        i += 1;
    }
    if i == start {
        return (None, 0);
    }
    let s: String = chars[start..i].iter().collect();
    (s.parse().ok(), i - start)
}

/// 解析 Kitty CSI-u：`\x1b[<cp>(:<shifted>)?(:<base>)?(;<mod>)?(:<event>)?u`。
fn parse_kitty_sequence(data: &str) -> Option<ParsedKitty> {
    let chars: Vec<char> = data.chars().collect();
    // CSI-u
    if chars.len() >= 4 && chars[0] == '\x1b' && chars[1] == '[' && *chars.last()? == 'u' {
        let mut i = 2;
        let (cp, n) = take_number(&chars, i);
        let cp = cp?;
        i += n;
        let mut shifted: Option<i64> = None;
        let mut base: Option<i64> = None;
        // 可选 :shifted
        if chars.get(i) == Some(&':') {
            i += 1;
            let (s, n) = take_number(&chars, i);
            shifted = s;
            i += n;
            // 可选 :base
            if chars.get(i) == Some(&':') {
                i += 1;
                let (b, n) = take_number(&chars, i);
                base = b;
                i += n;
            }
        }
        let mut modv = 1i64;
        if chars.get(i) == Some(&';') {
            i += 1;
            let (m, n) = take_number(&chars, i);
            if let Some(m) = m {
                modv = m;
            }
            i += n;
            // 可选 :event
            if chars.get(i) == Some(&':') {
                i += 1;
                let (_e, n) = take_number(&chars, i);
                i += n;
            }
        }
        // 必须紧接 'u' 结束
        if i == chars.len() - 1 && chars[i] == 'u' {
            let _ = shifted;
            return Some(ParsedKitty {
                codepoint: cp,
                base_layout_key: base,
                modifier: (modv - 1) as u32,
            });
        }
        return None;
    }

    // 方向键带修饰：\x1b[1;<mod>(:<event>)?[ABCD]
    if chars.len() >= 5 && chars[0] == '\x1b' && chars[1] == '[' && chars[2] == '1' && chars[3] == ';' {
        let last = *chars.last()?;
        if matches!(last, 'A' | 'B' | 'C' | 'D') {
            let (m, _n) = take_number(&chars, 4);
            if let Some(modv) = m {
                let cp = match last {
                    'A' => ARROW_UP,
                    'B' => ARROW_DOWN,
                    'C' => ARROW_RIGHT,
                    'D' => ARROW_LEFT,
                    _ => unreachable!(),
                };
                return Some(ParsedKitty { codepoint: cp, base_layout_key: None, modifier: (modv - 1) as u32 });
            }
        }
        if matches!(last, 'H' | 'F') {
            let (m, _n) = take_number(&chars, 4);
            if let Some(modv) = m {
                let cp = if last == 'H' { FN_HOME } else { FN_END };
                return Some(ParsedKitty { codepoint: cp, base_layout_key: None, modifier: (modv - 1) as u32 });
            }
        }
    }

    // 功能键：\x1b[<num>(;<mod>)?(:<event>)?~
    if chars.len() >= 3 && chars[0] == '\x1b' && chars[1] == '[' && *chars.last()? == '~' {
        let mut i = 2;
        let (num, n) = take_number(&chars, i);
        let num = num?;
        i += n;
        let mut modv = 1i64;
        if chars.get(i) == Some(&';') {
            i += 1;
            let (m, _n) = take_number(&chars, i);
            if let Some(m) = m {
                modv = m;
            }
        }
        let cp = match num {
            2 => Some(FN_INSERT),
            3 => Some(FN_DELETE),
            5 => Some(FN_PAGE_UP),
            6 => Some(FN_PAGE_DOWN),
            7 => Some(FN_HOME),
            8 => Some(FN_END),
            _ => None,
        };
        if let Some(cp) = cp {
            return Some(ParsedKitty { codepoint: cp, base_layout_key: None, modifier: (modv - 1) as u32 });
        }
    }
    None
}

/// 解析 xterm modifyOtherKeys：`\x1b[27;<mod>;<code>~`。
fn parse_mok(data: &str) -> Option<ParsedMok> {
    let chars: Vec<char> = data.chars().collect();
    if chars.len() < 7 || chars[0] != '\x1b' || chars[1] != '[' || *chars.last()? != '~' {
        return None;
    }
    let prefix: String = chars[2..4].iter().collect();
    if prefix != "27" {
        // 必须以 "27;" 开头
        if !(chars[2] == '2' && chars[3] == '7' && chars.get(4) == Some(&';')) {
            return None;
        }
    }
    if !(chars[2] == '2' && chars[3] == '7' && chars[4] == ';') {
        return None;
    }
    let (modv, n1) = take_number(&chars, 5);
    let modv = modv?;
    let mut i = 5 + n1;
    if chars.get(i) != Some(&';') {
        return None;
    }
    i += 1;
    let (code, _n2) = take_number(&chars, i);
    let code = code?;
    Some(ParsedMok { codepoint: code, modifier: (modv - 1) as u32 })
}

fn matches_kitty_sequence(data: &str, expected_cp: i64, expected_mod: u32) -> bool {
    let Some(parsed) = parse_kitty_sequence(data) else { return false };
    let actual_mod = parsed.modifier & !LOCK_MASK;
    let exp_mod = expected_mod & !LOCK_MASK;
    if actual_mod != exp_mod {
        return false;
    }
    let norm = normalize_shifted_letter_identity(normalize_kitty_functional(parsed.codepoint), parsed.modifier);
    let norm_exp = normalize_shifted_letter_identity(normalize_kitty_functional(expected_cp), expected_mod);
    if norm == norm_exp {
        return true;
    }
    // 非拉丁布局：base-layout-key 匹配（仅当码点不是已知拉丁字母 / 符号时）
    if let Some(base) = parsed.base_layout_key {
        if base == expected_cp {
            let is_latin = (97..=122).contains(&norm);
            let is_symbol = char::from_u32(norm as u32).map(is_symbol_key).unwrap_or(false);
            if !is_latin && !is_symbol {
                return true;
            }
        }
    }
    false
}

fn matches_mok(data: &str, expected_code: i64, expected_mod: u32) -> bool {
    match parse_mok(data) {
        Some(p) => p.codepoint == expected_code && p.modifier == expected_mod,
        None => false,
    }
}

fn matches_printable_mok(data: &str, expected_code: i64, expected_mod: u32) -> bool {
    if expected_mod == 0 {
        return false;
    }
    let Some(p) = parse_mok(data) else { return false };
    if p.modifier != expected_mod {
        return false;
    }
    normalize_shifted_letter_identity(p.codepoint, p.modifier)
        == normalize_shifted_letter_identity(expected_code, expected_mod)
}

/// ctrl+key 的控制字符（`code & 0x1f`），仅对字母与 `[ \ ] _ -` 有效。
fn raw_ctrl_char(key: char) -> Option<char> {
    let c = key.to_ascii_lowercase();
    let code = c as u32;
    if (97..=122).contains(&code) || matches!(c, '[' | '\\' | ']' | '_') {
        return char::from_u32(code & 0x1f);
    }
    if c == '-' {
        return char::from_u32(31);
    }
    None
}

// legacy 序列表（简化但覆盖常见终端）
fn legacy_sequences(key: &str) -> &'static [&'static str] {
    match key {
        "up" => &["\x1b[A", "\x1bOA"],
        "down" => &["\x1b[B", "\x1bOB"],
        "right" => &["\x1b[C", "\x1bOC"],
        "left" => &["\x1b[D", "\x1bOD"],
        "home" => &["\x1b[H", "\x1bOH", "\x1b[1~", "\x1b[7~"],
        "end" => &["\x1b[F", "\x1bOF", "\x1b[4~", "\x1b[8~"],
        "insert" => &["\x1b[2~"],
        "delete" => &["\x1b[3~"],
        "pageup" => &["\x1b[5~", "\x1b[[5~"],
        "pagedown" => &["\x1b[6~", "\x1b[[6~"],
        "clear" => &["\x1b[E", "\x1bOE"],
        "f1" => &["\x1bOP", "\x1b[11~", "\x1b[[A"],
        "f2" => &["\x1bOQ", "\x1b[12~", "\x1b[[B"],
        "f3" => &["\x1bOR", "\x1b[13~", "\x1b[[C"],
        "f4" => &["\x1bOS", "\x1b[14~", "\x1b[[D"],
        "f5" => &["\x1b[15~", "\x1b[[E"],
        "f6" => &["\x1b[17~"],
        "f7" => &["\x1b[18~"],
        "f8" => &["\x1b[19~"],
        "f9" => &["\x1b[20~"],
        "f10" => &["\x1b[21~"],
        "f11" => &["\x1b[23~"],
        "f12" => &["\x1b[24~"],
        _ => &[],
    }
}

fn legacy_shift_sequences(key: &str) -> &'static [&'static str] {
    match key {
        "up" => &["\x1b[a"],
        "down" => &["\x1b[b"],
        "right" => &["\x1b[c"],
        "left" => &["\x1b[d"],
        "clear" => &["\x1b[e"],
        "insert" => &["\x1b[2$"],
        "delete" => &["\x1b[3$"],
        "pageup" => &["\x1b[5$"],
        "pagedown" => &["\x1b[6$"],
        "home" => &["\x1b[7$"],
        "end" => &["\x1b[8$"],
        _ => &[],
    }
}

fn legacy_ctrl_sequences(key: &str) -> &'static [&'static str] {
    match key {
        "up" => &["\x1bOa"],
        "down" => &["\x1bOb"],
        "right" => &["\x1bOc"],
        "left" => &["\x1bOd"],
        "clear" => &["\x1bOe"],
        "insert" => &["\x1b[2^"],
        "delete" => &["\x1b[3^"],
        "pageup" => &["\x1b[5^"],
        "pagedown" => &["\x1b[6^"],
        "home" => &["\x1b[7^"],
        "end" => &["\x1b[8^"],
        _ => &[],
    }
}

fn matches_legacy(data: &str, seqs: &[&str]) -> bool {
    seqs.contains(&data)
}

fn matches_legacy_modifier(data: &str, key: &str, modifier: u32) -> bool {
    if modifier == MOD_SHIFT {
        return matches_legacy(data, legacy_shift_sequences(key));
    }
    if modifier == MOD_CTRL {
        return matches_legacy(data, legacy_ctrl_sequences(key));
    }
    false
}

struct ParsedKeyId {
    key: String,
    ctrl: bool,
    shift: bool,
    alt: bool,
    super_: bool,
}

fn parse_key_id(key_id: &str) -> Option<ParsedKeyId> {
    let lower = key_id.to_lowercase();
    let parts: Vec<&str> = lower.split('+').collect();
    let key = (*parts.last()?).to_string();
    if key.is_empty() {
        return None;
    }
    Some(ParsedKeyId {
        ctrl: parts.contains(&"ctrl"),
        shift: parts.contains(&"shift"),
        alt: parts.contains(&"alt"),
        super_: parts.contains(&"super"),
        key,
    })
}

/// 输入字节序列 `data` 是否匹配按键标识 `key_id`（如 `"ctrl+c"`、`"shift+tab"`、`"up"`）。
/// `kitty_active` 指示 Kitty 键盘协议是否激活，影响 legacy 序列的歧义解读。
pub fn matches_key(data: &str, key_id: &str, kitty_active: bool) -> bool {
    let Some(p) = parse_key_id(key_id) else { return false };
    let key = p.key.as_str();
    let mut modifier = 0u32;
    if p.shift {
        modifier |= MOD_SHIFT;
    }
    if p.alt {
        modifier |= MOD_ALT;
    }
    if p.ctrl {
        modifier |= MOD_CTRL;
    }
    if p.super_ {
        modifier |= MOD_SUPER;
    }

    match key {
        "escape" | "esc" => {
            if modifier != 0 {
                return false;
            }
            return data == "\x1b"
                || matches_kitty_sequence(data, CP_ESCAPE, 0)
                || matches_mok(data, CP_ESCAPE, 0);
        }
        "space" => {
            if !kitty_active {
                if modifier == MOD_CTRL && data == "\x00" {
                    return true;
                }
                if modifier == MOD_ALT && data == "\x1b " {
                    return true;
                }
            }
            if modifier == 0 {
                return data == " " || matches_kitty_sequence(data, CP_SPACE, 0) || matches_mok(data, CP_SPACE, 0);
            }
            return matches_kitty_sequence(data, CP_SPACE, modifier) || matches_mok(data, CP_SPACE, modifier);
        }
        "tab" => {
            if modifier == MOD_SHIFT {
                return data == "\x1b[Z"
                    || matches_kitty_sequence(data, CP_TAB, MOD_SHIFT)
                    || matches_mok(data, CP_TAB, MOD_SHIFT);
            }
            if modifier == 0 {
                return data == "\t" || matches_kitty_sequence(data, CP_TAB, 0);
            }
            return matches_kitty_sequence(data, CP_TAB, modifier) || matches_mok(data, CP_TAB, modifier);
        }
        "enter" | "return" => {
            if modifier == MOD_SHIFT {
                if matches_kitty_sequence(data, CP_ENTER, MOD_SHIFT)
                    || matches_kitty_sequence(data, CP_KP_ENTER, MOD_SHIFT)
                {
                    return true;
                }
                if matches_mok(data, CP_ENTER, MOD_SHIFT) {
                    return true;
                }
                if kitty_active {
                    return data == "\x1b\r" || data == "\n";
                }
                return false;
            }
            if modifier == MOD_ALT {
                if matches_kitty_sequence(data, CP_ENTER, MOD_ALT)
                    || matches_kitty_sequence(data, CP_KP_ENTER, MOD_ALT)
                {
                    return true;
                }
                if matches_mok(data, CP_ENTER, MOD_ALT) {
                    return true;
                }
                if !kitty_active {
                    return data == "\x1b\r";
                }
                return false;
            }
            if modifier == 0 {
                return data == "\r"
                    || (!kitty_active && data == "\n")
                    || data == "\x1bOM"
                    || matches_kitty_sequence(data, CP_ENTER, 0)
                    || matches_kitty_sequence(data, CP_KP_ENTER, 0);
            }
            return matches_kitty_sequence(data, CP_ENTER, modifier)
                || matches_kitty_sequence(data, CP_KP_ENTER, modifier)
                || matches_mok(data, CP_ENTER, modifier);
        }
        "backspace" => {
            if modifier == MOD_ALT {
                if data == "\x1b\x7f" || data == "\x1b\x08" {
                    return true;
                }
                return matches_kitty_sequence(data, CP_BACKSPACE, MOD_ALT)
                    || matches_mok(data, CP_BACKSPACE, MOD_ALT);
            }
            if modifier == MOD_CTRL {
                if data == "\x08" {
                    return true;
                }
                return matches_kitty_sequence(data, CP_BACKSPACE, MOD_CTRL)
                    || matches_mok(data, CP_BACKSPACE, MOD_CTRL);
            }
            if modifier == 0 {
                return data == "\x7f"
                    || matches_kitty_sequence(data, CP_BACKSPACE, 0)
                    || matches_mok(data, CP_BACKSPACE, 0);
            }
            return matches_kitty_sequence(data, CP_BACKSPACE, modifier) || matches_mok(data, CP_BACKSPACE, modifier);
        }
        "insert" | "delete" | "clear" | "home" | "end" | "pageup" | "pagedown" => {
            let fn_cp = match key {
                "insert" => FN_INSERT,
                "delete" => FN_DELETE,
                "home" => FN_HOME,
                "end" => FN_END,
                "pageup" => FN_PAGE_UP,
                "pagedown" => FN_PAGE_DOWN,
                _ => 0,
            };
            if modifier == 0 {
                if matches_legacy(data, legacy_sequences(key)) {
                    return true;
                }
                if key != "clear" && matches_kitty_sequence(data, fn_cp, 0) {
                    return true;
                }
                return false;
            }
            if matches_legacy_modifier(data, key, modifier) {
                return true;
            }
            if key != "clear" {
                return matches_kitty_sequence(data, fn_cp, modifier);
            }
            return false;
        }
        "up" | "down" | "left" | "right" => {
            let arrow_cp = match key {
                "up" => ARROW_UP,
                "down" => ARROW_DOWN,
                "left" => ARROW_LEFT,
                "right" => ARROW_RIGHT,
                _ => 0,
            };
            if modifier == MOD_ALT {
                let legacy_alt = match key {
                    "up" => data == "\x1bp",
                    "down" => data == "\x1bn",
                    "left" => data == "\x1b[1;3D" || (!kitty_active && data == "\x1bB") || data == "\x1bb",
                    "right" => data == "\x1b[1;3C" || (!kitty_active && data == "\x1bF") || data == "\x1bf",
                    _ => false,
                };
                return legacy_alt || matches_kitty_sequence(data, arrow_cp, MOD_ALT);
            }
            if modifier == MOD_CTRL && (key == "left" || key == "right") {
                let legacy_ctrl = if key == "left" { data == "\x1b[1;5D" } else { data == "\x1b[1;5C" };
                return legacy_ctrl
                    || matches_legacy_modifier(data, key, MOD_CTRL)
                    || matches_kitty_sequence(data, arrow_cp, MOD_CTRL);
            }
            if modifier == 0 {
                return matches_legacy(data, legacy_sequences(key)) || matches_kitty_sequence(data, arrow_cp, 0);
            }
            if matches_legacy_modifier(data, key, modifier) {
                return true;
            }
            return matches_kitty_sequence(data, arrow_cp, modifier);
        }
        "f1" | "f2" | "f3" | "f4" | "f5" | "f6" | "f7" | "f8" | "f9" | "f10" | "f11" | "f12" => {
            if modifier != 0 {
                return false;
            }
            return matches_legacy(data, legacy_sequences(key));
        }
        _ => {}
    }

    // 单个字母 / 数字 / 符号键
    let key_chars: Vec<char> = key.chars().collect();
    if key_chars.len() == 1 {
        let kc = key_chars[0];
        let is_letter = ('a'..='z').contains(&kc);
        let is_digit = kc.is_ascii_digit();
        if is_letter || is_digit || is_symbol_key(kc) {
            let codepoint = kc as i64;
            let rawctrl = raw_ctrl_char(kc);

            if modifier == (MOD_CTRL | MOD_ALT) && !kitty_active {
                if let Some(rc) = rawctrl {
                    if data == format!("\x1b{rc}") {
                        return true;
                    }
                }
            }
            if modifier == MOD_ALT && !kitty_active && (is_letter || is_digit) {
                if data == format!("\x1b{kc}") {
                    return true;
                }
            }
            if modifier == MOD_CTRL {
                if let Some(rc) = rawctrl {
                    if data.chars().count() == 1 && data.chars().next() == Some(rc) {
                        return true;
                    }
                }
                return matches_kitty_sequence(data, codepoint, MOD_CTRL)
                    || matches_printable_mok(data, codepoint, MOD_CTRL);
            }
            if modifier == (MOD_SHIFT | MOD_CTRL) {
                return matches_kitty_sequence(data, codepoint, MOD_SHIFT | MOD_CTRL)
                    || matches_printable_mok(data, codepoint, MOD_SHIFT | MOD_CTRL);
            }
            if modifier == MOD_SHIFT {
                if is_letter && data == kc.to_uppercase().to_string() {
                    return true;
                }
                return matches_kitty_sequence(data, codepoint, MOD_SHIFT)
                    || matches_printable_mok(data, codepoint, MOD_SHIFT);
            }
            if modifier != 0 {
                return matches_kitty_sequence(data, codepoint, modifier)
                    || matches_printable_mok(data, codepoint, modifier);
            }
            return data == key || matches_kitty_sequence(data, codepoint, 0);
        }
    }
    false
}

fn format_key_name_with_modifiers(name: &str, modifier: u32) -> Option<String> {
    let eff = modifier & !LOCK_MASK;
    let supported = MOD_SHIFT | MOD_CTRL | MOD_ALT | MOD_SUPER;
    if (eff & !supported) != 0 {
        return None;
    }
    let mut mods: Vec<&str> = Vec::new();
    if eff & MOD_SHIFT != 0 {
        mods.push("shift");
    }
    if eff & MOD_CTRL != 0 {
        mods.push("ctrl");
    }
    if eff & MOD_ALT != 0 {
        mods.push("alt");
    }
    if eff & MOD_SUPER != 0 {
        mods.push("super");
    }
    if mods.is_empty() {
        Some(name.to_string())
    } else {
        Some(format!("{}+{}", mods.join("+"), name))
    }
}

fn format_parsed_key(codepoint: i64, modifier: u32, base_layout_key: Option<i64>) -> Option<String> {
    let normalized = normalize_kitty_functional(codepoint);
    let identity = normalize_shifted_letter_identity(normalized, modifier);
    let is_latin = (97..=122).contains(&identity);
    let is_digit = (48..=57).contains(&identity);
    let is_symbol = char::from_u32(identity as u32).map(is_symbol_key).unwrap_or(false);
    let effective = if is_latin || is_digit || is_symbol {
        identity
    } else {
        base_layout_key.unwrap_or(identity)
    };

    let name: Option<String> = if effective == CP_ESCAPE {
        Some("escape".into())
    } else if effective == CP_TAB {
        Some("tab".into())
    } else if effective == CP_ENTER || effective == CP_KP_ENTER {
        Some("enter".into())
    } else if effective == CP_SPACE {
        Some("space".into())
    } else if effective == CP_BACKSPACE {
        Some("backspace".into())
    } else if effective == FN_DELETE {
        Some("delete".into())
    } else if effective == FN_INSERT {
        Some("insert".into())
    } else if effective == FN_HOME {
        Some("home".into())
    } else if effective == FN_END {
        Some("end".into())
    } else if effective == FN_PAGE_UP {
        Some("pageUp".into())
    } else if effective == FN_PAGE_DOWN {
        Some("pageDown".into())
    } else if effective == ARROW_UP {
        Some("up".into())
    } else if effective == ARROW_DOWN {
        Some("down".into())
    } else if effective == ARROW_LEFT {
        Some("left".into())
    } else if effective == ARROW_RIGHT {
        Some("right".into())
    } else if (48..=57).contains(&effective) || (97..=122).contains(&effective) {
        char::from_u32(effective as u32).map(|c| c.to_string())
    } else if char::from_u32(effective as u32).map(is_symbol_key).unwrap_or(false) {
        char::from_u32(effective as u32).map(|c| c.to_string())
    } else {
        None
    };

    format_key_name_with_modifiers(&name?, modifier)
}

/// 解析输入并返回按键标识（如 `"ctrl+c"`），无法识别时返回 None。
pub fn parse_key(data: &str, kitty_active: bool) -> Option<String> {
    if let Some(k) = parse_kitty_sequence(data) {
        return format_parsed_key(k.codepoint, k.modifier, k.base_layout_key);
    }
    if let Some(m) = parse_mok(data) {
        return format_parsed_key(m.codepoint, m.modifier, None);
    }
    if kitty_active && (data == "\x1b\r" || data == "\n") {
        return Some("shift+enter".into());
    }

    // 一些固定 legacy 序列
    let direct = match data {
        "\x1bOA" => Some("up"),
        "\x1bOB" => Some("down"),
        "\x1bOC" => Some("right"),
        "\x1bOD" => Some("left"),
        "\x1bOH" => Some("home"),
        "\x1bOF" => Some("end"),
        "\x1b[Z" => Some("shift+tab"),
        _ => None,
    };
    if let Some(d) = direct {
        return Some(d.into());
    }

    if data == "\x1b" {
        return Some("escape".into());
    }
    if data == "\x1c" {
        return Some("ctrl+\\".into());
    }
    if data == "\x1d" {
        return Some("ctrl+]".into());
    }
    if data == "\x1f" {
        return Some("ctrl+-".into());
    }
    if data == "\t" {
        return Some("tab".into());
    }
    if data == "\r" || (!kitty_active && data == "\n") || data == "\x1bOM" {
        return Some("enter".into());
    }
    if data == "\x00" {
        return Some("ctrl+space".into());
    }
    if data == " " {
        return Some("space".into());
    }
    if data == "\x7f" {
        return Some("backspace".into());
    }
    if !kitty_active && data == "\x1b\r" {
        return Some("alt+enter".into());
    }
    if !kitty_active && data == "\x1b " {
        return Some("alt+space".into());
    }
    if data == "\x1b\x7f" || data == "\x1b\x08" {
        return Some("alt+backspace".into());
    }
    if !kitty_active && data == "\x1bB" {
        return Some("alt+left".into());
    }
    if !kitty_active && data == "\x1bF" {
        return Some("alt+right".into());
    }
    // ESC + 单字符（alt+letter/digit、ctrl+alt+letter）
    if !kitty_active {
        let chars: Vec<char> = data.chars().collect();
        if chars.len() == 2 && chars[0] == '\x1b' {
            let code = chars[1] as u32;
            if (1..=26).contains(&code) {
                let letter = char::from_u32(code + 96).unwrap();
                return Some(format!("ctrl+alt+{letter}"));
            }
            if (97..=122).contains(&code) || (48..=57).contains(&code) {
                return Some(format!("alt+{}", chars[1]));
            }
        }
    }
    match data {
        "\x1b[A" => return Some("up".into()),
        "\x1b[B" => return Some("down".into()),
        "\x1b[C" => return Some("right".into()),
        "\x1b[D" => return Some("left".into()),
        "\x1b[H" => return Some("home".into()),
        "\x1b[F" => return Some("end".into()),
        "\x1b[3~" => return Some("delete".into()),
        "\x1b[5~" => return Some("pageUp".into()),
        "\x1b[6~" => return Some("pageDown".into()),
        _ => {}
    }

    // raw Ctrl+letter / 可打印 ASCII
    let chars: Vec<char> = data.chars().collect();
    if chars.len() == 1 {
        let code = chars[0] as u32;
        if (1..=26).contains(&code) {
            return Some(format!("ctrl+{}", char::from_u32(code + 96).unwrap()));
        }
        if (32..=126).contains(&code) {
            return Some(data.to_string());
        }
    }
    None
}

/// 把 Kitty CSI-u（仅 plain / shift 修饰）解码回可打印字符。Ctrl/Alt 等返回 None。
pub fn decode_kitty_printable(data: &str) -> Option<char> {
    let k = parse_kitty_sequence(data)?;
    // 只接受 plain / shift（外加 lock 位）
    let allowed = MOD_SHIFT | LOCK_MASK;
    if (k.modifier & !allowed) != 0 {
        return None;
    }
    if k.modifier & (MOD_ALT | MOD_CTRL) != 0 {
        return None;
    }
    let cp = normalize_kitty_functional(k.codepoint);
    if cp < 32 {
        return None;
    }
    char::from_u32(cp as u32)
}

/// modifyOtherKeys 的可打印解码（仅 plain / shift）。
fn decode_mok_printable(data: &str) -> Option<char> {
    let p = parse_mok(data)?;
    let modifier = p.modifier & !LOCK_MASK;
    if (modifier & !MOD_SHIFT) != 0 {
        return None;
    }
    if p.codepoint < 32 {
        return None;
    }
    char::from_u32(p.codepoint as u32)
}

/// 把可打印按键序列（Kitty CSI-u 或 modifyOtherKeys）解码成字符。
pub fn decode_printable_key(data: &str) -> Option<char> {
    decode_kitty_printable(data).or_else(|| decode_mok_printable(data))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ctrl_c_raw() {
        assert!(matches_key("\x03", "ctrl+c", false));
        assert!(!matches_key("\x03", "ctrl+d", false));
    }

    #[test]
    fn ctrl_letters_formula() {
        assert!(matches_key("\x01", "ctrl+a", false));
        assert!(matches_key("\x1a", "ctrl+z", false));
        assert!(matches_key("\x0b", "ctrl+k", false));
    }

    #[test]
    fn plain_char() {
        assert!(matches_key("a", "a", false));
        assert!(matches_key("5", "5", false));
        assert!(!matches_key("a", "b", false));
    }

    #[test]
    fn escape_key() {
        assert!(matches_key("\x1b", "escape", false));
        assert!(matches_key("\x1b", "esc", false));
        assert!(!matches_key("\x1b", "ctrl+escape", false));
    }

    #[test]
    fn enter_variants() {
        assert!(matches_key("\r", "enter", false));
        assert!(matches_key("\n", "enter", false)); // legacy
        assert!(!matches_key("\n", "enter", true)); // kitty: \n is shift+enter
    }

    #[test]
    fn tab_and_shift_tab() {
        assert!(matches_key("\t", "tab", false));
        assert!(matches_key("\x1b[Z", "shift+tab", false));
    }

    #[test]
    fn backspace_variants() {
        assert!(matches_key("\x7f", "backspace", false));
        assert!(matches_key("\x1b\x7f", "alt+backspace", false));
    }

    #[test]
    fn arrows_legacy() {
        assert!(matches_key("\x1b[A", "up", false));
        assert!(matches_key("\x1b[B", "down", false));
        assert!(matches_key("\x1b[C", "right", false));
        assert!(matches_key("\x1b[D", "left", false));
        assert!(matches_key("\x1bOA", "up", false));
    }

    #[test]
    fn arrows_with_ctrl() {
        assert!(matches_key("\x1b[1;5C", "ctrl+right", false));
        assert!(matches_key("\x1b[1;5D", "ctrl+left", false));
    }

    #[test]
    fn alt_left_right_word_nav() {
        assert!(matches_key("\x1bb", "alt+left", false));
        assert!(matches_key("\x1bf", "alt+right", false));
    }

    #[test]
    fn kitty_csi_u_ctrl() {
        // \x1b[99;5u = codepoint 99 ('c') with ctrl (mod 5 -> 4 = ctrl)
        assert!(matches_key("\x1b[99;5u", "ctrl+c", false));
    }

    #[test]
    fn kitty_csi_u_plain() {
        // codepoint 97 ('a'), no modifier
        assert!(matches_key("\x1b[97u", "a", false));
    }

    #[test]
    fn kitty_csi_u_shift_enter() {
        // codepoint 13 (enter) with shift (mod 2 -> 1 = shift)
        assert!(matches_key("\x1b[13;2u", "shift+enter", false));
    }

    #[test]
    fn modify_other_keys() {
        // \x1b[27;5;99~ = ctrl+c via modifyOtherKeys
        assert!(matches_key("\x1b[27;5;99~", "ctrl+c", false));
    }

    #[test]
    fn function_keys() {
        assert!(matches_key("\x1bOP", "f1", false));
        assert!(matches_key("\x1b[15~", "f5", false));
        assert!(matches_key("\x1b[24~", "f12", false));
    }

    #[test]
    fn delete_pageup_pagedown() {
        assert!(matches_key("\x1b[3~", "delete", false));
        assert!(matches_key("\x1b[5~", "pageUp", false));
        assert!(matches_key("\x1b[6~", "pageDown", false));
    }

    #[test]
    fn parse_key_basics() {
        assert_eq!(parse_key("\x03", false).as_deref(), Some("ctrl+c"));
        assert_eq!(parse_key("\x1b[A", false).as_deref(), Some("up"));
        assert_eq!(parse_key("\x1b", false).as_deref(), Some("escape"));
        assert_eq!(parse_key("a", false).as_deref(), Some("a"));
        assert_eq!(parse_key("\x1b[Z", false).as_deref(), Some("shift+tab"));
    }

    #[test]
    fn parse_key_kitty() {
        assert_eq!(parse_key("\x1b[99;5u", false).as_deref(), Some("ctrl+c"));
        assert_eq!(parse_key("\x1b[13;2u", false).as_deref(), Some("shift+enter"));
    }

    #[test]
    fn paste_event_detection() {
        assert!(is_key_release("\x1b[97;1:3u"));
        assert!(is_key_repeat("\x1b[97;1:2u"));
        // paste content not treated as release
        assert!(!is_key_release("\x1b[200~90:62:3F\x1b[201~"));
        assert_eq!(event_type("\x1b[97u"), KeyEventType::Press);
    }

    #[test]
    fn decode_printable() {
        assert_eq!(decode_kitty_printable("\x1b[97u"), Some('a'));
        // ctrl modifier should NOT decode to printable
        assert_eq!(decode_kitty_printable("\x1b[97;5u"), None);
        assert_eq!(decode_printable_key("\x1b[27;1;97~"), Some('a'));
    }

    #[test]
    fn non_latin_base_layout_match() {
        // Cyrillic С (U+0421) with ctrl, base layout key 99 ('c'): \x1b[1057::99;5u
        assert!(matches_key("\x1b[1057::99;5u", "ctrl+c", false));
    }
}
