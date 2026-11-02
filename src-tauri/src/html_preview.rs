//! Sandboxed generated HTML is served from an origin without the application's IPC access.
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use std::{
    collections::HashMap,
    sync::{LazyLock, Mutex, OnceLock},
    time::{Duration, Instant},
};
const MAX_HTML_BYTES: usize = 2 * 1024 * 1024;
const MAX_PREVIEWS: usize = 32;
const PREVIEW_TTL: Duration = Duration::from_secs(3600);
pub(crate) const PREVIEW_CSP: &str = "default-src 'none'; script-src 'unsafe-inline' https:; style-src 'unsafe-inline' https:; img-src https: data: blob:; font-src https: data:; connect-src https:; frame-src 'none'; object-src 'none'; base-uri 'none'; form-action 'none'; sandbox allow-scripts";
// Frame height follows the page, so the page never scrolls itself below the 2000px cap: a classic (WebView2) scrollbar would narrow the page and change its height mid-measure.
const BOOTSTRAP: &str = r#"<style id="kivio-preview-theme" data-kivio-theme="">html,body{margin:0;background:transparent}body{color:var(--foreground,#1d1d1f);font:14px/1.6 var(--font-sans,system-ui,sans-serif);overflow-wrap:anywhere}a{color:var(--accent,#2f6ff0)}</style><script>(function(){var s=document.getElementById('kivio-preview-theme'),v=s&&s.getAttribute('data-kivio-theme'),h=location.hash;if(h.indexOf('#kivio-theme=')===0){v=h.slice(13);try{history.replaceState(null,'',location.href.split('#')[0])}catch(_){try{location.hash=''}catch(_){}}}if(s){s.removeAttribute('data-kivio-theme');if(v)try{var b=atob(v.replace(/-/g,'+').replace(/_/g,'/'));s.textContent=new TextDecoder().decode(Uint8Array.from(b,function(c){return c.charCodeAt(0)}))}catch(_){}}var K='kivioHtmlPreview',last=-1,lastVh=-1,lastD=0,frozen=false;function send(t,v){parent.postMessage({[K]:t,value:v},'*')}function post(){var b=document.body,e=document.documentElement;if(!b||frozen)return;if(parent!==window)e.style.overflowY='hidden';var h=Math.ceil(Math.max(e.scrollHeight,e.getBoundingClientRect().height)),vh=innerHeight,d=h-vh;if(d>0&&d===lastD&&vh!==lastVh&&lastVh>=0){frozen=true;e.style.overflowY='auto';send('height',520);return}lastVh=vh;lastD=d;if(parent!==window)e.style.overflowY=h>2000?'auto':'hidden';if(h!==last){last=h;send('height',h)}}addEventListener('DOMContentLoaded',function(){new ResizeObserver(post).observe(document.body);post()});addEventListener('load',post);addEventListener('message',function(e){var d=e.data;if(e.source!==parent||!d||d[K]!=='theme'||typeof d.value!=='string')return;var s=document.getElementById('kivio-preview-theme');if(s)s.textContent=d.value});document.addEventListener('click',function(e){if(parent===window)return;var a=e.target&&e.target.closest&&e.target.closest('a[href]');if(!a||!/^https?:/i.test(a.href))return;e.preventDefault();send('open',a.href)},true)})()</script>"#;

static FULL_DOCUMENT: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(r"(?i)^(?:<!doctype\s+html[^>]*>\s*)?<html[\s>]")
        .expect("static HTML pattern")
});

fn find_tag(page: &str, target: &[u8], want_closing: bool) -> Option<usize> {
    let bytes = page.as_bytes();
    let mut cursor = 0;
    while cursor < bytes.len() {
        let start = cursor + bytes[cursor..].iter().position(|byte| *byte == b'<')?;
        if bytes[start..].starts_with(b"<!--") {
            let end = bytes[start + 4..]
                .windows(3)
                .position(|part| part == b"-->")?;
            cursor = start + 4 + end + 3;
            continue;
        }
        let mut name_start = start + 1;
        let closing = bytes.get(name_start) == Some(&b'/');
        if closing {
            name_start += 1;
        }
        let mut name_end = name_start;
        while bytes
            .get(name_end)
            .is_some_and(|byte| byte.is_ascii_alphanumeric() || matches!(*byte, b'-' | b'_' | b':'))
        {
            name_end += 1;
        }
        let mut end = name_end;
        let mut quote = None;
        while let Some(&byte) = bytes.get(end) {
            if let Some(delimiter) = quote {
                if byte == delimiter {
                    quote = None;
                }
            } else if byte == b'\'' || byte == b'"' {
                quote = Some(byte);
            } else if byte == b'>' {
                break;
            }
            end += 1;
        }
        if end == bytes.len() {
            return None;
        }
        end += 1;
        let name = &bytes[name_start..name_end];
        if closing == want_closing && name.eq_ignore_ascii_case(target) {
            return Some(end);
        }
        if target == b"head"
            && !want_closing
            && !closing
            && !name.is_empty()
            && !name.eq_ignore_ascii_case(b"html")
        {
            return None;
        }
        if !closing && name.eq_ignore_ascii_case(b"plaintext") {
            return None;
        }
        if !closing
            && [
                b"script".as_slice(),
                b"style".as_slice(),
                b"textarea".as_slice(),
                b"title".as_slice(),
                b"xmp".as_slice(),
                b"iframe".as_slice(),
                b"noembed".as_slice(),
                b"noframes".as_slice(),
                b"noscript".as_slice(),
            ]
            .iter()
            .any(|raw| name.eq_ignore_ascii_case(raw))
        {
            let mut raw_cursor = end;
            loop {
                let next = raw_cursor
                    + bytes[raw_cursor..]
                        .windows(2)
                        .position(|part| part == b"</")?;
                let close_start = next + 2;
                let close_end = close_start + name.len();
                if bytes
                    .get(close_start..close_end)
                    .is_some_and(|candidate| candidate.eq_ignore_ascii_case(name))
                    && bytes.get(close_end).is_some_and(|byte| {
                        byte.is_ascii_whitespace() || *byte == b'>' || *byte == b'/'
                    })
                {
                    cursor = next;
                    break;
                }
                raw_cursor = close_start;
            }
        } else {
            cursor = end;
        }
    }
    None
}

fn inject_bootstrap(html: &str) -> String {
    inject_bootstrap_with_theme(html, None)
}

fn inject_bootstrap_with_theme(html: &str, theme_css: Option<&str>) -> String {
    let bootstrap = match theme_css {
        Some(css) => std::borrow::Cow::Owned(BOOTSTRAP.replacen(
            r#"data-kivio-theme="""#,
            &format!(r#"data-kivio-theme="{}""#, URL_SAFE_NO_PAD.encode(css)),
            1,
        )),
        None => std::borrow::Cow::Borrowed(BOOTSTRAP),
    };
    let trimmed = html.trim();
    let full_document = FULL_DOCUMENT.is_match(trimmed);
    let mut page = std::borrow::Cow::Borrowed(if full_document { trimmed } else { html });
    if full_document && find_tag(&page, b"html", true).is_none() {
        if find_tag(&page, b"style", false).is_some() && find_tag(&page, b"style", true).is_none() {
            page.to_mut().push_str("\n</style>");
        }
        if find_tag(&page, b"head", false).is_some() && find_tag(&page, b"head", true).is_none() {
            page.to_mut().push_str("\n</head>");
        }
        if find_tag(&page, b"body", false).is_none() {
            page.to_mut().push_str("\n<body></body>");
        }
        if find_tag(&page, b"body", true).is_none() {
            page.to_mut().push_str("\n</body>");
        }
        if find_tag(&page, b"html", true).is_none() {
            page.to_mut().push_str("\n</html>");
        }
    }
    let mut output = String::with_capacity(page.len() + bootstrap.len() + "<!doctype html>".len());
    if let Some(end) = find_tag(&page, b"head", false).or_else(|| find_tag(&page, b"html", false)) {
        output.push_str(&page[..end]);
        output.push_str(&bootstrap);
        output.push_str(&page[end..]);
    } else {
        output.push_str("<!doctype html>");
        output.push_str(&bootstrap);
        output.push_str(&page);
    }
    output
}

fn decode_path(path: &str) -> Option<std::borrow::Cow<'_, str>> {
    if !path.contains('%') {
        return Some(std::borrow::Cow::Borrowed(path));
    }
    let input = path.as_bytes();
    let mut output = Vec::with_capacity(input.len());
    let mut index = 0;
    while index < input.len() {
        if input[index] == b'%' {
            let high = char::from(*input.get(index + 1)?).to_digit(16)? as u8;
            let low = char::from(*input.get(index + 2)?).to_digit(16)? as u8;
            output.push(high * 16 + low);
            index += 3;
        } else {
            output.push(input[index]);
            index += 1;
        }
    }
    String::from_utf8(output).ok().map(std::borrow::Cow::Owned)
}

fn render_ids(path: &str) -> Option<(&str, &str)> {
    let mut parts = path.split('/');
    if parts.next()? != "render" {
        return None;
    }
    let conversation_id = parts.next()?;
    let attachment_id = parts.next()?;
    if parts.next().is_some()
        || crate::chat::storage::validate_conversation_id(conversation_id).is_err()
    {
        return None;
    }
    if !crate::html_render::is_valid_render_attachment_id(attachment_id) {
        return None;
    }
    Some((conversation_id, attachment_id))
}

fn read_render(
    app: &tauri::AppHandle,
    conversation_id: &str,
    attachment_id: &str,
) -> Result<String, String> {
    let route = format!("render/{conversation_id}/{attachment_id}");
    render_ids(&route).ok_or_else(|| "Invalid HTML render reference".to_string())?;
    let dir = crate::chat::storage::conversation_attachments_path(app, conversation_id)?;
    std::fs::read_to_string(dir.join(format!("{attachment_id}.html")))
        .map_err(|error| format!("Read HTML render failed: {error}"))
}

fn render_page(
    app: &tauri::AppHandle,
    conversation_id: &str,
    attachment_id: &str,
) -> Option<String> {
    read_render(app, conversation_id, attachment_id)
        .ok()
        .map(|html| inject_bootstrap(&html))
}

#[tauri::command]
pub(crate) fn open_html_render(
    app: tauri::AppHandle,
    conversation_id: String,
    attachment_id: String,
    theme_css: String,
) -> Result<(), String> {
    let html = read_render(&app, &conversation_id, &attachment_id)?;
    crate::commands::open_html_preview(app, inject_bootstrap_with_theme(&html, Some(&theme_css)))
}

struct Preview {
    html: String,
    created: Instant,
}
fn previews() -> &'static Mutex<HashMap<String, Preview>> {
    static PREVIEWS: OnceLock<Mutex<HashMap<String, Preview>>> = OnceLock::new();
    PREVIEWS.get_or_init(Default::default)
}
#[tauri::command]
pub(crate) fn create_html_preview(html: String) -> Result<String, String> {
    if html.len() > MAX_HTML_BYTES {
        return Err("HTML preview exceeds 2 MiB".into());
    }
    let mut previews = previews().lock().map_err(|_| "HTML preview unavailable")?;
    previews.retain(|_, preview| preview.created.elapsed() < PREVIEW_TTL);
    if previews.len() >= MAX_PREVIEWS {
        return Err("Too many HTML previews are open".into());
    }
    let id = uuid::Uuid::new_v4().to_string();
    previews.insert(
        id.clone(),
        Preview {
            html: inject_bootstrap(&html),
            created: Instant::now(),
        },
    );
    Ok(id)
}
#[tauri::command]
pub(crate) fn release_html_preview(id: String) {
    if let Ok(mut previews) = previews().lock() {
        previews.remove(&id);
    }
}
pub(crate) fn serve(
    app: &tauri::AppHandle,
    request: tauri::http::Request<Vec<u8>>,
) -> tauri::http::Response<Vec<u8>> {
    serve_with(request, |conversation_id, attachment_id| {
        render_page(app, conversation_id, attachment_id)
    })
}

fn serve_with(
    request: tauri::http::Request<Vec<u8>>,
    load_render: impl FnOnce(&str, &str) -> Option<String>,
) -> tauri::http::Response<Vec<u8>> {
    let id = decode_path(request.uri().path().trim_start_matches('/'));
    let body = if request.method() != tauri::http::Method::GET {
        None
    } else {
        id.as_deref().and_then(|id| {
            if id.starts_with("render/") {
                render_ids(id)
                    .and_then(|(conversation, attachment)| load_render(conversation, attachment))
            } else {
                previews().lock().ok().and_then(|mut previews| {
                    if previews.get(id)?.created.elapsed() >= PREVIEW_TTL {
                        previews.remove(id);
                        None
                    } else {
                        previews.get(id).map(|preview| preview.html.clone())
                    }
                })
            }
        })
    };
    let status = if body.is_some() { 200 } else { 404 };
    let body = body.unwrap_or_else(|| {
        if request.method() == tauri::http::Method::GET {
            "<!doctype html><script>parent.postMessage({kivioHtmlPreview:'error'},'*')</script>"
                .to_owned()
        } else {
            String::new()
        }
    });
    tauri::http::Response::builder()
        .status(status)
        .header("Content-Type", "text/html; charset=utf-8")
        .header("Content-Security-Policy", PREVIEW_CSP)
        .header("Cache-Control", "no-store")
        .header("X-Content-Type-Options", "nosniff")
        .body(body.into_bytes())
        .expect("static preview response headers")
}
#[cfg(test)]
mod tests {
    fn serve(request: tauri::http::Request<Vec<u8>>) -> tauri::http::Response<Vec<u8>> {
        super::serve_with(request, |_, _| None)
    }

    #[test]
    fn bootstrap_skips_comment_and_header_tags_and_repairs_fragments() {
        let input = "<!-- <head> --><html><head><title>T</title></head><body>x</body></html>";
        let result = super::inject_bootstrap(input);
        assert!(result.starts_with("<!-- <head> --><html><head><style id=\"kivio-preview-theme\""));
        assert!(result.find("kivio-preview-theme").unwrap() < result.find("<title>").unwrap());
        let header = super::inject_bootstrap("<header>not a head</header>");
        assert!(header.starts_with("<!doctype html><style id=\"kivio-preview-theme\""));
        assert!(header.ends_with("<header>not a head</header>"));
        let no_head = super::inject_bootstrap("<html><body>x</body></html>");
        assert!(no_head.starts_with("<html><style id=\"kivio-preview-theme\""));
        let incomplete = super::inject_bootstrap("<html><head><style>body{color:red}");
        assert!(incomplete.contains("</style>\n</head>\n<body></body>\n</html>"));
    }

    #[test]
    fn bootstrap_ignores_tags_inside_attributes_and_raw_text() {
        for (html, prefix) in [
            (
                r#"<html data-example="<head>"><body>x</body></html>"#,
                r#"<html data-example="<head>"><style id="kivio-preview-theme""#,
            ),
            (
                r#"<html><script>const sample = "<head>";</script><body>x</body></html>"#,
                r#"<html><style id="kivio-preview-theme""#,
            ),
            (
                r#"<html><style>p::before { content: "<head>"; }</style><body>x</body></html>"#,
                r#"<html><style id="kivio-preview-theme""#,
            ),
            (
                r#"<html><textarea><head></textarea><body>x</body></html>"#,
                r#"<html><style id="kivio-preview-theme""#,
            ),
            (
                r#"<html><title><head></title><body>x</body></html>"#,
                r#"<html><style id="kivio-preview-theme""#,
            ),
            (
                r#"<html><div><head>not a document head</head></div></html>"#,
                r#"<html><style id="kivio-preview-theme""#,
            ),
            (
                r#"<html><head data-quoted=">"><title>x</title></head><body></body></html>"#,
                r#"<html><head data-quoted=">"><style id="kivio-preview-theme""#,
            ),
        ] {
            let output = super::inject_bootstrap(html);
            assert!(output.starts_with(prefix), "{output}");
            assert!(output.ends_with("</html>"));
        }
    }

    #[test]
    fn initial_theme_is_decoded_before_page_scripts_and_external_css_is_encoded() {
        use base64::Engine as _;
        let css = ":root{--chart-1:red;--label:'世界</style><script>bad()</script>'}";
        let encoded = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(css);
        let html = "<html><head><script>window.chart=getComputedStyle(document.documentElement).getPropertyValue('--chart-1')</script></head><body></body></html>";
        let output = super::inject_bootstrap_with_theme(html, Some(css));
        assert!(output.contains(&format!(r#"data-kivio-theme="{encoded}""#)));
        assert!(!output.contains(css));
        assert!(output.find("new TextDecoder()").unwrap() < output.find("window.chart=").unwrap());
        assert!(output.contains("location.hash"));
        assert!(output.contains("history.replaceState"));
        assert!(output.contains("parent===window"));
        assert!(super::inject_bootstrap(html).contains("data-kivio-theme=\"\""));
    }

    #[test]
    fn render_route_requires_valid_single_segment_ids() {
        let id = "htmlrender-12345678-1234-4123-8123-123456789abc";
        let good = format!("render/conv_abc-123/{id}");
        assert_eq!(super::render_ids(&good), Some(("conv_abc-123", id)));
        for path in [
            format!("render/../{id}"),
            format!("render/conv_../{id}"),
            format!("render/conv_%2e%2e/{id}"),
            "render/conv_ok/../../secret".into(),
            "render/conv_ok/htmlrender-../../secret".into(),
            format!("render%2Fconv_ok%2F..%2F{id}"),
            format!("render%252Fconv_ok%252F{id}"),
            format!("render%2Fconv_ok%2F{id}%2Fextra"),
            format!("render%2Fconv_%252e%252e%2F{id}"),
            format!("render/conv_ok/{id}/extra"),
            format!("render/conv_ok/{}", id.to_uppercase()),
            "render/conv_ok/htmlrender-12345678------------------------------------".into(),
        ] {
            assert!(super::render_ids(&path).is_none(), "{path}");
            let request = tauri::http::Request::builder()
                .uri(format!("kivio-preview://localhost/{path}"))
                .body(Vec::new())
                .unwrap();
            let response =
                super::serve_with(request, |_, _| panic!("invalid path reached storage"));
            assert_eq!(response.status(), 404, "{path}");
            assert!(String::from_utf8_lossy(response.body()).contains("kivioHtmlPreview:'error'"));
        }
        for path in [good.clone(), good.replace('/', "%2F")] {
            let request = tauri::http::Request::builder()
                .uri(format!("kivio-preview://localhost/{path}"))
                .body(Vec::new())
                .unwrap();
            let response = super::serve_with(request, |conv, attachment| {
                assert_eq!((conv, attachment), ("conv_abc-123", id));
                Some(super::inject_bootstrap("<p>stored</p>"))
            });
            assert_eq!(response.status(), 200);
            assert!(String::from_utf8_lossy(response.body()).contains("kivio-preview-theme"));
            assert!(response.headers()["Content-Security-Policy"]
                .to_str()
                .unwrap()
                .contains("sandbox allow-scripts"));
        }
    }

    #[test]
    fn preview_get_rejects_and_evicts_expired_html_without_creating_another_preview() {
        let html = "<p>private preview</p>";
        let id = super::create_html_preview(html.into()).unwrap();
        let request = || {
            tauri::http::Request::builder()
                .uri(format!("kivio-preview://localhost/{id}"))
                .body(Vec::new())
                .unwrap()
        };
        assert_eq!(
            serve(request()).body(),
            super::inject_bootstrap(html).as_bytes()
        );
        super::previews()
            .lock()
            .unwrap()
            .get_mut(&id)
            .unwrap()
            .created = std::time::Instant::now() - std::time::Duration::from_secs(3601);

        let response = serve(request());
        assert_eq!(response.status(), 404);
        assert!(String::from_utf8_lossy(response.body()).contains("kivioHtmlPreview:'error'"));
        assert!(!super::previews().lock().unwrap().contains_key(&id));
    }

    #[test]
    fn preview_serves_scripts_only_from_an_isolated_revocable_origin() {
        let html = "<script>document.body.textContent='interactive'</script>";
        let id = super::create_html_preview(html.into()).unwrap();
        let request = || {
            tauri::http::Request::builder()
                .uri(format!("kivio-preview://localhost/{id}"))
                .body(Vec::new())
                .unwrap()
        };
        let response = serve(request());
        assert_eq!(response.status(), 200);
        assert_eq!(response.body(), super::inject_bootstrap(html).as_bytes());
        let policy = response.headers()["Content-Security-Policy"]
            .to_str()
            .unwrap();
        assert!(policy.contains("sandbox allow-scripts"));
        assert!(policy.contains("connect-src https:;"));
        assert!(!policy.contains("ipc:"));
        assert!(!policy.contains("allow-same-origin"));
        super::release_html_preview(id.clone());
        assert_eq!(serve(request()).status(), 404);
    }
}
