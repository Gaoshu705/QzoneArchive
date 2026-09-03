use reqwest::header::{
    ACCEPT, ACCEPT_LANGUAGE, CACHE_CONTROL, COOKIE, ORIGIN, PRAGMA, REFERER, USER_AGENT,
};
use serde::Serialize;
use serde_json::{json, Value};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use tauri::{Manager, WebviewUrl, WebviewWindowBuilder};
use url::Url;
use std::sync::{Arc, Mutex};

use crate::qlogin::QLoginState;

const FEEDS_URL: &str = "https://mobile.qzone.qq.com/get_feeds";
const FEED_RESPONSE_ATTEMPTS: u32 = 6;
const RECYCLE_WINDOW_LABEL: &str = "qzone-recycle-auth";
const RECYCLE_TOKEN_BRIDGE_PATH: &str = "/__qza_pwd2sig__";
const RECYCLE_ALBUM_LIST_URL: &str =
    "https://user.qzone.qq.com/proxy/domain/photo.qzone.qq.com/cgi-bin/common/cgi_alist_recycle_v2";
const RECYCLE_PHOTO_LIST_URL: &str =
    "https://user.qzone.qq.com/proxy/domain/photo.qzone.qq.com/cgi-bin/common/cgi_plist_recycle_v2";
const RECOVER_PHOTO_URL: &str =
    "https://user.qzone.qq.com/proxy/domain/photo.qzone.qq.com/cgi-bin/common/cgi_recover_pic_v2";
const RECOVER_ALBUM_URL: &str =
    "https://user.qzone.qq.com/proxy/domain/photo.qzone.qq.com/cgi-bin/common/cgi_recover_album_v2";
const ALBUM_LIST_URL: &str =
    "https://h5.qzone.qq.com/proxy/domain/photo.qzone.qq.com/fcgi-bin/fcg_list_album_v3";
const CREATE_ALBUM_URL: &str =
    "https://user.qzone.qq.com/proxy/domain/photo.qzone.qq.com/cgi-bin/common/cgi_add_album_v2";

#[derive(Clone, Default)]
pub struct RecycleAuthState {
    pwd2sig: Arc<Mutex<Option<String>>>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecyclePasswordStatus {
    token: Option<String>,
    window_open: bool,
}

fn valid_pwd2sig(value: &str) -> bool {
    (5..=4096).contains(&value.len()) && !value.chars().any(char::is_control)
}

fn pwd2sig_from_url(value: &str) -> Option<String> {
    let url = Url::parse(value).ok()?;
    let from_pairs = |pairs: url::form_urlencoded::Parse<'_>| {
        pairs.into_iter().find_map(|(key, value)| {
            key.eq_ignore_ascii_case("pwd2sig")
                .then(|| value.into_owned())
        })
    };
    from_pairs(url.query_pairs())
        .or_else(|| {
            from_pairs(url::form_urlencoded::parse(
                url.fragment().unwrap_or_default().as_bytes(),
            ))
        })
        .filter(|token| valid_pwd2sig(token))
}

fn store_pwd2sig(state: &RecycleAuthState, token: String) {
    if !valid_pwd2sig(&token) {
        return;
    }
    if let Ok(mut guard) = state.pwd2sig.lock() {
        *guard = Some(token);
    }
}

fn is_qzone_url(url: &Url) -> bool {
    url.scheme() == "https"
        && url
            .host_str()
            .is_some_and(|host| host == "qzone.qq.com" || host.ends_with(".qzone.qq.com"))
}

fn set_recycle_window_cookies(
    window: &tauri::WebviewWindow,
    cookie_header: &str,
) -> Result<(), String> {
    let mut cookie_count = 0;
    for entry in cookie_header
        .split("; ")
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
    {
        let cookie = format!("{entry}; Domain=.qq.com; Path=/; Secure")
            .parse::<cookie::Cookie>()
            .map_err(|_| "解析 QQ 登录 Cookie 失败".to_string())?;
        window
            .set_cookie(cookie)
            .map_err(|error| format!("同步 QQ 登录状态到验证窗口失败：{error}"))?;
        cookie_count += 1;
    }
    if cookie_count == 0 {
        return Err("QQ 登录状态为空，请重新登录".into());
    }
    Ok(())
}

#[cfg(test)]
mod recycle_auth_tests {
    use super::{is_qzone_url, pwd2sig_from_url};
    use url::Url;

    #[test]
    fn extracts_pwd2sig_from_query_or_fragment() {
        assert_eq!(
            pwd2sig_from_url("https://user.qzone.qq.com/callback?pwd2sig=query-token"),
            Some("query-token".into())
        );
        assert_eq!(
            pwd2sig_from_url("https://user.qzone.qq.com/callback#pwd2sig=fragment%2Btoken"),
            Some("fragment+token".into())
        );
    }

    #[test]
    fn rejects_invalid_tokens_and_non_qzone_urls() {
        assert_eq!(
            pwd2sig_from_url("https://user.qzone.qq.com/callback?pwd2sig=bad%0Atoken"),
            None
        );
        assert!(is_qzone_url(
            &Url::parse("https://photo.qzone.qq.com/frame").unwrap()
        ));
        assert!(!is_qzone_url(
            &Url::parse("https://example.com/?pwd2sig=forged-token").unwrap()
        ));
        assert!(!is_qzone_url(
            &Url::parse("http://user.qzone.qq.com/?pwd2sig=insecure-token").unwrap()
        ));
    }
}

#[cfg(windows)]
fn install_recycle_request_listener(window: &tauri::WebviewWindow, state: RecycleAuthState) {
    use webview2_com::{
        Microsoft::Web::WebView2::Win32::{
            COREWEBVIEW2_WEB_RESOURCE_CONTEXT_ALL, ICoreWebView2,
        },
        take_pwstr, WebResourceRequestedEventHandler,
    };
    use windows::core::{HSTRING, PWSTR};

    let _ = window.with_webview(move |platform| {
        let controller = platform.controller();
        let Ok(webview): Result<ICoreWebView2, _> = (unsafe { controller.CoreWebView2() }) else {
            return;
        };
        unsafe {
            let _ = webview.AddWebResourceRequestedFilter(
                &HSTRING::from("*"),
                COREWEBVIEW2_WEB_RESOURCE_CONTEXT_ALL,
            );
            let handler = WebResourceRequestedEventHandler::create(Box::new(move |_, args| {
                let Some(args) = args else { return Ok(()); };
                let request = args.Request()?;
                let mut raw_uri = PWSTR::null();
                request.Uri(&mut raw_uri)?;
                let uri = take_pwstr(raw_uri);
                if uri.contains("cgi_plist_recycle_v2") {
                    if let Some(token) = pwd2sig_from_url(&uri) {
                        store_pwd2sig(&state, token);
                    }
                }
                Ok(())
            }));
            let mut registration = std::mem::zeroed();
            let _ = webview.add_WebResourceRequested(&handler, &mut registration);
        }
    });
}

fn parse_qzone_json(text: &str) -> Result<Value, String> {
    let normalized = text.trim().trim_start_matches('\u{feff}').trim();
    if normalized.is_empty() {
        return Ok(json!({ "code": 0 }));
    }
    if let Ok(value) = serde_json::from_str(normalized) {
        return Ok(value);
    }
    if let Some(callback) = normalized.rfind("frameElement.callback(") {
        if let Some(relative_start) = normalized[callback..].find('{') {
            let start = callback + relative_start;
            if let Some(end) = normalized.rfind('}') {
                if let Ok(value) = serde_json::from_str::<Value>(&normalized[start..=end]) {
                    return Ok(value);
                }
            }
        }
    }
    // QQ may wrap JSON in `_Callback(...)`, `try{...}catch{...}` or append
    // a semicolon. Extract the outermost JSON object as a final fallback.
    // The response can be an HTML shell containing setup scripts followed by
    // a callback such as `cb({...})`. Try candidate object spans from the end
    // so setup blocks like `try { document.domain = ... }` are ignored.
    let starts: Vec<usize> = normalized.match_indices('{').map(|(index, _)| index).collect();
    let mut best_with_code: Option<(usize, Value)> = None;
    let mut fallback: Option<Value> = None;
    for &start in starts.iter().rev() {
        let ends: Vec<usize> = normalized[start..].match_indices('}').map(|(index, _)| start + index + 1).collect();
        for &end in ends.iter().rev().take(80) {
            if let Ok(value) = serde_json::from_str::<Value>(&normalized[start..end]) {
                let span = end - start;
                if value.get("code").is_some()
                    && best_with_code.as_ref().map_or(true, |(best_span, _)| span > *best_span)
                {
                    best_with_code = Some((span, value));
                } else if fallback.is_none() {
                    fallback = Some(value);
                }
            }
        }
    }
    if let Some((_, value)) = best_with_code {
        return Ok(value);
    }
    if let Some(value) = fallback {
        return Ok(value);
    }
    Err(format!("解析 QQ 空间响应失败：响应片段：{}", normalized.chars().take(180).collect::<String>()))
}

fn parse_qzone_action_response(text: &str) -> Result<Value, String> {
    if text.trim().is_empty() {
        return Ok(json!({ "code": 0 }));
    }
    parse_qzone_json(text)
}

fn ensure_qzone_success(value: Value) -> Result<Value, String> {
    let code = value.get("code").and_then(|code| {
        code.as_i64()
            .or_else(|| code.as_str().and_then(|text| text.parse().ok()))
    }).ok_or("QQ 空间响应缺少 code 字段")?;
    if code == 0 {
        return Ok(value);
    }
    let message = value
        .get("message")
        .or_else(|| value.get("msg"))
        .and_then(Value::as_str)
        .unwrap_or("未知错误");
    Err(format!("QQ 空间接口返回错误 {code}：{message}"))
}

async fn recycle_get(
    state: &QLoginState,
    url: &str,
    pwd2sig: &str,
    extra: &[(&str, String)],
) -> Result<Value, String> {
    if pwd2sig.trim().is_empty() {
        return Err("独立密码验证已失效，请重新验证".into());
    }
    let auth = state.qzone_auth().await?;
    let mut query = vec![
        ("inCharset", "utf-8".into()),
        ("outCharset", "utf-8".into()),
        ("hostUin", auth.uin.clone()),
        ("notice", "0".into()),
        ("format", "json".into()),
        ("plat", "qzone".into()),
        ("source", "qzone".into()),
        ("appid", "4".into()),
        ("uin", auth.uin.clone()),
        ("output_type", "json".into()),
        ("pwd2sig", pwd2sig.into()),
        ("g_tk", auth.g_tk.to_string()),
    ];
    query.extend(extra.iter().cloned());
    let response = state
        .client()
        .get(url)
        .header(ACCEPT, "application/json, text/javascript, */*; q=0.01")
        .header(REFERER, format!("https://user.qzone.qq.com/{}/4", auth.uin))
        .header(USER_AGENT, &auth.user_agent)
        .header(COOKIE, &auth.cookie_header)
        .query(&query)
        .send()
        .await
        .map_err(|error| format!("请求相册回收站失败：{error}"))?;
    let status = response.status();
    let text = response
        .text()
        .await
        .map_err(|error| format!("读取相册回收站响应失败：{error}"))?;
    if !status.is_success() {
        return Err(format!("请求相册回收站失败：HTTP {status}"));
    }
    let parsed = parse_qzone_json(&text)?;
    ensure_qzone_success(parsed)
}

#[tauri::command]
pub async fn open_recycle_password_window(
    app: tauri::AppHandle,
    state: tauri::State<'_, QLoginState>,
    recycle_state: tauri::State<'_, RecycleAuthState>,
) -> Result<(), String> {
    let auth = state.qzone_auth().await?;
    if let Ok(mut guard) = recycle_state.pwd2sig.lock() {
        *guard = None;
    }
    let page_url = Url::parse(&format!(
        "https://user.qzone.qq.com/{}/photo/recycle",
        auth.uin
    ))
    .map_err(|error| format!("回收站地址无效：{error}"))?;
    if let Some(window) = app.get_webview_window(RECYCLE_WINDOW_LABEL) {
        set_recycle_window_cookies(&window, &auth.cookie_header)?;
        window
            .navigate(page_url)
            .map_err(|error| format!("打开独立密码验证页面失败：{error}"))?;
        window.set_focus().ok();
        return Ok(());
    }
    // The script runs inside each Qzone frame. A captured token is sent through a
    // synthetic HTTPS navigation that the native navigation handler cancels, so
    // the token never leaves this WebView and no cross-origin DOM access is needed.
    let bridge_script = r#"
      (() => {
        if (window.__qzaPwd2sigBridge) return;
        const host = location.hostname.toLowerCase();
        if (host !== 'qzone.qq.com' && !host.endsWith('.qzone.qq.com')) return;
        window.__qzaPwd2sigBridge = true;
        const prefix = '__QZA_PWD2SIG__';
        let published = false;
        const publish = (token) => {
          if (published || typeof token !== 'string' || token.length < 5 || token.length > 4096) return;
          published = true;
          try { document.title = prefix + token; } catch (_) {}
          try { location.replace('https://user.qzone.qq.com/__qza_pwd2sig__#pwd2sig=' + encodeURIComponent(token)); } catch (_) {}
        };
        const capture = (input) => {
          try {
            if (input instanceof FormData || input instanceof URLSearchParams) {
              const token = input.get('pwd2sig');
              if (token) publish(String(token));
              return;
            }
            if (input && typeof input === 'object' && input.url) capture(input.url);
            const text = typeof input === 'string' ? input : '';
            const match = text.match(/(?:[?&#]|^)pwd2sig=([^&#]+)/i);
            if (match) publish(decodeURIComponent(match[1].replace(/\+/g, ' ')));
          } catch (_) {}
        };
        try {
          const originalOpen = XMLHttpRequest.prototype.open;
          const originalSend = XMLHttpRequest.prototype.send;
          XMLHttpRequest.prototype.open = function(method, url, ...rest) { this.__qzaUrl = String(url || ''); capture(this.__qzaUrl); return originalOpen.call(this, method, url, ...rest); };
          XMLHttpRequest.prototype.send = function(body) { capture(this.__qzaUrl); capture(body); return originalSend.call(this, body); };
        } catch (_) {}
        try {
          const originalFetch = window.fetch;
          window.fetch = function(input, init) { capture(input); capture(init?.body); return originalFetch.apply(this, arguments); };
        } catch (_) {}
        const read = (w) => {
          try {
            const dc = w.QZONE && w.QZONE.dataCenter;
            const token = dc && typeof dc.get === 'function' && dc.get('pwd2sig');
            if (typeof token === 'string' && token.length > 4) return token;
          } catch (_) {}
          try {
            const seen = new WeakSet();
            const scan = (value, depth) => {
              if (!value || depth > 4 || (typeof value !== 'object' && typeof value !== 'function')) return '';
              if (seen.has(value)) return ''; seen.add(value);
              for (const key of Object.keys(value)) {
                let child; try { child = value[key]; } catch (_) { continue; }
                if (key.toLowerCase().includes('pwd2sig') && typeof child === 'string' && child.length > 4) return child;
                const found = scan(child, depth + 1); if (found) return found;
              }
              return '';
            };
            const found = scan(w.QZONE, 0) || scan(w.QPHOTO, 0);
            if (found) return found;
            for (const storage of [w.localStorage, w.sessionStorage]) {
              for (let i = 0; i < storage.length; i++) {
                const key = storage.key(i) || ''; const value = storage.getItem(key) || '';
                if (key.toLowerCase().includes('pwd2sig') && value.length > 4) return value;
              }
            }
          } catch (_) {}
          try {
            for (const entry of w.performance.getEntriesByType('resource')) {
              capture(entry.name);
              if (published) return '';
            }
            for (let i = 0; i < w.frames.length; i++) {
              const token = read(w.frames[i]);
              if (token) return token;
            }
          } catch (_) {}
          return '';
        };
        const tick = () => {
          const token = read(window);
          if (token) publish(token);
          try {
            if (window !== window.top) return;
            const roots = [document];
            for (const frame of document.querySelectorAll('iframe')) {
              if (frame.contentDocument) roots.push(frame.contentDocument);
            }
            for (const root of roots) {
              for (const node of root.querySelectorAll('*')) {
                if ((node.textContent || '').trim() === '回收站' && !sessionStorage.getItem('__qzaRecycleOpened')) {
                  sessionStorage.setItem('__qzaRecycleOpened', '1');
                  const clickable = node.closest('a,button,[role="button"]') || node;
                  clickable.click();
                  return;
                }
              }
            }
          } catch (_) {}
        };
        window.__qzaReadPwd2sig = tick;
        setInterval(tick, 800);
        setTimeout(tick, 200);
      })();
    "#;
    let builder = WebviewWindowBuilder::new(
        &app,
        RECYCLE_WINDOW_LABEL,
        // A packaged page provides WKWebView with a stable initial document while
        // native cookies are installed, avoiding the about:blank navigation stall.
        WebviewUrl::App("recycle-auth.html".into()),
    )
    .title("验证 QQ 空间独立密码")
    .inner_size(960.0, 720.0)
    .initialization_script_for_all_frames(bridge_script);
    let nav_state = recycle_state.inner().clone();
    let builder = builder.on_navigation(move |url| {
        if url.scheme() == "https"
            && url.host_str() == Some("user.qzone.qq.com")
            && url.path() == RECYCLE_TOKEN_BRIDGE_PATH
        {
            if let Some(token) = pwd2sig_from_url(url.as_str()) {
                store_pwd2sig(&nav_state, token);
            }
            return false;
        }
        if is_qzone_url(url) {
            if let Some(token) = pwd2sig_from_url(url.as_str()) {
                store_pwd2sig(&nav_state, token);
            }
        }
        true
    });
    let title_state = recycle_state.inner().clone();
    let builder = builder.on_document_title_changed(move |_window, title| {
        if let Some(token) = title
            .strip_prefix("__QZA_PWD2SIG__")
            .filter(|token| valid_pwd2sig(token))
        {
            store_pwd2sig(&title_state, token.to_owned());
        }
    });
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    let builder = builder.center();
    let window = builder
        .build()
        .map_err(|error| format!("打开独立密码验证窗口失败：{error}"))?;
    #[cfg(windows)]
    install_recycle_request_listener(&window, recycle_state.inner().clone());
    set_recycle_window_cookies(&window, &auth.cookie_header)?;
    window
        .navigate(page_url)
        .map_err(|error| format!("打开独立密码验证页面失败：{error}"))?;
    window.set_focus().ok();
    Ok(())
}

#[tauri::command]
pub async fn check_recycle_password(
    app: tauri::AppHandle,
    recycle_state: tauri::State<'_, RecycleAuthState>,
) -> Result<RecyclePasswordStatus, String> {
    if let Ok(guard) = recycle_state.pwd2sig.lock() {
        if let Some(token) = guard.clone() {
            return Ok(RecyclePasswordStatus {
                token: Some(token),
                window_open: app.get_webview_window(RECYCLE_WINDOW_LABEL).is_some(),
            });
        }
    }
    let Some(window) = app.get_webview_window(RECYCLE_WINDOW_LABEL) else {
        return Ok(RecyclePasswordStatus {
            token: None,
            window_open: false,
        });
    };
    window.eval("window.__qzaReadPwd2sig?.()").ok();
    tokio::time::sleep(std::time::Duration::from_millis(80)).await;
    // Do not call WebviewWindow::url here. WKWebView temporarily has no URL while
    // redirecting and wry 0.55.1 unwraps that nil value, crashing the macOS app.
    let token = recycle_state
        .pwd2sig
        .lock()
        .ok()
        .and_then(|guard| guard.clone())
        .or_else(|| {
            window.cookies().ok().and_then(|cookies| {
                cookies.iter().find_map(|cookie| {
                    cookie
                        .name()
                        .eq_ignore_ascii_case("pwd2sig")
                        .then(|| cookie.value().to_owned())
                        .filter(|token| valid_pwd2sig(token))
                })
            })
        });
    if let Some(token) = token.as_ref() {
        store_pwd2sig(recycle_state.inner(), token.clone());
    }
    Ok(RecyclePasswordStatus {
        token,
        window_open: true,
    })
}

#[tauri::command]
pub async fn close_recycle_password_window(app: tauri::AppHandle) -> Result<(), String> {
    if let Some(window) = app.get_webview_window(RECYCLE_WINDOW_LABEL) {
        window.close().map_err(|error| error.to_string())?;
    }
    Ok(())
}

#[tauri::command]
pub async fn list_recycle_albums(
    state: tauri::State<'_, QLoginState>,
    pwd2sig: String,
) -> Result<Value, String> {
    recycle_get(
        &state,
        RECYCLE_ALBUM_LIST_URL,
        &pwd2sig,
        &[
            ("begin", "0".into()),
            ("size", "100".into()),
            ("refresh", "true".into()),
            ("day", "0".into()),
            ("dayNum", "365".into()),
        ],
    )
    .await
}

#[tauri::command]
pub async fn list_recycle_photos(
    state: tauri::State<'_, QLoginState>,
    pwd2sig: String,
    album_id: Option<String>,
) -> Result<Value, String> {
    let mut extra = vec![
        ("begin", "0".into()),
        ("size", "18".into()),
        ("type", "0".into()),
        ("refresh", "true".into()),
        ("day", "0".into()),
        ("dayNum", "90".into()),
    ];
    if let Some(album_id) = album_id.filter(|value| !value.is_empty()) {
        extra.push(("albumId", album_id));
    }
    recycle_get(&state, RECYCLE_PHOTO_LIST_URL, &pwd2sig, &extra).await
}

#[tauri::command]
pub async fn load_recycle_photo_preview(
    state: tauri::State<'_, QLoginState>,
    image_url: String,
) -> Result<String, String> {
    let url = Url::parse(&image_url).map_err(|_| "照片缩略图地址无效".to_owned())?;
    let host = url.host_str().unwrap_or_default();
    if !(host.ends_with("qq.com") || host.ends_with("qpic.cn")) {
        return Err("照片缩略图地址不是 QQ 图片域名".into());
    }
    let auth = state.qzone_auth().await?;
    let response = state.client().get(url)
        .header(ACCEPT, "image/avif,image/webp,image/png,image/jpeg,image/*;q=0.8")
        .header(REFERER, format!("https://user.qzone.qq.com/{}/photo/recycle", auth.uin))
        .header(USER_AGENT, &auth.user_agent)
        .header(COOKIE, &auth.cookie_header)
        .send().await.map_err(|error| format!("读取照片缩略图失败：{error}"))?;
    if !response.status().is_success() { return Err(format!("读取照片缩略图失败：HTTP {}", response.status())); }
    let content_type = response.headers().get("content-type").and_then(|value| value.to_str().ok()).unwrap_or("image/jpeg").split(';').next().unwrap_or("image/jpeg").to_owned();
    let bytes = response.bytes().await.map_err(|error| format!("读取照片缩略图失败：{error}"))?;
    Ok(format!("data:{content_type};base64,{}", BASE64.encode(bytes)))
}

#[tauri::command]
pub async fn list_qzone_albums(state: tauri::State<'_, QLoginState>) -> Result<Value, String> {
    let auth = state.qzone_auth().await?;
    let request_id = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .rem_euclid(1_000_000_000)
        .to_string();
    let query = vec![
        ("g_tk", auth.g_tk.to_string()),
        ("t", request_id),
        ("hostUin", auth.uin.clone()),
        ("uin", auth.uin.clone()),
        ("appid", "4".into()),
        ("inCharset", "utf-8".into()),
        ("outCharset", "utf-8".into()),
        ("source", "qzone".into()),
        ("plat", "qzone".into()),
        ("format", "jsonp".into()),
        ("notice", "0".into()),
        ("mode", "2".into()),
        ("sortOrder", "4".into()),
        ("pageStart", "0".into()),
        ("pageNum", "1000".into()),
        ("idcNum", "4".into()),
        ("callbackFun", "shine0".into()),
    ];
    let response = state
        .client()
        .get(ALBUM_LIST_URL)
        .query(&query)
        .header(ACCEPT, "application/json, text/javascript, */*; q=0.01")
        .header(REFERER, "https://user.qzone.qq.com/")
        .header(USER_AGENT, &auth.user_agent)
        .header(COOKIE, &auth.cookie_header)
        .send()
        .await
        .map_err(|error| format!("获取相册列表失败：{error}"))?;
    let status = response.status();
    let text = response
        .text()
        .await
        .map_err(|error| format!("读取相册列表响应失败：{error}"))?;
    if !status.is_success() {
        return Err(format!("获取相册列表失败：HTTP {status}"));
    }
    ensure_qzone_success(parse_qzone_json(&text)?)
}

#[tauri::command]
pub async fn create_qzone_album(
    state: tauri::State<'_, QLoginState>,
    name: String,
) -> Result<Value, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("相册名称不能为空".into());
    }
    if name.chars().count() > 30 {
        return Err("相册名称不能超过 30 个字符".into());
    }
    let auth = state.qzone_auth().await?;
    let form = [
        ("album_type", ""),
        ("birth_time", ""),
        ("degree_type", "0"),
        ("enroll_time", ""),
        ("albumname", name),
        ("albumdesc", ""),
        ("albumclass", "100"),
        ("priv", "1"),
        ("question", ""),
        ("answer", ""),
        ("whiteList", ""),
        ("bitmap", "10000000"),
        ("uin", auth.uin.as_str()),
        ("hostUin", auth.uin.as_str()),
        ("format", "fs"),
        ("inCharset", "utf-8"),
        ("outCharset", "utf-8"),
        ("notice", "0"),
        ("callbackFun", "_Callback"),
        ("plat", "qzone"),
        ("source", "qzone"),
        ("appid", "4"),
    ];
    let response = state
        .client()
        .post(CREATE_ALBUM_URL)
        .query(&[("g_tk", auth.g_tk.to_string())])
        .header(
            REFERER,
            format!("https://user.qzone.qq.com/{}/photo", auth.uin),
        )
        .header(USER_AGENT, &auth.user_agent)
        .header(COOKIE, &auth.cookie_header)
        .header(ORIGIN, "https://user.qzone.qq.com")
        .header(
            "content-type",
            "application/x-www-form-urlencoded;charset=UTF-8",
        )
        .form(&form)
        .send()
        .await
        .map_err(|error| format!("创建相册失败：{error}"))?;
    let status = response.status();
    let text = response
        .text()
        .await
        .map_err(|error| format!("读取创建相册响应失败：{error}"))?;
    if !status.is_success() {
        return Err(format!("创建相册失败：HTTP {status}"));
    }
    ensure_qzone_success(parse_qzone_action_response(&text)?)
}

#[tauri::command]
pub async fn recover_recycle_album(
    state: tauri::State<'_, QLoginState>,
    pwd2sig: String,
    album_id: String,
) -> Result<Value, String> {
    if pwd2sig.trim().is_empty() {
        return Err("独立密码验证已失效，请重新验证".into());
    }
    if album_id.trim().is_empty() {
        return Err("缺少回收站相册 ID".into());
    }
    let auth = state.qzone_auth().await?;
    let qzreferrer = format!("https://user.qzone.qq.com/{}", auth.uin);
    let form = [
        ("inCharset", "utf-8"),
        ("outCharset", "utf-8"),
        ("hostUin", auth.uin.as_str()),
        ("notice", "0"),
        ("callbackFun", "_Callback"),
        ("format", "fs"),
        ("plat", "qzone"),
        ("source", "qzone"),
        ("appid", "4"),
        ("uin", auth.uin.as_str()),
        ("albumId", album_id.as_str()),
        ("pwd2sig", pwd2sig.as_str()),
        ("qzreferrer", qzreferrer.as_str()),
    ];
    let response = state
        .client()
        .post(RECOVER_ALBUM_URL)
        .query(&[("g_tk", auth.g_tk.to_string())])
        .header(ACCEPT, "*/*")
        .header(
            ACCEPT_LANGUAGE,
            "zh-CN,zh;q=0.9,en;q=0.8,en-GB;q=0.7,en-US;q=0.6,zh-TW;q=0.5",
        )
        .header(CACHE_CONTROL, "no-cache")
        .header(PRAGMA, "no-cache")
        .header(ORIGIN, "https://user.qzone.qq.com")
        .header(REFERER, format!("https://user.qzone.qq.com/{}", auth.uin))
        .header(USER_AGENT, &auth.user_agent)
        .header(COOKIE, &auth.cookie_header)
        .header("priority", "u=1, i")
        .header("sec-fetch-dest", "empty")
        .header("sec-fetch-mode", "cors")
        .header("sec-fetch-site", "same-origin")
        .header("content-type", "application/x-www-form-urlencoded;charset=UTF-8")
        .form(&form)
        .send()
        .await
        .map_err(|error| format!("恢复相册失败：{error}"))?;
    let status = response.status();
    let text = response.text().await.map_err(|error| format!("读取恢复相册响应失败：{error}"))?;
    if !status.is_success() {
        return Err(format!("恢复相册失败：HTTP {status}"));
    }
    let parsed = ensure_qzone_success(parse_qzone_action_response(&text)?)?;
    let data = parsed.get("data").cloned().unwrap_or_default();
    let succeeded = data.get("succ_num").and_then(Value::as_u64).unwrap_or(0);
    let failed = data.get("fail_num").and_then(Value::as_u64).unwrap_or(0);
    if succeeded != 1 || failed != 0 {
        return Err(format!("相册恢复未完成：成功 {succeeded} 个，失败 {failed} 个"));
    }
    Ok(parsed)
}

#[tauri::command]
pub async fn recover_recycle_photos(
    state: tauri::State<'_, QLoginState>,
    pwd2sig: String,
    source_album_id: String,
    target_album_id: String,
    photo_ids: Vec<String>,
) -> Result<Value, String> {
    if photo_ids.is_empty() {
        return Err("请先选择需要恢复的照片".into());
    }
    let auth = state.qzone_auth().await?;
    if source_album_id.trim().is_empty() { return Err("照片缺少回收站来源相册 ID".into()); }
    if target_album_id.trim().is_empty() { return Err("照片缺少恢复目标相册 ID".into()); }
    let pic_list = format!("{}@{}", source_album_id, photo_ids.join("_"));
    let g_tk = auth.g_tk.to_string();
    let qzreferrer = format!("https://user.qzone.qq.com/{}", auth.uin);
    let form = vec![
        ("uin", auth.uin.as_str()),
        ("hostUin", auth.uin.as_str()),
        // Destination album and recycle-bin source group are different IDs.
        ("albumId", target_album_id.as_str()),
        ("picList", pic_list.as_str()),
            ("pwd2sig", pwd2sig.as_str()),
            ("format", "fs"),
            ("inCharset", "utf-8"),
            ("outCharset", "utf-8"),
            ("notice", "0"),
            ("callbackFun", "_Callback"),
            ("plat", "qzone"),
            ("source", "qzone"),
            ("appid", "4"),
            ("qzreferrer", qzreferrer.as_str()),
    ];
    let response = state
        .client()
        .post(RECOVER_PHOTO_URL)
        .query(&[("g_tk", g_tk.as_str())])
        .header(ACCEPT, "*/*")
        .header(
            ACCEPT_LANGUAGE,
            "zh-CN,zh;q=0.9,en;q=0.8,en-GB;q=0.7,en-US;q=0.6,zh-TW;q=0.5",
        )
        .header(CACHE_CONTROL, "no-cache")
        .header(PRAGMA, "no-cache")
        .header(REFERER, format!("https://user.qzone.qq.com/{}", auth.uin))
        .header(USER_AGENT, &auth.user_agent)
        .header(COOKIE, &auth.cookie_header)
        .header(ORIGIN, "https://user.qzone.qq.com")
        .header("priority", "u=1, i")
        .header("sec-fetch-dest", "empty")
        .header("sec-fetch-mode", "cors")
        .header("sec-fetch-site", "same-origin")
        .header("content-type", "application/x-www-form-urlencoded;charset=UTF-8")
        .form(&form)
        .send()
        .await
        .map_err(|error| format!("恢复照片失败：{error}"))?;
    let status = response.status();
    let text = response.text().await.map_err(|error| error.to_string())?;
    if !status.is_success() {
        let detail = text.chars().take(300).collect::<String>();
        return Err(format!("恢复照片失败：HTTP {status} {detail}"));
    }
    let parsed = ensure_qzone_success(parse_qzone_action_response(&text)?)?;
    if let Some(succeeded) = parsed
        .get("data")
        .and_then(|data| data.get("succ_num"))
        .and_then(Value::as_u64)
    {
        let expected = photo_ids.len() as u64;
        if succeeded != expected {
            let failed = parsed
                .get("data")
                .and_then(|data| data.get("fail_num"))
                .and_then(Value::as_u64)
                .unwrap_or(expected.saturating_sub(succeeded));
            return Err(format!(
                "照片恢复未完成：请求 {expected} 张，成功 {succeeded} 张，失败 {failed} 张"
            ));
        }
    }
    Ok(parsed)
}

fn retryable_response_reason(status: reqwest::StatusCode, body: &str) -> Option<String> {
    if status == reqwest::StatusCode::TOO_MANY_REQUESTS || status.is_server_error() {
        return Some(format!("HTTP {status}"));
    }
    if !status.is_success() {
        return None;
    }
    let value = match serde_json::from_str::<Value>(body) {
        Ok(value) => value,
        Err(_) => return Some("响应不是有效 JSON".into()),
    };
    if let Some(code) = value.get("code").and_then(Value::as_i64) {
        if code != 0 {
            let message = value
                .get("message")
                .or_else(|| value.get("msg"))
                .and_then(Value::as_str)
                .unwrap_or("未知错误");
            let permanent = ["未登录", "登录失效", "权限", "封禁", "禁止访问", "p_skey"]
                .iter()
                .any(|keyword| message.contains(keyword));
            return (!permanent).then(|| format!("接口错误 {code}：{message}"));
        }
    }
    if value.get("data").is_none() {
        return Some("响应中暂时缺少 data".into());
    }
    None
}

fn feed_retry_delay(attempt: u32) -> std::time::Duration {
    std::time::Duration::from_millis(1_500 * 2_u64.pow(attempt.saturating_sub(1)))
}

fn sec_ch_ua(user_agent: &str) -> String {
    if let Some(start) = user_agent.find("Chrome/") {
        let version_start = start + 7;
        let major = user_agent[version_start..]
            .chars()
            .take_while(|c| c.is_ascii_digit())
            .collect::<String>();
        let version = if major.is_empty() { "131" } else { &major };
        format!("\"Not;A=Brand\";v=\"8\", \"Chromium\";v=\"{version}\", \"Microsoft Edge\";v=\"{version}\"")
    } else {
        "\"Not;A=Brand\";v=\"8\", \"Apple\";v=\"0\", \"Safari\";v=\"18\"".to_owned()
    }
}

fn sec_platform(user_agent: &str) -> &'static str {
    if user_agent.contains("iPhone") {
        "\"iOS\""
    } else {
        "\"Android\""
    }
}
fn response_headers(headers: &reqwest::header::HeaderMap) -> Vec<Value> {
    headers
        .iter()
        .map(|(name, value)| {
            json!({
                "name": name.as_str(),
                "value": String::from_utf8_lossy(value.as_bytes()),
            })
        })
        .collect()
}

fn log_feed_request_error(
    stage: &str,
    request_url: &str,
    query: &[(&str, String)],
    user_agent: &str,
    status: Option<reqwest::StatusCode>,
    headers: Option<&reqwest::header::HeaderMap>,
    response_body: Option<&str>,
    attempts: &[String],
    error: &str,
) {
    let parameters = query
        .iter()
        .map(|(name, value)| ((*name).to_owned(), Value::String(value.clone())))
        .collect::<serde_json::Map<String, Value>>();
    let parsed_body = response_body.and_then(|text| serde_json::from_str::<Value>(text).ok());
    let body = match (response_body, parsed_body) {
        (_, Some(value)) => Some(value),
        (Some(text), None) => Some(json!({
            "format": "raw",
            "bytesReceived": text.as_bytes().len(),
            "content": "非完整 JSON 或非 JSON 响应，原始正文见本诊断块下方"
        })),
        (None, None) => None,
    };
    let diagnostic = json!({
        "event": "qzone_archive_request_error",
        "stage": stage,
        "error": error,
        "request": {
            "method": "GET",
            "url": request_url,
            "parameters": parameters,
            "headers": {
                "Accept": "application/json",
                "Accept-Encoding": "gzip, deflate, br",
                "Accept-Language": "zh-CN,zh;q=0.9,en;q=0.8,en-GB;q=0.7,en-US;q=0.6,zh-TW;q=0.5",
                "Cache-Control": "no-cache",
                "Pragma": "no-cache",
                "Origin": "https://h5.qzone.qq.com",
                "Referer": "https://h5.qzone.qq.com/",
                "Sec-Fetch-Dest": "empty",
                "Sec-Fetch-Mode": "cors",
                "Sec-Fetch-Site": "same-site",
                "Sec-Ch-Ua-Mobile": "?1",
                "User-Agent": user_agent,
                "Cookie": "[已隐藏：登录凭证不会写入控制台]"
            }
        },
        "response": {
            "status": status.map(|value| value.as_u16()),
            "statusText": status.and_then(|value| value.canonical_reason()),
            "headers": headers.map(response_headers),
            "body": body,
        },
        "transportAttempts": attempts,
    });
    let formatted =
        serde_json::to_string_pretty(&diagnostic).unwrap_or_else(|_| diagnostic.to_string());
    eprintln!("\n================ QZONE ARCHIVE REQUEST ERROR ================\n{formatted}");
    if let Some(text) = response_body {
        eprintln!("---------------- RAW RESPONSE BODY ----------------\n{text}\n---------------- END RAW RESPONSE BODY ----------------");
    }
    eprintln!("================ END QZONE ARCHIVE REQUEST ERROR ================\n");
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FeedPage {
    pub(crate) feeds: Vec<Value>,
    pub(crate) attach_info: Option<String>,
    pub(crate) has_more: bool,
}

fn parse_feed_page(value: Value) -> Result<FeedPage, String> {
    if let Some(code) = value.get("code").and_then(Value::as_i64) {
        if code != 0 {
            let message = value
                .get("message")
                .or_else(|| value.get("msg"))
                .and_then(Value::as_str)
                .unwrap_or("未知错误");
            return Err(format!("QQ 空间动态接口返回错误 {code}：{message}"));
        }
    }
    let data = value.get("data").ok_or("动态响应中缺少 data")?;
    let feeds = data
        .get("vFeeds")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let attach_info = data
        .get("attachinfo")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_owned);
    let server_has_more = data.get("hasmore").and_then(Value::as_i64).unwrap_or(0) != 0;
    let has_more = server_has_more && !feeds.is_empty() && attach_info.is_some();
    Ok(FeedPage {
        feeds,
        attach_info,
        has_more,
    })
}

pub(crate) async fn fetch_feeds(
    state: &QLoginState,
    refresh_type: &str,
    attach_info: Option<&str>,
) -> Result<FeedPage, String> {
    fetch_feeds_with_attempts(state, refresh_type, attach_info, FEED_RESPONSE_ATTEMPTS).await
}

pub(crate) async fn fetch_feeds_once(
    state: &QLoginState,
    refresh_type: &str,
    attach_info: Option<&str>,
) -> Result<FeedPage, String> {
    fetch_feeds_with_attempts(state, refresh_type, attach_info, 1).await
}

pub(crate) fn feed_error_can_skip(error: &str) -> bool {
    error.contains("HTTP 5")
        || error.starts_with("解析空间动态失败：")
        || error.starts_with("QQ 空间动态接口返回错误")
}

async fn fetch_feeds_with_attempts(
    state: &QLoginState,
    refresh_type: &str,
    attach_info: Option<&str>,
    attempts: u32,
) -> Result<FeedPage, String> {
    let auth = state.qzone_auth().await?;
    let mut query = vec![
        ("g_tk", auth.g_tk.to_string()),
        ("res_type", "1".into()),
        ("refresh_type", refresh_type.into()),
        ("format", "json".into()),
    ];
    if let Some(attach_info) = attach_info {
        if attach_info.trim().is_empty() {
            let error = "分页游标不能为空";
            log_feed_request_error(
                "validate_request",
                FEEDS_URL,
                &query,
                &auth.user_agent,
                None,
                None,
                None,
                &[],
                error,
            );
            return Err(error.into());
        }
        query.push(("res_attach", attach_info.to_owned()));
    }
    let request_url = reqwest::Url::parse_with_params(FEEDS_URL, &query)
        .map(|url| url.to_string())
        .unwrap_or_else(|_| FEEDS_URL.to_owned());
    let client = state.client();
    let mut response = None;
    let mut last_error = None;
    let mut transport_attempts = Vec::new();
    let mut failed_response_status = None;
    let mut failed_response_headers = None;
    let mut failed_response_body = None;
    let mut last_attempt_logged = false;
    let attempts = attempts.max(1);
    for attempt in 1..=attempts {
        match client
            .get(FEEDS_URL)
            .header(ACCEPT, "application/json")
            .header(ACCEPT_LANGUAGE, "zh-CN,zh;q=0.9,en;q=0.8,en-GB;q=0.7,en-US;q=0.6,zh-TW;q=0.5")
            .header(CACHE_CONTROL, "no-cache")
            .header(PRAGMA, "no-cache")
            .header(ORIGIN, "https://h5.qzone.qq.com")
            .header(REFERER, "https://h5.qzone.qq.com/")
            .header(USER_AGENT, &auth.user_agent)
            .header(COOKIE, &auth.cookie_header)
            .header("Sec-Fetch-Dest", "empty")
            .header("Sec-Fetch-Mode", "cors")
            .header("Sec-Fetch-Site", "same-site")
            .header("Sec-Ch-Ua", sec_ch_ua(&auth.user_agent))
            .header("Sec-Ch-Ua-Mobile", "?1")
            .header("Sec-Ch-Ua-Platform", sec_platform(&auth.user_agent))
            .query(&query)
            .send()
            .await
        {
            Ok(mut value) => {
                let status = value.status();
                let headers = value.headers().clone();
                let mut bytes = Vec::new();
                let mut read_error = None;
                loop {
                    match value.chunk().await {
                        Ok(Some(chunk)) => bytes.extend_from_slice(&chunk),
                        Ok(None) => break,
                        Err(reason) => {
                            read_error = Some(reason);
                            break;
                        }
                    }
                }
                let body = String::from_utf8_lossy(&bytes).into_owned();
                if let Some(reason) = read_error {
                    let detail = format!(
                        "响应体读取失败（第 {attempt}/{attempts} 次，已接收 {} 字节）：{reason:#}",
                        bytes.len()
                    );
                    transport_attempts.push(detail.clone());
                    last_error = Some(detail);
                    log_feed_request_error(
                        &format!("read_response_attempt_{attempt}"),
                        &request_url,
                        &query,
                        &auth.user_agent,
                        Some(status),
                        Some(&headers),
                        Some(&body),
                        &transport_attempts,
                        transport_attempts.last().expect("刚写入的重试错误应当存在"),
                    );
                    failed_response_status = Some(status);
                    failed_response_headers = Some(headers);
                    failed_response_body = Some(body);
                    last_attempt_logged = true;
                    if attempt < attempts {
                        tokio::time::sleep(feed_retry_delay(attempt)).await;
                    }
                } else {
                    if let Some(reason) = retryable_response_reason(status, &body) {
                        let detail = format!("{reason}（第 {attempt}/{attempts} 次）");
                        transport_attempts.push(detail.clone());
                        log_feed_request_error(
                            &format!("retryable_response_attempt_{attempt}"),
                            &request_url,
                            &query,
                            &auth.user_agent,
                            Some(status),
                            Some(&headers),
                            Some(&body),
                            &transport_attempts,
                            &detail,
                        );
                        if attempt < attempts {
                            tokio::time::sleep(feed_retry_delay(attempt)).await;
                            continue;
                        }
                    }
                    response = Some((status, headers, body));
                    break;
                }
            }
            Err(error) => {
                let kind = if error.is_timeout() {
                    "请求超时"
                } else if error.is_connect() {
                    "连接失败"
                } else {
                    "传输失败"
                };
                let detail = format!("{kind}（第 {attempt}/{attempts} 次）：{error:#}");
                transport_attempts.push(detail.clone());
                last_error = Some(detail);
                last_attempt_logged = false;
                if attempt < attempts {
                    tokio::time::sleep(feed_retry_delay(attempt)).await;
                }
            }
        }
    }
    let Some((status, headers, body)) = response else {
        let error = format!(
            "获取空间动态失败：{}",
            last_error.unwrap_or_else(|| "未知网络错误".into())
        );
        let stage = if failed_response_status.is_some() {
            "read_response"
        } else {
            "transport"
        };
        if !last_attempt_logged {
            log_feed_request_error(
                stage,
                &request_url,
                &query,
                &auth.user_agent,
                failed_response_status,
                failed_response_headers.as_ref(),
                failed_response_body.as_deref(),
                &transport_attempts,
                &error,
            );
        }
        return Err(error);
    };
    if !status.is_success() {
        let error = format!("获取空间动态失败：HTTP {status}");
        log_feed_request_error(
            "http_status",
            &request_url,
            &query,
            &auth.user_agent,
            Some(status),
            Some(&headers),
            Some(&body),
            &transport_attempts,
            &error,
        );
        return Err(error);
    }
    let value = match serde_json::from_str::<Value>(&body) {
        Ok(value) => value,
        Err(reason) => {
            let error = format!("解析空间动态失败：{reason}");
            log_feed_request_error(
                "parse_json",
                &request_url,
                &query,
                &auth.user_agent,
                Some(status),
                Some(&headers),
                Some(&body),
                &transport_attempts,
                &error,
            );
            return Err(error);
        }
    };
    match parse_feed_page(value) {
        Ok(page) => Ok(page),
        Err(error) => {
            log_feed_request_error(
                "parse_api_response",
                &request_url,
                &query,
                &auth.user_agent,
                Some(status),
                Some(&headers),
                Some(&body),
                &transport_attempts,
                &error,
            );
            Err(error)
        }
    }
}

#[tauri::command]
pub async fn fetch_first_feeds(state: tauri::State<'_, QLoginState>) -> Result<FeedPage, String> {
    fetch_feeds(&state, "1", None).await
}

#[tauri::command]
pub async fn fetch_more_feeds(
    state: tauri::State<'_, QLoginState>,
    attach_info: String,
) -> Result<FeedPage, String> {
    fetch_feeds(&state, "2", Some(&attach_info)).await
}

#[cfg(test)]
mod tests {
    use super::{ensure_qzone_success, feed_error_can_skip, parse_feed_page, parse_qzone_json, retryable_response_reason, FEEDS_URL};
    use reqwest::StatusCode;
    use serde_json::json;

    #[test]
    fn keeps_first_page_feeds_and_cursor() {
        let page = parse_feed_page(json!({
            "code": 0,
            "data": { "attachinfo": "next-cursor", "hasmore": 1, "vFeeds": [{"id": 1}] }
        }))
        .unwrap();
        assert_eq!(page.feeds.len(), 1);
        assert_eq!(page.attach_info.as_deref(), Some("next-cursor"));
        assert!(page.has_more);
    }

    #[test]
    fn empty_page_finishes_pagination() {
        let page = parse_feed_page(json!({"code": 0, "data": {"vFeeds": []}})).unwrap();
        assert!(page.feeds.is_empty());
        assert!(!page.has_more);
    }

    #[test]
    fn cursor_remains_server_encoded_until_query_serialization() {
        let cursor = "att=back%5Fserver%5Finfo%3Doffset%253D6&tl=123";
        let encoded =
            reqwest::Url::parse_with_params(FEEDS_URL, &[("res_attach", cursor)]).unwrap();
        assert!(encoded
            .as_str()
            .contains("back%255Fserver%255Finfo%253Doffset%25253D6%26tl%3D123"));
        assert_eq!(
            encoded
                .query_pairs()
                .find(|(key, _)| key == "res_attach")
                .unwrap()
                .1,
            cursor
        );
    }

    #[test]
    fn retries_rate_limits_and_temporary_api_errors() {
        assert!(retryable_response_reason(StatusCode::TOO_MANY_REQUESTS, "busy").is_some());
        assert!(retryable_response_reason(
            StatusCode::OK,
            r#"{"code":-1,"message":"系统繁忙，请稍后再试"}"#,
        )
        .is_some());
    }

    #[test]
    fn does_not_retry_expired_login_response() {
        assert!(retryable_response_reason(
            StatusCode::OK,
            r#"{"code":-3000,"message":"登录失效，请重新登录"}"#,
        )
        .is_none());
    }

    #[test]
    fn parses_qzone_callback_response() {
        let value = parse_qzone_json(r#"<script>frameElement.callback({"code":0,"data":{"succ_num":1}});</script>"#).unwrap();
        assert_eq!(value["code"], 0);
        assert_eq!(value["data"]["succ_num"], 1);
        assert!(ensure_qzone_success(value).is_ok());
    }

    #[test]
    fn parses_outer_object_from_nested_jsonp_response() {
        let value = parse_qzone_json(
            r#"shine0({"code":0,"data":{"albumList":[{"id":"album-1","name":"恢复相册"}]}});"#,
        )
        .unwrap();
        assert_eq!(value["code"], 0);
        assert_eq!(value["data"]["albumList"][0]["id"], "album-1");
    }

    #[test]
    fn rejects_response_without_code() {
        let value = serde_json::json!({"data": {"succ_num": 1}});
        assert!(ensure_qzone_success(value).is_err());
    }

    #[test]
    fn only_skips_page_specific_server_or_response_errors() {
        assert!(feed_error_can_skip(
            "获取空间动态失败：HTTP 500 Internal Server Error"
        ));
        assert!(feed_error_can_skip("解析空间动态失败：expected value"));
        assert!(!feed_error_can_skip(
            "获取空间动态失败：HTTP 429 Too Many Requests"
        ));
        assert!(!feed_error_can_skip("尚未登录 QQ 空间"));
    }
}
