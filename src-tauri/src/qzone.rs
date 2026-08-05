use reqwest::header::{
    ACCEPT, ACCEPT_LANGUAGE, CACHE_CONTROL, COOKIE, ORIGIN, PRAGMA, REFERER, USER_AGENT,
};
use serde::Serialize;
use serde_json::{json, Value};

use crate::qlogin::QLoginState;

const FEEDS_URL: &str = "https://mobile.qzone.qq.com/get_feeds";
const FEED_RESPONSE_ATTEMPTS: u32 = 3;

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

fn sec_ch_ua(user_agent: &str) -> &'static str {
    if user_agent.contains("Chrome") {
        "\"Not(A:Brand\";v=\"99\", \"Google Chrome\";v=\"131\", \"Chromium\";v=\"131\""
    } else {
        "\"Not(A:Brand\";v=\"99\", \"Apple\";v=\"0\", \"Safari\";v=\"18\""
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

#[allow(clippy::too_many_arguments)]
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
            "bytesReceived": text.len(),
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
                "Accept-Language": "zh-CN,zh;q=0.9,en;q=0.8",
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
            .header(ACCEPT_LANGUAGE, "zh-CN,zh;q=0.9,en;q=0.8")
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
    use super::{feed_error_can_skip, parse_feed_page, retryable_response_reason, FEEDS_URL};
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
