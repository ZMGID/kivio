//! Publishing and previewing self-contained HTML pages in a conversation.
mod browser;

use std::{
    borrow::Cow,
    collections::{hash_map::Entry, HashMap, VecDeque},
    fs,
    io::Read,
    path::{Component, Path, PathBuf},
    sync::LazyLock,
    time::{Duration, Instant},
};

use base64::{engine::general_purpose::STANDARD, Engine as _};
use parking_lot::Mutex;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::{AppHandle, Manager};

pub(crate) const HTML_RENDER_TOOL: &str = "html_render";
pub(crate) const HTML_PREVIEW_TOOL: &str = "html_preview";
pub(crate) const MIN_HEIGHT: u32 = 80;
pub(crate) const MAX_HEIGHT: u32 = 2000;
pub(crate) const COLUMN_WIDTH: u32 = 728;
pub(crate) const MAX_TITLE: usize = 200;
const MAX_HTML_INPUT: usize = 512_000;
const MAX_IMAGE_BYTES: u64 = 10 * 1024 * 1024;
const MAX_PAGE_BYTES: usize = 25 * 1024 * 1024;
const MIN_PREVIEW_WIDTH: u32 = 240;
const MAX_PREVIEW_WIDTH: u32 = 1600;
const PENDING_TTL: Duration = Duration::from_secs(10 * 60);

pub(crate) const THEME_GUIDE: &str = "Use theme CSS variables on :root rather than fixed colors: --background, --foreground, --muted, --muted-foreground, --card, --card-foreground, --popover, --popover-foreground, --secondary, --secondary-foreground, --border, --input, --ring, --primary, --primary-foreground, --accent, --accent-foreground, --destructive, --destructive-foreground, --warning, --warning-foreground, --success, --success-foreground, --info, --info-foreground, --code-background, --code-foreground, --chart-1 through --chart-6, --radius, --font-sans, and --font-mono.";
pub(crate) const LAYOUT_GUIDE: &str = "The frame is the reply column, about 728px wide on desktop and 360px on phones. Leave html, body, and the outer element without a page background, outer card, border, banner title, or side padding. Use fluid widths. A box needing a background gets 16px padding and var(--radius) corners. Give charts fixed pixel heights. Let content set the height: never use vh, svh, lvh, or dvh units, or height:100% on html or body.";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct HtmlRenderReference {
    #[serde(rename = "type")]
    kind: String,
    pub(crate) attachment_id: String,
    pub(crate) title: String,
    pub(crate) height: u32,
}

pub(crate) fn is_valid_render_attachment_id(id: &str) -> bool {
    let Some(uuid) = id.strip_prefix("htmlrender-") else {
        return false;
    };
    uuid::Uuid::parse_str(uuid).is_ok_and(|parsed| parsed.hyphenated().to_string() == uuid)
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct HtmlPreviewOutput {
    pub(crate) png_base64: String,
    pub(crate) width: u32,
    pub(crate) content_height: u32,
    pub(crate) captured_height: u32,
    pub(crate) console: Vec<browser::ConsoleMessage>,
    pub(crate) missing_images: Vec<String>,
}

pub(crate) struct HtmlToolResult {
    pub(crate) text: String,
    pub(crate) structured: Option<Value>,
    pub(crate) image_png_base64: Option<String>,
}

fn validate_html(html: &str) -> Result<(), String> {
    if html.is_empty() || html.len() > MAX_HTML_INPUT {
        return Err(format!(
            "HTML must be between 1 and {MAX_HTML_INPUT} bytes."
        ));
    }
    Ok(())
}

fn validate_render(html: &str, title: &str, height: u32) -> Result<(), String> {
    validate_html(html)?;
    if title.trim().is_empty() || title.chars().count() > MAX_TITLE {
        return Err(format!(
            "Title must be between 1 and {MAX_TITLE} characters."
        ));
    }
    if !(MIN_HEIGHT..=MAX_HEIGHT).contains(&height) {
        return Err(format!(
            "Height must be between {MIN_HEIGHT} and {MAX_HEIGHT} pixels."
        ));
    }
    Ok(())
}

// Match the same CSS sources as T3, including style attributes in script strings.
fn css_sources() -> &'static Regex {
    static SOURCES: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(
        r#"(?is)<!--(?:.*?-->|.*\z)|<style\b[^>]*(?:>|\z)(.*?)(?:</style\s*>|\z)|\sstyle\s*=\s*(?:"([^"]*)(?:"|\z)|'([^']*)(?:'|\z)|([^\s>]+))"#,
    ).expect("valid CSS source pattern")
    });
    &SOURCES
}

fn css_tokens() -> &'static Regex {
    static TOKENS: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(
        r#"(?is)/\*(?:.*?\*/|.*\z)|<!--(?:.*?-->|.*\z)|data:[^\s"'()]*?;base64,[a-z0-9+/=]+|(?P<unit>(?:[0-9]+(?:\.[0-9]+)?|\.[0-9]+)[sld]?vh)"#,
    ).expect("valid CSS token pattern")
    });
    &TOKENS
}

fn css_word(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

fn reject_viewport_height(html: &str) -> Result<(), String> {
    for source in css_sources().captures_iter(html) {
        let Some(css) = (1..=4).find_map(|index| source.get(index)) else {
            continue;
        };
        let text = css.as_str();
        for token in css_tokens().captures_iter(text) {
            let Some(unit) = token.name("unit") else {
                continue;
            };
            let before = unit
                .start()
                .checked_sub(1)
                .and_then(|index| text.as_bytes().get(index));
            let after = text.as_bytes().get(unit.end());
            if before.is_some_and(|byte| css_word(*byte) || matches!(*byte, b'.' | b'+' | b'/'))
                || after.is_some_and(|byte| css_word(*byte) || *byte == b'-')
            {
                continue;
            }
            return Err(format!(
                "The page uses a viewport height unit ({}). The frame grows to fit the page; let content set the height or use fixed pixel heights.",
                unit.as_str()
            ));
        }
    }
    Ok(())
}

struct ImageReference<'a> {
    start: usize,
    end: usize,
    path: &'a str,
}

fn local_image_pattern() -> &'static Regex {
    static PATTERN: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(
        r#"(?i)"(/[^"\r\n]*|[a-z]:[\\/][^"\r\n]*|\\\\[^"\r\n]*)"|'(/[^'\r\n]*|[a-z]:[\\/][^'\r\n]*|\\\\[^'\r\n]*)'|`(/[^`\r\n]*|[a-z]:[\\/][^`\r\n]*|\\\\[^`\r\n]*)`|url\(\s*(/[^\s"'`()]*|[a-z]:[\\/][^\s"'()`]*|\\\\[^\s"'()`]*)\s*\)"#,
    ).expect("valid local image pattern")
    });
    &PATTERN
}

fn local_images(html: &str) -> impl Iterator<Item = ImageReference<'_>> + '_ {
    local_image_pattern()
        .captures_iter(html)
        .filter_map(|capture| {
            let path = (1..=4).find_map(|index| capture.get(index))?;
            let value = path.as_str();
            if value.len() > 2048 || value.starts_with("//") {
                return None;
            }
            let extension = value.rsplit_once('.')?.1;
            if ![
                "png", "jpg", "jpeg", "gif", "webp", "bmp", "ico", "avif", "svg",
            ]
            .iter()
            .any(|candidate| extension.eq_ignore_ascii_case(candidate))
            {
                return None;
            }
            Some(ImageReference {
                start: path.start(),
                end: path.end(),
                path: value,
            })
        })
}

fn after_token(bytes: &[u8], from: usize, token: &[u8]) -> Option<usize> {
    bytes
        .get(from..)?
        .windows(token.len())
        .position(|part| part == token)
        .map(|index| from + index + token.len())
}

fn after_doctype(bytes: &[u8], mut at: usize) -> Option<usize> {
    let mut in_subset = false;
    while at < bytes.len() {
        match bytes[at] {
            b'\'' | b'"' => {
                let quote = bytes[at];
                at = after_token(bytes, at + 1, &[quote])?;
            }
            b'<' if in_subset && bytes[at..].starts_with(b"<!--") => {
                at = after_token(bytes, at + 4, b"-->")?;
            }
            b'<' if in_subset && bytes[at..].starts_with(b"<?") => {
                at = after_token(bytes, at + 2, b"?>")?;
            }
            b'>' if !in_subset => return Some(at + 1),
            b'[' => {
                in_subset = true;
                at += 1;
            }
            b']' => {
                in_subset = false;
                at += 1;
            }
            _ => at += 1,
        }
    }
    None
}

fn has_svg_root(bytes: &[u8]) -> bool {
    let bytes = &bytes[..bytes.len().min(4096)];
    let mut at = if bytes.starts_with(b"\xef\xbb\xbf") {
        3
    } else {
        0
    };
    while at < bytes.len() {
        while bytes.get(at).is_some_and(u8::is_ascii_whitespace) {
            at += 1;
        }
        let Some(rest) = bytes.get(at..) else {
            return false;
        };
        if rest.starts_with(b"<?") {
            let Some(end) = after_token(bytes, at + 2, b"?>") else {
                return false;
            };
            at = end;
        } else if rest.starts_with(b"<!--") {
            let Some(end) = after_token(bytes, at + 4, b"-->") else {
                return false;
            };
            at = end;
        } else if rest
            .get(..9)
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case(b"<!doctype"))
        {
            let Some(end) = after_doctype(bytes, at + 9) else {
                return false;
            };
            at = end;
        } else {
            return rest.starts_with(b"<svg")
                && rest.get(4).is_some_and(|byte| {
                    matches!(*byte, b' ' | b'\t' | b'\r' | b'\n' | b'/' | b'>')
                });
        }
    }
    false
}

// Detect the content's type, not just the extension, before exposing local bytes.
fn image_mime(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if bytes.starts_with(b"\xff\xd8\xff") {
        Some("image/jpeg")
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some("image/gif")
    } else if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(&b"WEBP"[..]) {
        Some("image/webp")
    } else if bytes.starts_with(b"BM") && bytes.get(6..10) == Some(&[0_u8, 0, 0, 0][..]) {
        Some("image/bmp")
    } else if bytes.starts_with(b"\0\0\x01\0") {
        Some("image/x-icon")
    } else if bytes.get(4..8) == Some(&b"ftyp"[..])
        && bytes
            .get(8..12)
            .is_some_and(|brand| matches!(brand, b"avif" | b"avis" | b"mif1"))
    {
        Some("image/avif")
    } else if has_svg_root(bytes) {
        Some("image/svg+xml")
    } else {
        None
    }
}

struct ImageRoot {
    aliases: [PathBuf; 2],
    directory: fs::File,
}

fn image_roots(paths: Vec<PathBuf>) -> Vec<ImageRoot> {
    paths
        .into_iter()
        .filter_map(|path| {
            if !path.to_str().is_some_and(is_local_path) {
                return None;
            }
            let (directory, canonical) = resolve_directory_root(&path)?;
            Some(ImageRoot {
                aliases: [path, canonical],
                directory,
            })
        })
        .collect()
}

fn relative_to_root(path: &Path, root: &Path) -> Option<(PathBuf, bool)> {
    let mut components = path.components();
    for expected in root.components() {
        let actual = components.next()?;
        #[cfg(windows)]
        let equal = actual
            .as_os_str()
            .to_string_lossy()
            .eq_ignore_ascii_case(&expected.as_os_str().to_string_lossy());
        #[cfg(not(windows))]
        let equal = actual == expected;
        if !equal {
            return None;
        }
    }
    let mut relative = PathBuf::new();
    let mut depth = 0;
    let mut has_parent = false;
    for component in components {
        match component {
            Component::Normal(name) => {
                relative.push(name);
                depth += 1;
            }
            Component::CurDir => {}
            Component::ParentDir if depth > 0 => {
                relative.push("..");
                depth -= 1;
                has_parent = true;
            }
            _ => return None,
        }
    }
    (!relative.as_os_str().is_empty()).then_some((relative, has_parent))
}

#[cfg(unix)]
fn open_child(
    directory: &fs::File,
    name: &std::ffi::OsStr,
    is_directory: bool,
) -> Option<fs::File> {
    use std::os::unix::{
        ffi::OsStrExt,
        io::{AsRawFd, FromRawFd},
    };
    let name = std::ffi::CString::new(name.as_bytes()).ok()?;
    let flags = libc::O_RDONLY
        | libc::O_CLOEXEC
        | libc::O_NOFOLLOW
        | libc::O_NONBLOCK
        | libc::O_NOCTTY
        | if is_directory { libc::O_DIRECTORY } else { 0 };
    let fd = unsafe { libc::openat(directory.as_raw_fd(), name.as_ptr(), flags) };
    (fd >= 0).then(|| unsafe { fs::File::from_raw_fd(fd) })
}

#[cfg(windows)]
fn open_child_checked(
    directory: &fs::File,
    name: &std::ffi::OsStr,
    is_directory: bool,
    inspect_reparse: bool,
) -> Result<fs::File, i32> {
    use std::{
        ffi::c_void,
        os::windows::{
            ffi::OsStrExt,
            fs::MetadataExt,
            io::{AsRawHandle, FromRawHandle},
        },
    };
    const READ_ATTRIBUTES_AND_DATA: u32 = 0x0010_0081;
    const FILE_TRAVERSE: u32 = 0x20;
    const OBJ_CASE_INSENSITIVE: u32 = 0x40;
    const SHARE_ALL: u32 = 7;
    const FILE_OPEN: u32 = 1;
    const OPEN_REPARSE_POINT: u32 = 0x0020_0000;
    const SYNCHRONOUS_IO_NONALERT: u32 = 0x20;
    const DIRECTORY_FILE: u32 = 1;
    const NON_DIRECTORY_FILE: u32 = 0x40;
    const ATTRIBUTE_REPARSE_POINT: u32 = 0x400;

    #[repr(C)]
    struct UnicodeString {
        length: u16,
        maximum_length: u16,
        buffer: *mut u16,
    }
    #[repr(C)]
    struct ObjectAttributes {
        length: u32,
        root_directory: *mut c_void,
        object_name: *mut UnicodeString,
        attributes: u32,
        security_descriptor: *mut c_void,
        security_quality_of_service: *mut c_void,
    }
    #[repr(C)]
    struct IoStatusBlock {
        status: usize,
        information: usize,
    }
    #[link(name = "ntdll")]
    extern "system" {
        fn NtCreateFile(
            handle: *mut *mut c_void,
            access: u32,
            attributes: *mut ObjectAttributes,
            status: *mut IoStatusBlock,
            allocation_size: *const i64,
            file_attributes: u32,
            share_access: u32,
            disposition: u32,
            options: u32,
            ea_buffer: *mut c_void,
            ea_length: u32,
        ) -> i32;
    }

    let mut name: Vec<u16> = name.encode_wide().collect();
    if name.is_empty() || name.contains(&0) || name.len() > u16::MAX as usize / 2 {
        return Err(i32::MIN);
    }
    let length = (name.len() * 2) as u16;
    let mut object_name = UnicodeString {
        length,
        maximum_length: length,
        buffer: name.as_mut_ptr(),
    };
    let mut attributes = ObjectAttributes {
        length: std::mem::size_of::<ObjectAttributes>() as u32,
        root_directory: directory.as_raw_handle(),
        object_name: &mut object_name,
        attributes: OBJ_CASE_INSENSITIVE,
        security_descriptor: std::ptr::null_mut(),
        security_quality_of_service: std::ptr::null_mut(),
    };
    let mut status = IoStatusBlock {
        status: 0,
        information: 0,
    };
    let mut handle = std::ptr::null_mut();
    // Open relative to the already-open parent and never traverse a reparse point.
    let result = unsafe {
        NtCreateFile(
            &mut handle,
            READ_ATTRIBUTES_AND_DATA | if is_directory { FILE_TRAVERSE } else { 0 },
            &mut attributes,
            &mut status,
            std::ptr::null(),
            0,
            SHARE_ALL,
            FILE_OPEN,
            OPEN_REPARSE_POINT
                | SYNCHRONOUS_IO_NONALERT
                | if inspect_reparse {
                    0
                } else if is_directory {
                    DIRECTORY_FILE
                } else {
                    NON_DIRECTORY_FILE
                },
            std::ptr::null_mut(),
            0,
        )
    };
    if result < 0 {
        return Err(result);
    }
    if handle.is_null() {
        return Err(i32::MIN);
    }
    let file = unsafe { fs::File::from_raw_handle(handle) };
    let metadata = file.metadata().map_err(|_| i32::MIN)?;
    if (!inspect_reparse && metadata.file_attributes() & ATTRIBUTE_REPARSE_POINT != 0)
        || (!inspect_reparse && is_directory && !metadata.is_dir())
    {
        return Err(i32::MIN);
    }
    Ok(file)
}

#[cfg(windows)]
fn open_child(
    directory: &fs::File,
    name: &std::ffi::OsStr,
    is_directory: bool,
) -> Option<fs::File> {
    open_child_checked(directory, name, is_directory, false).ok()
}

#[cfg(unix)]
fn read_link_at(directory: &fs::File, name: &std::ffi::OsStr) -> Option<PathBuf> {
    use std::os::unix::{
        ffi::{OsStrExt, OsStringExt},
        io::AsRawFd,
    };
    let name = std::ffi::CString::new(name.as_bytes()).ok()?;
    let mut buffer = [0u8; 4096];
    let length = unsafe {
        libc::readlinkat(
            directory.as_raw_fd(),
            name.as_ptr(),
            buffer.as_mut_ptr().cast(),
            buffer.len(),
        )
    };
    if length < 0 || length as usize == buffer.len() {
        return None;
    }
    Some(PathBuf::from(std::ffi::OsString::from_vec(
        buffer[..length as usize].to_vec(),
    )))
}

#[cfg(windows)]
fn read_link_at(directory: &fs::File, name: &std::ffi::OsStr) -> Option<PathBuf> {
    use std::{
        ffi::c_void,
        os::windows::{fs::MetadataExt, io::AsRawHandle},
    };
    #[link(name = "kernel32")]
    extern "system" {
        fn DeviceIoControl(
            handle: *mut c_void,
            control: u32,
            input: *const c_void,
            input_len: u32,
            output: *mut c_void,
            output_len: u32,
            returned: *mut u32,
            overlapped: *mut c_void,
        ) -> i32;
    }
    let file = open_child_checked(directory, name, false, true).ok()?;
    if file.metadata().ok()?.file_attributes() & 0x400 == 0 {
        return None;
    }
    let mut data = [0u8; 16 * 1024];
    let mut returned = 0;
    let ok = unsafe {
        DeviceIoControl(
            file.as_raw_handle(),
            0x0009_00a8,
            std::ptr::null(),
            0,
            data.as_mut_ptr().cast(),
            data.len() as u32,
            &mut returned,
            std::ptr::null_mut(),
        )
    };
    if ok == 0 || returned < 16 || returned as usize > data.len() {
        return None;
    }
    decode_reparse_target(&data[..returned as usize])
}

#[cfg(windows)]
fn decode_reparse_target(data: &[u8]) -> Option<PathBuf> {
    use std::os::windows::ffi::OsStringExt;
    let tag = u32::from_le_bytes(data.get(..4)?.try_into().ok()?);
    let total = 8usize.checked_add(u16::from_le_bytes(data.get(4..6)?.try_into().ok()?) as usize)?;
    if total > data.len() {
        return None;
    }
    let (base, relative) = match tag {
        0xa000_000c => (
            20,
            u32::from_le_bytes(data.get(16..20)?.try_into().ok()?) & 1 != 0,
        ),
        0xa000_0003 => (16, false),
        _ => return None,
    };
    let offset = u16::from_le_bytes(data.get(8..10)?.try_into().ok()?) as usize;
    let length = u16::from_le_bytes(data.get(10..12)?.try_into().ok()?) as usize;
    let start = base + offset;
    let end = start.checked_add(length)?;
    if offset % 2 != 0 || length % 2 != 0 || end > total {
        return None;
    }
    let wide: Vec<u16> = data
        .get(start..end)?
        .chunks_exact(2)
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
        .collect();
    let target = std::ffi::OsString::from_wide(&wide);
    let target = PathBuf::from(target);
    if relative {
        if target.has_root() || matches!(target.components().next(), Some(Component::Prefix(_))) {
            return None;
        }
        Some(target)
    } else {
        let target = target.to_str()?.strip_prefix(r"\??\")?;
        is_local_path(target).then(|| PathBuf::from(target))
    }
}

#[cfg(unix)]
fn volume_root(path: &Path) -> Option<(fs::File, PathBuf, std::path::Components<'_>)> {
    let mut components = path.components();
    if components.next()? != Component::RootDir {
        return None;
    }
    Some((fs::File::open("/").ok()?, PathBuf::from("/"), components))
}

#[cfg(windows)]
fn volume_root(path: &Path) -> Option<(fs::File, PathBuf, std::path::Components<'_>)> {
    use std::{os::windows::fs::OpenOptionsExt, path::Prefix};
    let mut components = path.components();
    let Component::Prefix(prefix) = components.next()? else {
        return None;
    };
    let Prefix::Disk(drive) = prefix.kind() else {
        return None;
    };
    if components.next()? != Component::RootDir {
        return None;
    }
    let root = PathBuf::from(format!("{}:\\", drive as char));
    let directory = fs::OpenOptions::new()
        .read(true)
        .access_mode(0x0010_00a1) // SYNCHRONIZE, READ_ATTRIBUTES, LIST_DIRECTORY, TRAVERSE
        .custom_flags(0x0200_0000) // FILE_FLAG_BACKUP_SEMANTICS
        .open(&root)
        .ok()?;
    Some((directory, root, components))
}

fn resolve_directory_root(path: &Path) -> Option<(fs::File, PathBuf)> {
    let mut candidate = path.to_path_buf();
    let mut links = 0;
    'resolve: loop {
        if !candidate.to_str().is_some_and(is_local_path) {
            return None;
        }
        let (mut directory, mut resolved, mut components) = volume_root(&candidate)?;
        let mut parents = Vec::new();
        while let Some(component) = components.next() {
            match component {
                Component::Normal(name) => {
                    if let Some(next) = open_child(&directory, name, true) {
                        parents.push(directory);
                        directory = next;
                        resolved.push(name);
                    } else {
                        let target = read_link_at(&directory, name)?;
                        let mut next = if target.is_absolute() {
                            target
                        } else if target.has_root()
                            || matches!(target.components().next(), Some(Component::Prefix(_)))
                        {
                            return None;
                        } else {
                            resolved.join(target)
                        };
                        next.push(components.as_path());
                        candidate = next;
                        links += 1;
                        if links > 40 {
                            return None;
                        }
                        continue 'resolve;
                    }
                }
                Component::ParentDir => {
                    directory = parents.pop()?;
                    if !resolved.pop() {
                        return None;
                    }
                }
                Component::CurDir => {}
                _ => return None,
            }
        }
        #[cfg(windows)]
        let resolved = canonical_directory_path(&directory)?;
        return Some((directory, resolved));
    }
}

fn open_directory_root(path: &Path) -> Option<fs::File> {
    resolve_directory_root(path).map(|(directory, _)| directory)
}

#[cfg(windows)]
fn canonical_directory_path(directory: &fs::File) -> Option<PathBuf> {
    use std::{
        ffi::c_void,
        os::windows::{ffi::OsStringExt, io::AsRawHandle},
    };
    #[link(name = "kernel32")]
    extern "system" {
        fn GetFinalPathNameByHandleW(
            handle: *mut c_void,
            path: *mut u16,
            length: u32,
            flags: u32,
        ) -> u32;
    }
    let length =
        unsafe { GetFinalPathNameByHandleW(directory.as_raw_handle(), std::ptr::null_mut(), 0, 0) };
    if length == 0 {
        return None;
    }
    let mut path = vec![0u16; length as usize + 1];
    let written = unsafe {
        GetFinalPathNameByHandleW(
            directory.as_raw_handle(),
            path.as_mut_ptr(),
            path.len() as u32,
            0,
        )
    };
    if written == 0 || written as usize >= path.len() {
        return None;
    }
    let path = crate::utils::strip_windows_verbatim_prefix(PathBuf::from(
        std::ffi::OsString::from_wide(&path[..written as usize]),
    ));
    path.to_str().filter(|path| is_local_path(path))?;
    Some(path)
}

fn is_local_path(reference: &str) -> bool {
    if matches!(reference.as_bytes().get(1), Some(&b'/') | Some(&b'\\')) {
        return false;
    }
    let path = Path::new(reference);
    if !path.is_absolute() {
        return false;
    }
    #[cfg(windows)]
    if !matches!(path.components().next(), Some(Component::Prefix(prefix)) if matches!(prefix.kind(), std::path::Prefix::Disk(_)))
    {
        return false;
    }
    true
}

fn open_local_image(reference: &str, roots: &[ImageRoot]) -> Option<fs::File> {
    if !is_local_path(reference) {
        return None;
    }
    let path = if reference.as_bytes().get(1) == Some(&b':') && reference.contains("\\\\") {
        Cow::Owned(reference.replace("\\\\", "\\"))
    } else {
        Cow::Borrowed(reference)
    };
    let mut candidate = Cow::Borrowed(Path::new(&*path));
    let mut links = 0;
    'resolve: loop {
        for root in roots {
            'aliases: for alias in &root.aliases {
                let Some((relative, has_parent)) = relative_to_root(&candidate, alias) else {
                    continue;
                };
                let mut components = relative.components().peekable();
                let Ok(mut directory) = root.directory.try_clone() else {
                    continue;
                };
                let mut processed = 0;
                let mut parents = Vec::new();
                while let Some(component) = components.next() {
                    match component {
                        Component::Normal(name) => {
                            let Some(next) =
                                open_child(&directory, name, components.peek().is_some())
                            else {
                                let Some(target) = read_link_at(&directory, name) else {
                                    continue 'aliases;
                                };
                                let mut next = if target.is_absolute() {
                                    target
                                } else if target.has_root()
                                    || matches!(
                                        target.components().next(),
                                        Some(Component::Prefix(_))
                                    )
                                {
                                    return None;
                                } else {
                                    let mut base = root.aliases[1].clone();
                                    for component in relative.components().take(processed) {
                                        match component {
                                            Component::Normal(name) => base.push(name),
                                            Component::ParentDir if base.pop() => {}
                                            _ => return None,
                                        }
                                    }
                                    let mut leading = true;
                                    for component in target.components() {
                                        match component {
                                            Component::CurDir if leading => {}
                                            Component::ParentDir if leading => {
                                                if !base.pop() {
                                                    return None;
                                                }
                                            }
                                            _ => {
                                                leading = false;
                                                base.push(component.as_os_str());
                                            }
                                        }
                                    }
                                    base
                                };
                                for component in components {
                                    next.push(component.as_os_str());
                                }
                                if !next.to_str().is_some_and(is_local_path) {
                                    return None;
                                }
                                candidate = Cow::Owned(next);
                                links += 1;
                                if links > 40 {
                                    return None;
                                }
                                continue 'resolve;
                            };
                            if has_parent {
                                parents.push(std::mem::replace(&mut directory, next));
                            } else {
                                directory = next;
                            }
                        }
                        Component::ParentDir => {
                            let Some(parent) = parents.pop() else {
                                continue 'aliases;
                            };
                            directory = parent;
                        }
                        _ => continue 'aliases,
                    }
                    processed += 1;
                }
                return Some(directory);
            }
        }
        return None;
    }
}

fn load_local_image(reference: &str, roots: &[ImageRoot]) -> Result<Option<String>, String> {
    let Some(file) = open_local_image(reference, roots) else {
        return Ok(None);
    };
    let Ok(metadata) = file.metadata() else {
        return Ok(None);
    };
    if !metadata.is_file() {
        return Ok(None);
    }
    if metadata.len() > MAX_IMAGE_BYTES {
        return Err(format!("Local image {reference} exceeds the 10 MiB limit."));
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    if file
        .take(MAX_IMAGE_BYTES + 1)
        .read_to_end(&mut bytes)
        .is_err()
    {
        return Ok(None);
    }
    if bytes.len() as u64 > MAX_IMAGE_BYTES {
        return Err(format!("Local image {reference} exceeds the 10 MiB limit."));
    }
    let Some(mime) = image_mime(&bytes) else {
        return Ok(None);
    };
    let mut data = String::with_capacity(5 + mime.len() + 8 + bytes.len().div_ceil(3) * 4);
    data.push_str("data:");
    data.push_str(mime);
    data.push_str(";base64,");
    STANDARD.encode_string(&bytes, &mut data);
    Ok(Some(data))
}

fn push_page(output: &mut String, part: &str) -> Result<(), String> {
    if part.len() > MAX_PAGE_BYTES.saturating_sub(output.len()) {
        return Err("HTML with inlined images exceeds the 25 MiB limit.".into());
    }
    output.push_str(part);
    Ok(())
}

fn inline_local_images(html: String, paths: Vec<PathBuf>) -> Result<(String, Vec<String>), String> {
    let mut references = local_images(&html).peekable();
    if references.peek().is_none() {
        drop(references);
        if html.len() > MAX_PAGE_BYTES {
            return Err("HTML with inlined images exceeds the 25 MiB limit.".into());
        }
        return Ok((html, Vec::new()));
    }
    let mut roots = None;
    let mut paths = Some(paths);
    let mut cache: HashMap<&str, Option<String>> = HashMap::new();
    let mut missing = Vec::new();
    let mut output = String::with_capacity(html.len().min(MAX_PAGE_BYTES));
    let mut cursor = 0;
    for reference in references {
        let data = match cache.entry(reference.path) {
            Entry::Occupied(entry) => entry.into_mut(),
            Entry::Vacant(entry) => {
                let data = if is_local_path(reference.path) {
                    let roots =
                        roots.get_or_insert_with(|| image_roots(paths.take().unwrap_or_default()));
                    load_local_image(reference.path, roots)?
                } else {
                    None
                };
                if data.is_none() {
                    missing.push(reference.path.to_string());
                }
                entry.insert(data)
            }
        };
        if let Some(data) = data {
            push_page(&mut output, &html[cursor..reference.start])?;
            push_page(&mut output, data)?;
            cursor = reference.end;
        }
    }
    push_page(&mut output, &html[cursor..])?;
    Ok((output, missing))
}

fn prepare(html: String, paths: Vec<PathBuf>) -> Result<(String, Vec<String>), String> {
    reject_viewport_height(&html)?;
    inline_local_images(html, paths)
}

fn prepare_render(html: String, paths: Vec<PathBuf>) -> Result<String, String> {
    let (html, missing) = prepare(html, paths)?;
    if !missing.is_empty() {
        return Err(format!(
            "These local images could not be read or are not images: {}. Use absolute paths to existing image files, or remove them.",
            missing.join(", ")
        ));
    }
    Ok(html)
}

fn configured_working_path(configured: &str) -> Option<PathBuf> {
    let raw = if configured.trim().is_empty() {
        Cow::Owned(crate::settings::default_chat_working_directory())
    } else {
        Cow::Borrowed(configured.trim())
    };
    if matches!(raw.as_bytes().first(), Some(&b'/') | Some(&b'\\'))
        && matches!(raw.as_bytes().get(1), Some(&b'/') | Some(&b'\\'))
    {
        return None;
    }
    let home_relative = raw.strip_prefix("~/");
    #[cfg(windows)]
    let home_relative = home_relative.or_else(|| raw.strip_prefix("~\\"));
    let path = if &*raw == "~" {
        crate::native_tools::user_home_dir().ok()?
    } else if let Some(relative) = home_relative {
        crate::native_tools::user_home_dir().ok()?.join(relative)
    } else {
        let path = PathBuf::from(&*raw);
        if path.is_absolute() {
            path
        } else {
            crate::native_tools::user_home_dir().ok()?.join(path)
        }
    };
    path.to_str().filter(|path| is_local_path(path))?;
    Some(path)
}

#[cfg(unix)]
fn child_directory_exists(parent: &Path, child: &str) -> Option<bool> {
    use std::os::unix::{ffi::OsStrExt, io::AsRawFd};
    let directory = open_directory_root(parent)?;
    let name = std::ffi::CString::new(std::ffi::OsStr::new(child).as_bytes()).ok()?;
    let mut metadata = std::mem::MaybeUninit::<libc::stat>::uninit();
    let result = unsafe {
        libc::fstatat(
            directory.as_raw_fd(),
            name.as_ptr(),
            metadata.as_mut_ptr(),
            libc::AT_SYMLINK_NOFOLLOW,
        )
    };
    if result != 0 {
        return (std::io::Error::last_os_error().kind() == std::io::ErrorKind::NotFound)
            .then_some(false);
    }
    let kind = unsafe { metadata.assume_init() }.st_mode & libc::S_IFMT;
    if kind == libc::S_IFDIR {
        Some(true)
    } else if kind == libc::S_IFLNK {
        resolve_directory_root(&parent.join(child)).map(|_| true)
    } else {
        None
    }
}

#[cfg(windows)]
fn child_directory_exists(parent: &Path, child: &str) -> Option<bool> {
    let directory = open_directory_root(parent)?;
    match open_child_checked(&directory, std::ffi::OsStr::new(child), true, false) {
        Ok(_) => Some(true),
        Err(status) if matches!(status as u32, 0xc000_000f | 0xc000_0034 | 0xc000_003a) => {
            Some(false)
        }
        Err(_) => resolve_directory_root(&parent.join(child)).map(|_| true),
    }
}

fn ordinary_image_root(configured: &str, conversation_id: &str) -> Option<PathBuf> {
    let root = configured_working_path(configured)?;
    if child_directory_exists(&root, conversation_id)? {
        Some(root.join(conversation_id))
    } else {
        Some(root)
    }
}

fn external_legacy_root(app: &AppHandle, conversation_id: &str) -> Option<Option<PathBuf>> {
    let data = app.path().app_data_dir().ok()?;
    if !data.to_str().is_some_and(is_local_path) {
        return None;
    }
    if !child_directory_exists(&data, "chat-workspaces")? {
        return Some(None);
    }
    let parent = data.join("chat-workspaces");
    if child_directory_exists(&parent, conversation_id)? {
        Some(Some(parent.join(conversation_id)))
    } else {
        Some(None)
    }
}

fn conversation_image_paths(
    app: &AppHandle,
    conversation: &crate::chat::Conversation,
) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    let external = conversation.agent_runtime.is_external();
    let cwd = external
        .then(|| crate::external_agents::session::load_live_handle(app, &conversation.id))
        .flatten()
        .map(|handle| handle.cwd)
        .filter(|cwd| !cwd.trim().is_empty());
    match crate::chat::storage::resolve_conversation_project(app, conversation) {
        Ok(Some(project)) => {
            if let Some(root) = project.root_path {
                paths.push(PathBuf::from(root.trim()));
            }
        }
        Ok(None) if cwd.is_none() => {
            let legacy = if external {
                external_legacy_root(app, &conversation.id)
            } else {
                Some(None)
            };
            if let Some(legacy) = legacy {
                if let Some(root) = legacy {
                    paths.push(root);
                } else {
                    let settings = crate::settings::load_settings(app);
                    if let Some(root) = ordinary_image_root(
                        &settings.chat_tools.native_tools.working_directory,
                        &conversation.id,
                    ) {
                        paths.push(root);
                    }
                }
            }
        }
        _ => {}
    }
    if let Some(cwd) = cwd {
        paths.push(PathBuf::from(cwd));
    }
    paths.extend(
        conversation
            .additional_directories
            .iter()
            .map(|entry| PathBuf::from(&entry.path)),
    );
    if let Ok(path) = crate::chat::storage::conversation_attachments_path(app, &conversation.id) {
        paths.push(path);
    }
    paths
}

pub(crate) async fn publish(
    app: &AppHandle,
    conversation_id: &str,
    html: &str,
    title: &str,
    height: u32,
) -> Result<HtmlRenderReference, String> {
    validate_render(html, title, height)?;
    let conversation = crate::chat::repository::repository(app)
        .get(app, conversation_id)
        .await
        .map_err(crate::chat::repository::repository_error)?;
    let input = html.to_owned();
    let app_for_prepare = app.clone();
    let html = tauri::async_runtime::spawn_blocking(move || {
        prepare_render(
            input,
            conversation_image_paths(&app_for_prepare, &conversation),
        )
    })
    .await
    .map_err(|error| format!("HTML render task failed: {error}"))??;
    let attachment_id = format!("htmlrender-{}", uuid::Uuid::new_v4());
    crate::chat::repository::repository(app)
        .store_html_render(app, conversation_id, &attachment_id, html)
        .await
        .map_err(crate::chat::repository::repository_error)?;
    Ok(HtmlRenderReference {
        kind: HTML_RENDER_TOOL.to_owned(),
        attachment_id,
        title: title.trim().to_owned(),
        height,
    })
}

fn is_automation_preview(conversation_id: &str) -> bool {
    conversation_id.strip_prefix("auto_").is_some_and(|id| {
        !id.is_empty()
            && id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    })
}

pub(crate) async fn preview(
    app: &AppHandle,
    conversation_id: &str,
    html: &str,
    width: Option<u32>,
    appearance: Option<&str>,
) -> Result<HtmlPreviewOutput, String> {
    validate_html(html)?;
    let dark = match appearance {
        None | Some("dark") => true,
        Some("light") => false,
        Some(_) => return Err("Appearance must be 'dark' or 'light'.".into()),
    };
    let width = width
        .unwrap_or(COLUMN_WIDTH)
        .clamp(MIN_PREVIEW_WIDTH, MAX_PREVIEW_WIDTH);
    let input = html.to_owned();
    let conversation = if is_automation_preview(conversation_id) {
        None
    } else {
        Some(
            crate::chat::repository::repository(app)
                .get(app, conversation_id)
                .await
                .map_err(crate::chat::repository::repository_error)?,
        )
    };
    let app_for_prepare = app.clone();
    let (html, missing_images) = tauri::async_runtime::spawn_blocking(move || {
        let paths = conversation
            .as_ref()
            .map(|conversation| conversation_image_paths(&app_for_prepare, conversation))
            .unwrap_or_default();
        prepare(input, paths)
    })
    .await
    .map_err(|error| format!("HTML preview task failed: {error}"))??;
    let executable = browser::executable(app).await?;
    let captured = browser::capture(&executable, &html, width, dark).await?;
    Ok(HtmlPreviewOutput {
        png_base64: captured.png_base64,
        width,
        content_height: captured.content_height,
        captured_height: captured.captured_height,
        console: captured.console,
        missing_images,
    })
}

pub(crate) fn tool_definitions() -> Vec<Value> {
    let page_rules = "Provide a complete, self-contained HTML document with inline style and script. Local images use absolute paths within this conversation's working, project, additional or attachment directories; symlinks are supported when their targets remain in those directories. Remote URLs load as-is.";
    vec![
        json!({
            "name": HTML_RENDER_TOOL,
            "description": format!("Show a finished HTML page, such as a chart, diagram, or mockup, inline above your final reply. Do not announce or restate the page. {page_rules} {LAYOUT_GUIDE} {THEME_GUIDE}"),
            "inputSchema": {
                "type": "object",
                "properties": {
                    "html": {"type": "string", "minLength": 1, "maxLength": MAX_HTML_INPUT, "description": "Complete HTML document."},
                    "title": {"type": "string", "minLength": 1, "maxLength": MAX_TITLE},
                    "height": {"type": "integer", "minimum": MIN_HEIGHT, "maximum": MAX_HEIGHT, "description": "Frame height in CSS pixels; use contentHeight from html_preview."}
                },
                "required": ["html", "title", "height"]
            }
        }),
        json!({
            "name": HTML_PREVIEW_TOOL,
            "description": format!("Check a page before html_render. Returns a PNG, content height, console messages, and missing local images. If the browser is installing, call again in a minute. {page_rules} {LAYOUT_GUIDE} {THEME_GUIDE}"),
            "inputSchema": {
                "type": "object",
                "properties": {
                    "html": {"type": "string", "minLength": 1, "maxLength": MAX_HTML_INPUT, "description": "Complete HTML document."},
                    "width": {"type": "integer", "minimum": MIN_PREVIEW_WIDTH, "maximum": MAX_PREVIEW_WIDTH, "default": COLUMN_WIDTH, "description": "Viewport width in CSS pixels; use about 390 for phones."},
                    "appearance": {"type": "string", "enum": ["dark", "light"], "default": "dark"}
                },
                "required": ["html"]
            }
        }),
    ]
}

fn string_arg<'a>(args: &'a Value, name: &str) -> Result<&'a str, String> {
    args.get(name)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("{name} must be a string."))
}

fn integer_arg(args: &Value, name: &str) -> Result<u32, String> {
    args.get(name)
        .and_then(Value::as_u64)
        .and_then(|value| u32::try_from(value).ok())
        .ok_or_else(|| format!("{name} must be a non-negative integer."))
}

fn render_args(args: &Value) -> Result<(&str, &str, u32), String> {
    Ok((
        string_arg(args, "html")?,
        string_arg(args, "title")?,
        integer_arg(args, "height")?,
    ))
}

fn render_result(reference: HtmlRenderReference) -> Result<HtmlToolResult, String> {
    let text = format!("Rendered page {}.", reference.attachment_id);
    let structured = serde_json::to_value(reference).map_err(|error| error.to_string())?;
    Ok(HtmlToolResult {
        text,
        structured: Some(structured),
        image_png_base64: None,
    })
}

#[cfg(test)]
pub(crate) async fn call_render_in_directory(
    directory: &std::path::Path,
    args: &Value,
) -> Result<HtmlToolResult, String> {
    let (html, title, height) = render_args(args)?;
    validate_render(html, title, height)?;
    let input = html.to_owned();
    let title = title.trim().to_owned();
    let directory = directory.to_path_buf();
    let reference = tauri::async_runtime::spawn_blocking(move || {
        let html = prepare_render(input, vec![directory.clone()])?;
        let attachment_id = format!("htmlrender-{}", uuid::Uuid::new_v4());
        crate::chat::storage::atomic_write(
            &directory.join(format!("{attachment_id}.html")),
            &html,
            "HTML render",
        )?;
        Ok::<_, String>(HtmlRenderReference {
            kind: HTML_RENDER_TOOL.to_owned(),
            attachment_id,
            title,
            height,
        })
    })
    .await
    .map_err(|error| format!("HTML render task failed: {error}"))??;
    render_result(reference)
}

pub(crate) async fn call_tool(
    app: &AppHandle,
    conversation_id: &str,
    name: &str,
    args: &Value,
) -> Result<HtmlToolResult, String> {
    match name {
        HTML_RENDER_TOOL => {
            let (html, title, height) = render_args(args)?;
            let reference = publish(app, conversation_id, html, title, height).await?;
            render_result(reference)
        }
        HTML_PREVIEW_TOOL => {
            let width = if args.get("width").is_some() {
                Some(integer_arg(args, "width")?)
            } else {
                None
            };
            let appearance = if args.get("appearance").is_some() {
                Some(string_arg(args, "appearance")?)
            } else {
                None
            };
            let output = preview(
                app,
                conversation_id,
                string_arg(args, "html")?,
                width,
                appearance,
            )
            .await?;
            let metadata = json!({
                "width": output.width,
                "contentHeight": output.content_height,
                "capturedHeight": output.captured_height,
                "console": output.console,
                "missingImages": output.missing_images,
            });
            let text = serde_json::to_string(&metadata).map_err(|error| error.to_string())?;
            Ok(HtmlToolResult {
                text,
                structured: Some(metadata),
                image_png_base64: Some(output.png_base64),
            })
        }
        _ => Err(format!("Unknown HTML tool: {name}")),
    }
}

struct PendingRender {
    reference: HtmlRenderReference,
    created: Instant,
}

type PendingLedger = HashMap<String, VecDeque<PendingRender>>;

fn pending_renders() -> &'static Mutex<PendingLedger> {
    static PENDING: LazyLock<Mutex<PendingLedger>> = LazyLock::new(Default::default);
    &PENDING
}

fn prune_pending(ledger: &mut PendingLedger, now: Instant) {
    ledger.retain(|_, entries| {
        entries.retain(|entry| now.saturating_duration_since(entry.created) < PENDING_TTL);
        !entries.is_empty()
    });
}

/// Called only after an MCP render succeeds; native calls have structured results already.
pub(crate) fn record_pending_render(conversation_id: &str, reference: HtmlRenderReference) {
    let now = Instant::now();
    let mut ledger = pending_renders().lock();
    prune_pending(&mut ledger, now);
    ledger
        .entry(conversation_id.to_owned())
        .or_default()
        .push_back(PendingRender {
            reference,
            created: now,
        });
}

pub(crate) fn clear_pending_renders(conversation_id: &str) {
    pending_renders().lock().remove(conversation_id);
}

pub(crate) fn take_pending_render(
    conversation_id: &str,
    attachment_id: &str,
) -> Option<HtmlRenderReference> {
    let mut ledger = pending_renders().lock();
    prune_pending(&mut ledger, Instant::now());
    let entries = ledger.get_mut(conversation_id)?;
    let position = entries
        .iter()
        .position(|entry| entry.reference.attachment_id == attachment_id)?;
    let reference = entries.remove(position)?.reference;
    if entries.is_empty() {
        ledger.remove(conversation_id);
    }
    Some(reference)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs::File,
        io::{Seek, SeekFrom, Write},
    };

    fn reference(title: &str, id: &str) -> HtmlRenderReference {
        HtmlRenderReference {
            kind: HTML_RENDER_TOOL.into(),
            attachment_id: id.into(),
            title: title.into(),
            height: 480,
        }
    }

    #[test]
    fn reference_wire_shape_and_validation() {
        let render = reference("Chart", "htmlrender-12345678-1234-4234-9234-123456789abc");
        let wire = serde_json::to_value(&render).unwrap();
        assert_eq!(
            wire,
            json!({"type":"html_render", "attachmentId":render.attachment_id.clone(), "title":"Chart", "height":480})
        );
        assert_eq!(
            serde_json::from_value::<HtmlRenderReference>(wire).unwrap(),
            render
        );
        assert!(is_valid_render_attachment_id(&render.attachment_id));
        for invalid in [
            "htmlrender-12345678-1234-4234-9234-123456789ABc",
            "htmlrender-12345678123442349234123456789abc",
            "htmlrender-../../secret",
            "other-12345678-1234-4234-9234-123456789abc",
        ] {
            assert!(!is_valid_render_attachment_id(invalid), "{invalid}");
        }
        assert!(validate_render("<p>x</p>", " title ", MIN_HEIGHT).is_ok());
        assert!(validate_render("", "title", MIN_HEIGHT).is_err());
        assert!(validate_render("x", "   ", MIN_HEIGHT).is_err());
        assert!(validate_render("x", &"a".repeat(MAX_TITLE + 1), MIN_HEIGHT).is_err());
        assert!(validate_render("x", &format!("{}a", " ".repeat(MAX_TITLE)), MIN_HEIGHT).is_err());
        assert!(validate_render("x", "title", MAX_HEIGHT + 1).is_err());
        assert!(validate_html(&"x".repeat(MAX_HTML_INPUT + 1)).is_err());
        let definitions = tool_definitions();
        assert_eq!(definitions[0]["name"], HTML_RENDER_TOOL);
        assert_eq!(definitions[1]["name"], HTML_PREVIEW_TOOL);
        assert!(definitions[0]["description"]
            .as_str()
            .unwrap()
            .contains("--chart-6"));
    }

    #[test]
    fn viewport_height_is_rejected_only_in_css_and_not_comments_or_base64() {
        for html in [
            "<style>body { height: 50vh }</style>",
            "<style>.x { height: .5dVH }</style>",
            "<div style='height:12svh'></div>",
            "<p style=height:2lvh></p>",
            "<script>const html = '<div style=\"height: 5vh\">'</script>",
        ] {
            assert!(reject_viewport_height(html).is_err(), "{html}");
        }
        for html in [
            "<p>50vh</p>",
            "<!-- <style>height:50vh</style> --><style>height:20px</style>",
            "<style>/* 20vh */ .x { height: 20px }</style>",
            "<div style=\"background:url(data:image/png;base64,1vh==);height:20px\"></div>",
            "<style>.x{background:url(data:image/png;base64,/1vh+);content:'abc1vh';color:var(--2vh-name)}</style>",
        ] {
            assert!(reject_viewport_height(html).is_ok(), "{html}");
        }
        assert!(reject_viewport_height("<style>/* unclosed 50vh").is_ok());
        assert!(reject_viewport_height(
            "<style>background:url(data:image/png;base64,1vh==);height:10vh</style>"
        )
        .is_err());
    }

    #[test]
    fn image_signatures_and_svg_root_are_checked() {
        let examples: &[(&[u8], &str)] = &[
            (b"\x89PNG\r\n\x1a\ncontent", "image/png"),
            (b"\xff\xd8\xffcontent", "image/jpeg"),
            (b"GIF89acontent", "image/gif"),
            (b"RIFF1234WEBPcontent", "image/webp"),
            (b"BMxxxx\0\0\0\0content", "image/bmp"),
            (b"\0\0\x01\0content", "image/x-icon"),
            (b"\0\0\0\x18ftypavif", "image/avif"),
            (b"\xef\xbb\xbf <?xml version='1.0'?> <!--comment--> <!DOCTYPE svg [<!ENTITY x '>'>]><svg xmlns='http://www.w3.org/2000/svg'/>", "image/svg+xml"),
        ];
        for (bytes, mime) in examples {
            assert_eq!(image_mime(bytes), Some(*mime));
        }
        for bytes in [
            b"secret <svg/>".as_slice(),
            b"<svgx/>".as_slice(),
            b"<!--<svg/>".as_slice(),
            b"GIF8secret".as_slice(),
            b"\x89PNGsecret".as_slice(),
        ] {
            assert_eq!(image_mime(bytes), None);
        }
    }

    #[test]
    fn recognizes_absolute_quoted_and_css_image_paths_only() {
        let html = r#"<img src="C:\\Users\\me\\chart.PNG"><div style="background:url(/tmp/a.webp)"></div><img src="//cdn.example/x.png"><img src="relative.png"><img src="https://example.com/x.png">"#;
        let paths: Vec<_> = local_images(html).map(|item| item.path).collect();
        assert_eq!(paths, vec![r"C:\\Users\\me\\chart.PNG", "/tmp/a.webp"]);
    }

    #[test]
    fn local_images_are_inlined_once_and_missing_files_are_reported() {
        let dir = tempfile::tempdir().unwrap();
        let image = dir.path().join("image with spaces.PNG");
        let secret = dir.path().join("secret.png");
        let missing = dir.path().join("missing.jpg");
        fs::write(&image, b"\x89PNG\r\n\x1a\nimage").unwrap();
        fs::write(&secret, b"private data, not an image").unwrap();
        let html = format!("<img src=\"{}\"><img src='{}'><img src='{}'><img src='{}'><img src='{}'><img src='/relative-without-extension'><img src='https://example.com/x.png'>", image.display(), image.display(), secret.display(), missing.display(), missing.display());
        let (prepared, missing_images) =
            inline_local_images(html, vec![dir.path().to_path_buf()]).unwrap();
        assert_eq!(prepared.matches("data:image/png;base64,").count(), 2);
        assert!(!prepared.contains("private data"));
        assert_eq!(
            missing_images,
            vec![
                secret.to_string_lossy().to_string(),
                missing.to_string_lossy().to_string()
            ]
        );
        assert!(prepared.contains("https://example.com/x.png"));
        assert!(prepared.contains(&secret.to_string_lossy().to_string()));
    }

    #[test]
    fn local_image_and_total_page_limits_are_enforced() {
        let dir = tempfile::tempdir().unwrap();
        let image = dir.path().join("large.png");
        let mut file = File::create(&image).unwrap();
        file.write_all(b"\x89PNG\r\n\x1a\n").unwrap();
        file.set_len(MAX_IMAGE_BYTES + 1).unwrap();
        let html = format!("<img src=\"{}\">", image.display());
        assert!(
            inline_local_images(html.clone(), vec![dir.path().to_path_buf()])
                .unwrap_err()
                .contains("10 MiB")
        );
        file.set_len(MAX_IMAGE_BYTES).unwrap();
        file.seek(SeekFrom::Start(0)).unwrap();
        file.write_all(b"\x89PNG\r\n\x1a\n").unwrap();
        drop(file);
        assert!(
            inline_local_images(format!("{html}{html}"), vec![dir.path().to_path_buf()])
                .unwrap_err()
                .contains("25 MiB")
        );
    }

    #[test]
    fn only_conversation_roots_are_inlined_and_outside_images_are_missing() {
        let temp = tempfile::tempdir().unwrap();
        let working = temp.path().join("working");
        let additional = temp.path().join("additional");
        let attachments = temp.path().join("attachments");
        let outside = temp.path().join("working-other");
        for directory in [&working, &additional, &attachments, &outside] {
            fs::create_dir(directory).unwrap();
            fs::write(directory.join("image.png"), b"\x89PNG\r\n\x1a\nimage").unwrap();
        }
        fs::create_dir(working.join("subdir")).unwrap();
        let mut inside: Vec<_> = [&working, &additional, &attachments]
            .iter()
            .map(|path| path.join("image.png"))
            .collect();
        inside.push(working.join("subdir").join("..").join("image.png"));
        let secret = outside.join("image.png");
        File::options()
            .write(true)
            .open(&secret)
            .unwrap()
            .set_len(MAX_IMAGE_BYTES + 1)
            .unwrap();
        let traversal = working.join("..").join("working-other").join("image.png");
        let html = inside
            .iter()
            .chain([&secret, &traversal])
            .map(|path| format!("<img src='{}'>", path.display()))
            .collect::<String>();
        let roots = vec![working, additional, attachments];
        let (preview, missing) = prepare(html.clone(), roots.clone()).unwrap();
        assert_eq!(preview.matches("data:image/png;base64,").count(), 4);
        assert_eq!(
            missing,
            vec![
                secret.to_string_lossy().to_string(),
                traversal.to_string_lossy().to_string()
            ]
        );
        let error = prepare_render(html, roots).unwrap_err();
        assert!(error.contains(&secret.to_string_lossy().to_string()));
        assert!(error.contains(&traversal.to_string_lossy().to_string()));
    }

    #[tokio::test]
    async fn rendered_tool_result_inlines_allowed_images_and_rejects_outside_images() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("attachments");
        let outside = temp.path().join("secret.png");
        fs::create_dir(&root).unwrap();
        let inside = root.join("image.png");
        fs::write(&inside, b"\x89PNG\r\n\x1a\ninside").unwrap();
        fs::write(&outside, b"\x89PNG\r\n\x1a\nsecret").unwrap();
        let args = json!({
            "html": format!("<img src='{}'>", inside.display()),
            "title": "Chart", "height": 480
        });
        let output = call_render_in_directory(&root, &args).await.unwrap();
        let id = output.structured.as_ref().unwrap()["attachmentId"]
            .as_str()
            .unwrap();
        assert_eq!(output.text, format!("Rendered page {id}."));
        let page = fs::read_to_string(root.join(format!("{id}.html"))).unwrap();
        assert!(page.contains("data:image/png;base64,"));
        let denied = json!({
            "html": format!("<img src='{}'>", outside.display()),
            "title": "Chart", "height": 480
        });
        let error = call_render_in_directory(&root, &denied)
            .await
            .err()
            .unwrap();
        assert!(error.contains(&outside.to_string_lossy().to_string()));
    }

    #[test]
    fn network_and_device_references_are_rejected_before_opening() {
        for path in [
            r"/\attacker.example\share\image.png",
            r"\\attacker.example\share\image.png",
            r"\\?\UNC\attacker.example\share\image.png",
            r"\\?\C:\secret.png",
            r"\\.\C:\secret.png",
        ] {
            let html = format!("<img src='{path}'>");
            let (_, missing) = prepare(html, Vec::new()).unwrap();
            assert_eq!(missing, vec![path.to_owned()]);
            assert!(open_local_image(path, &[]).is_none());
            assert!(!is_local_path(path));
            assert!(image_roots(vec![PathBuf::from(path)]).is_empty());
            assert!(configured_working_path(path).is_none());
        }
        assert!(open_local_image("//attacker.example/share/image.png", &[]).is_none());
        assert!(image_roots(vec![PathBuf::from("//attacker.example/share")]).is_empty());
    }

    #[test]
    fn configured_working_paths_expand_home_before_root_screening() {
        let home = crate::native_tools::user_home_dir().unwrap();
        let expected = home.join("kivio-root-test");
        if expected.to_str().is_some_and(is_local_path) {
            assert_eq!(
                configured_working_path("~/kivio-root-test"),
                Some(expected.clone())
            );
            #[cfg(windows)]
            assert_eq!(
                configured_working_path("~\\kivio-root-test"),
                Some(expected)
            );
            assert_eq!(configured_working_path("a/b"), Some(home.join("a/b")));
        } else {
            assert!(configured_working_path("~/kivio-root-test").is_none());
        }
    }

    #[cfg(unix)]
    #[test]
    fn configured_root_symlinks_reject_network_spelling_before_open() {
        use std::os::unix::fs::symlink;
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("root-link");
        symlink("//unresolvable.invalid/share", &root).unwrap();
        assert!(image_roots(vec![root]).is_empty());
        let conversation = temp.path().join("conv_test");
        symlink("/\\unresolvable.invalid/share", &conversation).unwrap();
        assert!(ordinary_image_root(temp.path().to_str().unwrap(), "conv_test").is_none());
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_roots_are_canonicalized_but_child_links_cannot_escape() {
        use std::os::unix::fs::symlink;
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("working");
        let outside = temp.path().join("outside");
        fs::create_dir(&root).unwrap();
        fs::create_dir(&outside).unwrap();
        fs::create_dir(root.join("nested")).unwrap();
        fs::write(root.join("inside.png"), b"\x89PNG\r\n\x1a\ninside").unwrap();
        fs::write(
            root.join("nested").join("inside.png"),
            b"\x89PNG\r\n\x1a\nnested",
        )
        .unwrap();
        fs::write(outside.join("secret.png"), b"\x89PNG\r\n\x1a\nsecret").unwrap();
        let alias = temp.path().join("root-alias");
        symlink(&root, &alias).unwrap();
        symlink("inside.png", root.join("local.png")).unwrap();
        symlink("nested", root.join("local-dir")).unwrap();
        symlink(root.join("inside.png"), root.join("absolute.png")).unwrap();
        symlink(&outside, root.join("escape")).unwrap();
        symlink("../outside", root.join("other-root")).unwrap();
        symlink(outside.join("secret.png"), root.join("escape.png")).unwrap();
        symlink("cycle-b.png", root.join("cycle-a.png")).unwrap();
        symlink("cycle-a.png", root.join("cycle-b.png")).unwrap();
        let good = [
            alias.join("inside.png"),
            root.join("local.png"),
            root.join("local-dir").join("inside.png"),
            root.join("absolute.png"),
        ];
        let through_dir = root.join("escape").join("secret.png");
        let through_parent = root.join("escape").join("..").join("inside.png");
        let through_file = root.join("escape.png");
        let cycle = root.join("cycle-a.png");
        let html = good
            .iter()
            .chain([&through_dir, &through_file, &through_parent, &cycle])
            .map(|path| format!("<img src='{}'>", path.display()))
            .collect::<String>();
        let (prepared, missing) = prepare(html, vec![alias, root.clone()]).unwrap();
        assert_eq!(prepared.matches("data:image/png;base64,").count(), 4);
        assert_eq!(
            missing,
            vec![
                through_dir.to_string_lossy().to_string(),
                through_file.to_string_lossy().to_string(),
                through_parent.to_string_lossy().to_string(),
                cycle.to_string_lossy().to_string(),
            ]
        );
        let cross_root = root.join("other-root").join("secret.png");
        let html = format!("<img src='{}'>", cross_root.display());
        let (prepared, missing) = prepare(html, vec![root, outside]).unwrap();
        assert!(prepared.contains("data:image/png;base64,"));
        assert!(missing.is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn replacing_an_allowed_directory_cannot_retarget_an_open_root() {
        use std::os::unix::fs::symlink;
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("working");
        let moved = temp.path().join("original");
        let outside = temp.path().join("outside");
        fs::create_dir(&root).unwrap();
        fs::create_dir(&outside).unwrap();
        fs::write(root.join("image.png"), b"\x89PNG\r\n\x1a\ninside").unwrap();
        fs::write(outside.join("image.png"), b"\x89PNG\r\n\x1a\nsecret").unwrap();
        let roots = image_roots(vec![root.clone()]);
        fs::rename(&root, moved).unwrap();
        symlink(&outside, &root).unwrap();
        let reference = root.join("image.png").to_string_lossy().to_string();
        let mut opened = open_local_image(&reference, &roots).unwrap();
        let mut bytes = Vec::new();
        opened.read_to_end(&mut bytes).unwrap();
        assert_eq!(bytes, b"\x89PNG\r\n\x1a\ninside");
    }

    #[cfg(windows)]
    #[test]
    fn child_symlinks_are_allowed_only_when_their_targets_are_in_scope() {
        use std::os::windows::fs::symlink_file;
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("working");
        let outside = temp.path().join("secret.png");
        fs::create_dir(&root).unwrap();
        let inside = root.join("inside.png");
        fs::write(&inside, b"\x89PNG\r\n\x1a\ninside").unwrap();
        fs::write(&outside, b"\x89PNG\r\n\x1a\nsecret").unwrap();
        let allowed = root.join("allowed.png");
        let denied = root.join("denied.png");
        if symlink_file(&inside, &allowed).is_ok() && symlink_file(&outside, &denied).is_ok() {
            let html = format!(
                "<img src='{}'><img src='{}'>",
                allowed.display(),
                denied.display()
            );
            let (prepared, missing) = prepare(html, vec![root]).unwrap();
            assert_eq!(prepared.matches("data:image/png;base64,").count(), 1);
            assert_eq!(missing, vec![denied.to_string_lossy().to_string()]);
        }
    }

    #[cfg(windows)]
    #[test]
    fn configured_roots_cannot_follow_unc_junction_targets() {
        use std::os::windows::fs::symlink_dir;
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("root-link");
        if symlink_dir(r"\\unresolvable.invalid\share", &root).is_ok() {
            assert!(image_roots(vec![root]).is_empty());
        }
        let conversation = temp.path().join("conv_test");
        if symlink_dir(r"\\unresolvable.invalid\share", &conversation).is_ok() {
            assert!(ordinary_image_root(temp.path().to_str().unwrap(), "conv_test").is_none());
        }
    }

    #[cfg(windows)]
    #[test]
    fn reparse_targets_never_follow_unc_or_device_names() {
        fn symlink_data(target: &str, relative: bool) -> Vec<u8> {
            let name: Vec<u16> = target.encode_utf16().collect();
            let length = name.len() * 2;
            let mut data = vec![0u8; 20 + length];
            data[..4].copy_from_slice(&0xa000_000cu32.to_le_bytes());
            data[4..6].copy_from_slice(&((12 + length) as u16).to_le_bytes());
            data[10..12].copy_from_slice(&(length as u16).to_le_bytes());
            data[16..20].copy_from_slice(&(if relative { 1u32 } else { 0u32 }).to_le_bytes());
            for (chunk, character) in data[20..].chunks_exact_mut(2).zip(name) {
                chunk.copy_from_slice(&character.to_le_bytes());
            }
            data
        }
        assert_eq!(
            decode_reparse_target(&symlink_data(r"\??\C:\working\image.png", false)),
            Some(PathBuf::from(r"C:\working\image.png")),
        );
        assert_eq!(
            decode_reparse_target(&symlink_data(r"..\working\image.png", true)),
            Some(PathBuf::from(r"..\working\image.png")),
        );
        for target in [
            r"\??\UNC\attacker.example\share\image.png",
            r"\??\GLOBALROOT\Device\HarddiskVolume1\image.png",
            r"\Device\Mup\attacker.example\share\image.png",
        ] {
            assert!(
                decode_reparse_target(&symlink_data(target, false)).is_none(),
                "{target}"
            );
        }
        assert!(
            decode_reparse_target(&symlink_data(r"\\attacker.example\share\image.png", true))
                .is_none()
        );
    }

    #[test]
    fn automation_preview_has_no_local_image_roots() {
        assert!(is_automation_preview("auto_job-1"));
        for invalid in [
            "auto_",
            "auto_../secret",
            "auto_job/secret",
            "conv_auto_job",
        ] {
            assert!(!is_automation_preview(invalid), "{invalid}");
        }
        let html = "<p>safe preview</p>".to_owned();
        let (prepared, missing) = prepare(html.clone(), Vec::new()).unwrap();
        assert_eq!(prepared, html);
        assert!(missing.is_empty());
        let temp = tempfile::tempdir().unwrap();
        let image = temp.path().join("image.png");
        fs::write(&image, b"\x89PNG\r\n\x1a\nprivate").unwrap();
        let html = format!("<img src='{}'>", image.display());
        let (prepared, missing) = prepare(html.clone(), Vec::new()).unwrap();
        assert_eq!(prepared, html);
        assert_eq!(missing, vec![image.to_string_lossy().to_string()]);
    }

    #[test]
    fn pending_is_per_conversation_by_attachment_id_and_expires() {
        let first = format!("conv_{}", uuid::Uuid::new_v4());
        let second = format!("conv_{}", uuid::Uuid::new_v4());
        record_pending_render(&first, reference("same", "first"));
        record_pending_render(&first, reference("other", "other"));
        record_pending_render(&first, reference("same", "second"));
        record_pending_render(&second, reference("same", "separate"));
        assert_eq!(
            take_pending_render(&first, "second").unwrap().attachment_id,
            "second"
        );
        assert!(take_pending_render(&second, "first").is_none());
        assert_eq!(
            take_pending_render(&second, "separate")
                .unwrap()
                .attachment_id,
            "separate"
        );
        assert_eq!(
            take_pending_render(&first, "first").unwrap().attachment_id,
            "first"
        );
        {
            let mut ledger = pending_renders().lock();
            ledger.get_mut(&first).unwrap().front_mut().unwrap().created =
                Instant::now() - PENDING_TTL;
        }
        assert!(take_pending_render(&first, "other").is_none());
        assert!(!pending_renders().lock().contains_key(&first));
    }

    #[test]
    fn revocation_clears_only_the_named_conversation() {
        let first = format!("conv_{}", uuid::Uuid::new_v4());
        let second = format!("conv_{}", uuid::Uuid::new_v4());
        record_pending_render(&first, reference("first", "first"));
        record_pending_render(&first, reference("other", "other"));
        record_pending_render(&second, reference("second", "second"));
        clear_pending_renders(&first);
        assert!(take_pending_render(&first, "first").is_none());
        assert!(take_pending_render(&first, "other").is_none());
        assert_eq!(
            take_pending_render(&second, "second")
                .unwrap()
                .attachment_id,
            "second"
        );
    }
}
