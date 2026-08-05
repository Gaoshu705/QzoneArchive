use std::{
    collections::{HashMap, HashSet},
    sync::{
        atomic::{AtomicU64, AtomicUsize, Ordering},
        Arc,
    },
    time::{SystemTime, UNIX_EPOCH},
};

use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use encoding_rs::GB18030;
use regex::Regex;
use reqwest::{
    cookie::{CookieStore, Jar},
    header::{CONTENT_TYPE, LOCATION, REFERER, USER_AGENT},
    redirect::Policy,
    Client,
};
use serde::{Deserialize, Serialize};
use tauri::{Manager, WebviewUrl, WebviewWindowBuilder};
use tokio::sync::Mutex;
use url::Url;

const APP_ID: &str = "549000929";
const DAID: &str = "5";
const XLOGIN_URL: &str = "https://xui.ptlogin2.qq.com/cgi-bin/xlogin";
const S_URL: &str = "https://h5.qzone.qq.com/mqzone/index";
const PROXY_URL: &str = "";
const MOBILE_USER_AGENTS: &[&str] = &[
    "Mozilla/5.0 (iPhone; CPU iPhone OS 18_5 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.5 Mobile/15E148 Safari/604.1",
    "Mozilla/5.0 (iPhone; CPU iPhone OS 17_6 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.6 Mobile/15E148 Safari/604.1",
    "Mozilla/5.0 (Linux; Android 15; Pixel 8 Build/AP3A.241105.007) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Mobile Safari/537.36",
    "Mozilla/5.0 (Linux; Android 14; SM-S9280 Build/UP1A.231005.007) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Mobile Safari/537.36",
    "Mozilla/5.0 (Linux; Android 14; 23127PN0CC Build/UKQ1.231003.002) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Mobile Safari/537.36",
    "Mozilla/5.0 (Linux; Android 14; V2309A Build/UP1A.231005.007) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/129.0.0.0 Mobile Safari/537.36",
];
static USER_AGENT_SEQUENCE: AtomicUsize = AtomicUsize::new(0);

const QR_SESSION_TTL_MS: u128 = 5 * 60 * 1000;
const MAX_QR_SESSIONS: usize = 16;
const MAX_POLLS: u32 = 180;
const MAX_REFRESHES: u32 = 3;
const WEB_LOGIN_URL: &str = "https://i.qq.com";
const WEB_LOGIN_WINDOW_LABEL: &str = "qq-web-login";
const WEB_LOGIN_OPEN_ERROR: &str = "无法打开 QQ 登录窗口，请稍后重试";
const WEB_LOGIN_CLOSE_POLL_ATTEMPTS: usize = 20;
const WEB_LOGIN_CLOSE_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(25);
const MAX_PROFILE_BYTES: usize = 256 * 1024;
const MAX_AVATAR_BYTES: usize = 2 * 1024 * 1024;
const REQUIRED_WEB_COOKIES: &[&str] = &["uin", "p_uin", "p_skey", "skey", "pt4_token"];

#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub(crate) struct QrSessionId(String);

impl QrSessionId {
    fn generate() -> Self {
        Self(uuid::Uuid::new_v4().simple().to_string())
    }
}

impl From<&str> for QrSessionId {
    fn from(value: &str) -> Self {
        Self(value.to_owned())
    }
}

#[derive(Clone, Debug)]
struct QrLoginSession {
    created_at: u128,
    refresh_count: u32,
    poll_count: u32,
    generation: u64,
    cancel_epoch: u64,
    jar: Arc<Jar>,
    client: Client,
    user_agent: String,
    appid: &'static str,
    u1: &'static str,
    ptqrtoken: i64,
    login_sig: String,
    uin: Option<String>,
    g_tk: Option<i64>,
}

impl QrLoginSession {
    fn new(
        jar: Arc<Jar>,
        client: Client,
        user_agent: String,
        generation: u64,
        cancel_epoch: u64,
    ) -> Self {
        Self {
            created_at: unix_millis(),
            refresh_count: 0,
            poll_count: 0,
            generation,
            cancel_epoch,
            jar,
            client,
            user_agent,
            appid: APP_ID,
            u1: S_URL,
            ptqrtoken: 0,
            login_sig: String::new(),
            uin: None,
            g_tk: None,
        }
    }

    fn clear_secrets(&mut self) {
        self.jar = Arc::new(Jar::default());
        self.user_agent.clear();
        self.ptqrtoken = 0;
        self.login_sig.clear();
        self.uin = None;
        self.g_tk = None;
    }
}

#[derive(Default)]
struct LoginSession {
    cookies: HashMap<String, String>,
    uin: Option<String>,
    g_tk: Option<i64>,
    user_agent: String,
    web_attempt: Option<u64>,
}

pub struct QLoginState {
    client: Client,
    session: Mutex<Option<LoginSession>>,
    sessions: Mutex<HashMap<QrSessionId, QrLoginSession>>,
    last_user_agent: Mutex<Option<String>>,
    generation: AtomicU64,
    cancel_epochs: Mutex<HashMap<QrSessionId, u64>>,
    in_flight: Mutex<HashSet<QrSessionId>>,
    lifecycle_commit: Mutex<()>,
    web_open: Mutex<()>,
    web_generation: AtomicU64,
    active_web_attempt: AtomicU64,
}

pub(crate) struct QzoneAuth {
    pub uin: String,
    pub g_tk: i64,
    pub cookie_header: String,
    pub user_agent: String,
}

impl QLoginState {
    pub fn new() -> Self {
        let client = Client::builder()
            .redirect(Policy::none())
            .connect_timeout(std::time::Duration::from_secs(15))
            .timeout(std::time::Duration::from_secs(35))
            .build()
            .expect("failed to build QQ login HTTP client");
        Self {
            client,
            session: Mutex::new(None),
            sessions: Mutex::new(HashMap::new()),
            last_user_agent: Mutex::new(None),
            generation: AtomicU64::new(0),
            cancel_epochs: Mutex::new(HashMap::new()),
            in_flight: Mutex::new(HashSet::new()),
            lifecycle_commit: Mutex::new(()),
            web_open: Mutex::new(()),
            web_generation: AtomicU64::new(0),
            active_web_attempt: AtomicU64::new(0),
        }
    }

    pub(crate) fn client(&self) -> Client {
        self.client.clone()
    }

    pub(crate) async fn qzone_auth(&self) -> Result<QzoneAuth, String> {
        let guard = self.session.lock().await;
        let session = guard.as_ref().ok_or("尚未登录 QQ 空间")?;
        let g_tk = session.g_tk.ok_or("登录会话缺少 g_tk")?;
        if session
            .cookies
            .get("p_skey")
            .is_none_or(|value| value.trim().is_empty())
        {
            return Err("登录会话缺少有效的 p_skey".into());
        }
        let uin = session.uin.clone().ok_or("登录会话缺少 uin")?;
        Ok(QzoneAuth {
            uin,
            g_tk,
            cookie_header: cookie_header(&session.cookies),
            user_agent: session.user_agent.clone(),
        })
    }

    async fn next_mobile_user_agent(&self) -> String {
        let mut previous = self.last_user_agent.lock().await;
        let selected = select_mobile_user_agent(previous.as_deref());
        *previous = Some(selected.clone());
        selected
    }

    pub(crate) async fn clear_session(&self) {
        let _commit = self.lifecycle_commit.lock().await;
        self.generation.fetch_add(1, Ordering::SeqCst);
        self.web_generation.fetch_add(1, Ordering::SeqCst);
        self.active_web_attempt.store(0, Ordering::SeqCst);
        *self.session.lock().await = None;
        self.sessions.lock().await.clear();
        self.cancel_epochs.lock().await.clear();
        self.in_flight.lock().await.clear();
    }
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QrLoginStart {
    session_id: QrSessionId,
    qr_image: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QzoneLoginUser {
    uin: String,
    nickname: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    avatar_image: Option<String>,
}

#[derive(Deserialize)]
struct UserInfoResponse {
    code: i64,
    data: Option<serde_json::Map<String, serde_json::Value>>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LoginStatus {
    #[serde(skip_serializing_if = "Option::is_none")]
    session_id: Option<QrSessionId>,
    status: &'static str,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    qr_image: Option<String>,
}

fn unix_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

fn select_mobile_user_agent(previous: Option<&str>) -> String {
    let sequence = USER_AGENT_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let seed = unix_millis() as usize ^ sequence.wrapping_mul(0x9E37_79B1);
    let mut index = seed % MOBILE_USER_AGENTS.len();
    if previous.is_some_and(|value| value == MOBILE_USER_AGENTS[index]) {
        index = (index + 1) % MOBILE_USER_AGENTS.len();
    }
    MOBILE_USER_AGENTS[index].to_owned()
}

fn account_user_agent(uin: &str) -> String {
    let hash = uin
        .bytes()
        .fold(0u32, |acc, b| acc.wrapping_mul(31).wrapping_add(b as u32));
    MOBILE_USER_AGENTS[hash as usize % MOBILE_USER_AGENTS.len()].to_owned()
}

fn ptqr_token(qrsig: &str) -> i64 {
    let mut value: u32 = 0;
    for character in qrsig.chars() {
        value = value
            .wrapping_add(value.wrapping_shl(5))
            .wrapping_add(character as u32);
    }
    (value & 0x7fff_ffff) as i64
}

fn bkn(p_skey: &str) -> i64 {
    let mut value: u32 = 5381;
    for character in p_skey.chars() {
        value = value
            .wrapping_add(value.wrapping_shl(5))
            .wrapping_add(character as u32);
    }
    (value & 0x7fff_ffff) as i64
}

fn cookie_header(cookies: &HashMap<String, String>) -> String {
    cookies
        .iter()
        .map(|(key, value)| format!("{key}={value}"))
        .collect::<Vec<_>>()
        .join("; ")
}

fn normalized_uin(value: &str) -> Option<String> {
    let value = value.strip_prefix('o').unwrap_or(value);
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let normalized = value.trim_start_matches('0');
    (!normalized.is_empty()).then(|| normalized.to_owned())
}

fn allowed_qq_cookie_domain(domain: Option<&str>) -> bool {
    domain.is_some_and(|domain| {
        let normalized = domain.trim_start_matches('.').to_ascii_lowercase();
        matches!(normalized.as_str(), "qq.com" | "i.qq.com" | "qzone.qq.com")
    })
}

fn random_hex(len: usize) -> String {
    let mut result = String::new();
    while result.len() < len {
        result.push_str(&uuid::Uuid::new_v4().simple().to_string());
    }
    result.truncate(len);
    result
}

fn parse_poll_callback(text: &str) -> Result<(String, String), String> {
    let regex =
        Regex::new(r"^ptuiCB\('([^']*)','([^']*)','([^']*)','([^']*)','([^']*)','([^']*)'\);?\s*$")
            .map_err(|_| "登录状态解析失败")?;
    let captures = regex
        .captures(text.trim())
        .ok_or("QQ 登录返回了无法识别的状态")?;
    Ok((captures[1].to_owned(), captures[3].to_owned()))
}

fn validate_login_hop(url: &Url) -> bool {
    if url.scheme() != "https" {
        return false;
    }
    match url.host_str() {
        Some("ptlogin2.qzone.qq.com" | "ssl.ptlogin2.qq.com" | "ptlogin2.qq.com") => true,
        Some("qzone.qq.com" | "h5.qzone.qq.com") => {
            matches!(url.path(), "/" | "/mqzone/index" | "/index")
        }
        _ => false,
    }
}

fn validate_success_url(value: &str) -> Result<Url, String> {
    let url = Url::parse(value).map_err(|_| "登录跳转地址无效")?;
    let allowed = matches!(
        url.host_str(),
        Some("ptlogin2.qzone.qq.com" | "ssl.ptlogin2.qq.com" | "ptlogin2.qq.com")
    );
    if url.scheme() != "https" || !allowed || url.path() != "/check_sig" {
        return Err("登录跳转地址不受信任".into());
    }
    let has = |name: &str| {
        url.query_pairs()
            .any(|(key, value)| key == name && !value.is_empty())
    };
    if !has("uin") || !has("ptsigx") {
        return Err("登录跳转信息不完整".into());
    }
    Ok(url)
}

fn jar_cookies(jar: &Jar, url: &Url) -> HashMap<String, String> {
    jar.cookies(url)
        .and_then(|value| value.to_str().ok().map(str::to_owned))
        .unwrap_or_default()
        .split(';')
        .filter_map(|item| item.trim().split_once('='))
        .map(|(name, value)| (name.to_owned(), value.to_owned()))
        .collect()
}

async fn initialize_qr(
    user_agent: String,
    generation: u64,
    cancel_epoch: u64,
) -> Result<(QrLoginSession, String), String> {
    let jar = Arc::new(Jar::default());
    let client = Client::builder()
        .cookie_provider(jar.clone())
        .redirect(Policy::none())
        .connect_timeout(std::time::Duration::from_secs(15))
        .timeout(std::time::Duration::from_secs(35))
        .build()
        .map_err(|_| "无法初始化登录客户端")?;
    let xlogin = client
        .get(XLOGIN_URL)
        .header(USER_AGENT, &user_agent)
        .query(&[
            ("hide_title_bar", "1"),
            ("style", "22"),
            ("daid", DAID),
            ("low_login", "0"),
            ("qlogin_auto_login", "1"),
            ("no_verifyimg", "1"),
            ("link_target", "blank"),
            ("appid", APP_ID),
            ("target", "self"),
            ("s_url", S_URL),
            ("proxy_url", PROXY_URL),
            ("pt_no_auth", "1"),
        ])
        .send()
        .await
        .map_err(|_| "初始化 QQ 登录失败")?;
    if !xlogin.status().is_success() {
        return Err("初始化 QQ 登录失败".into());
    }
    let xurl = Url::parse(XLOGIN_URL).map_err(|_| "登录配置无效")?;
    let login_sig = jar_cookies(&jar, &xurl)
        .remove("pt_login_sig")
        .filter(|v| !v.is_empty())
        .ok_or("QQ 登录初始化响应不完整")?;
    let mut qr_url =
        Url::parse("https://ssl.ptlogin2.qq.com/ptqrshow").map_err(|_| "登录配置无效")?;
    qr_url.query_pairs_mut().extend_pairs([
        ("appid", APP_ID),
        ("e", "2"),
        ("l", "M"),
        ("s", "3"),
        ("d", "72"),
        ("v", "4"),
        ("t", &unix_millis().to_string()),
        ("daid", DAID),
        ("pt_3rd_aid", "0"),
        ("u1", S_URL),
    ]);
    let response = client
        .get(qr_url.clone())
        .header(USER_AGENT, &user_agent)
        .send()
        .await
        .map_err(|_| "获取登录二维码失败")?;
    if !response.status().is_success() {
        return Err("获取登录二维码失败".into());
    }
    let qrsig = jar_cookies(&jar, &qr_url)
        .remove("qrsig")
        .filter(|v| !v.is_empty())
        .ok_or("二维码响应不完整")?;
    let image = response.bytes().await.map_err(|_| "读取登录二维码失败")?;
    let qq = Url::parse("https://qq.com/").map_err(|_| "登录配置无效")?;
    jar.add_cookie_str(
        &format!(
            "_qimei_fingerprint={}; Domain=.qq.com; Path=/; Secure",
            random_hex(32)
        ),
        &qq,
    );
    jar.add_cookie_str(
        &format!(
            "_qimei_uuid42={}; Domain=.qq.com; Path=/; Secure",
            random_hex(42)
        ),
        &qq,
    );
    let mut session = QrLoginSession::new(jar, client, user_agent, generation, cancel_epoch);
    session.ptqrtoken = ptqr_token(&qrsig);
    session.login_sig = login_sig;
    Ok((
        session,
        format!("data:image/png;base64,{}", BASE64.encode(image)),
    ))
}

#[tauri::command]
pub async fn start_qr_login(state: tauri::State<'_, QLoginState>) -> Result<QrLoginStart, String> {
    {
        let mut sessions = state.sessions.lock().await;
        sessions.retain(|_, v| unix_millis().saturating_sub(v.created_at) <= QR_SESSION_TTL_MS);
        if sessions.len() >= MAX_QR_SESSIONS {
            return Err("二维码登录会话过多，请稍后重试".into());
        }
    }
    let id = QrSessionId::generate();
    let generation = state.generation.load(Ordering::SeqCst);
    let epoch = *state
        .cancel_epochs
        .lock()
        .await
        .entry(id.clone())
        .or_insert(0);
    let ua = state.next_mobile_user_agent().await;
    let (session, image) = initialize_qr(ua, generation, epoch).await?;
    let mut sessions = state.sessions.lock().await;
    sessions.retain(|_, value| unix_millis().saturating_sub(value.created_at) <= QR_SESSION_TTL_MS);
    if sessions.len() >= MAX_QR_SESSIONS {
        return Err("二维码登录会话过多，请稍后重试".into());
    }
    sessions.insert(id.clone(), session);
    Ok(QrLoginStart {
        session_id: id,
        qr_image: image,
    })
}

fn login_status(
    id: &QrSessionId,
    status: &'static str,
    message: &str,
    image: Option<String>,
) -> LoginStatus {
    LoginStatus {
        session_id: Some(id.clone()),
        status,
        message: message.into(),
        qr_image: image,
    }
}
async fn cancelled(state: &QLoginState, id: &QrSessionId, s: &QrLoginSession) -> bool {
    s.generation != state.generation.load(Ordering::SeqCst)
        || state
            .cancel_epochs
            .lock()
            .await
            .get(id)
            .copied()
            .unwrap_or(0)
            != s.cancel_epoch
}
async fn restore(state: &QLoginState, id: QrSessionId, s: QrLoginSession) {
    if !cancelled(state, &id, &s).await {
        state.sessions.lock().await.insert(id, s);
    }
}

#[tauri::command]
pub async fn poll_qr_login(
    state: tauri::State<'_, QLoginState>,
    id: QrSessionId,
) -> Result<LoginStatus, String> {
    {
        let mut f = state.in_flight.lock().await;
        if !f.insert(id.clone()) {
            return Ok(login_status(&id, "error", "二维码登录正在处理中", None));
        }
    }
    let r = poll_inner(&state, id.clone()).await;
    state.in_flight.lock().await.remove(&id);
    if !state.sessions.lock().await.contains_key(&id) {
        state.cancel_epochs.lock().await.remove(&id);
    }
    r
}
async fn poll_inner(state: &QLoginState, id: QrSessionId) -> Result<LoginStatus, String> {
    let mut s = state
        .sessions
        .lock()
        .await
        .remove(&id)
        .ok_or("二维码登录会话不存在或已失效")?;
    if cancelled(state, &id, &s).await {
        s.clear_secrets();
        return Ok(login_status(&id, "cancelled", "二维码登录已取消", None));
    }
    if unix_millis().saturating_sub(s.created_at) > QR_SESSION_TTL_MS || s.poll_count >= MAX_POLLS {
        s.clear_secrets();
        return Ok(login_status(&id, "timedOut", "二维码登录已超时", None));
    }
    s.poll_count += 1;
    let response = match s
        .client
        .get("https://ssl.ptlogin2.qq.com/ptqrlogin")
        .header(USER_AGENT, &s.user_agent)
        .query(&[
            ("u1", s.u1),
            ("ptqrtoken", &s.ptqrtoken.to_string()),
            ("ptredirect", "0"),
            ("action", &format!("0-0-{}", unix_millis())),
            ("login_sig", &s.login_sig),
            ("aid", s.appid),
            ("daid", DAID),
        ])
        .send()
        .await
    {
        Ok(v) => v,
        Err(_) => {
            restore(state, id.clone(), s).await;
            return Ok(login_status(
                &id,
                "error",
                "检查扫码状态失败，请稍后重试",
                None,
            ));
        }
    };
    if cancelled(state, &id, &s).await {
        s.clear_secrets();
        return Ok(login_status(&id, "cancelled", "二维码登录已取消", None));
    }
    let text = match response.text().await {
        Ok(v) => v,
        Err(_) => {
            restore(state, id.clone(), s).await;
            return Ok(login_status(
                &id,
                "error",
                "读取扫码状态失败，请稍后重试",
                None,
            ));
        }
    };
    let (code, url) = match parse_poll_callback(&text) {
        Ok(v) => v,
        Err(_) => {
            s.clear_secrets();
            return Ok(login_status(
                &id,
                "error",
                "QQ 登录返回了无法识别的状态",
                None,
            ));
        }
    };
    match code.as_str() {
        "66" => {
            restore(state, id.clone(), s).await;
            Ok(login_status(
                &id,
                "waiting",
                "请使用手机 QQ 扫描二维码",
                None,
            ))
        }
        "67" => {
            restore(state, id.clone(), s).await;
            Ok(login_status(
                &id,
                "scanned",
                "已扫码，请在手机上确认登录",
                None,
            ))
        }
        "65" => {
            if s.refresh_count >= MAX_REFRESHES {
                s.clear_secrets();
                return Ok(login_status(&id, "expired", "二维码刷新次数已达上限", None));
            }
            let mut q = Url::parse("https://ssl.ptlogin2.qq.com/ptqrshow").unwrap();
            q.query_pairs_mut().extend_pairs([
                ("appid", s.appid),
                ("e", "2"),
                ("l", "M"),
                ("s", "3"),
                ("d", "72"),
                ("v", "4"),
                ("t", &unix_millis().to_string()),
                ("daid", DAID),
                ("pt_3rd_aid", "0"),
                ("u1", s.u1),
            ]);
            let old_sig = jar_cookies(&s.jar, &q).get("qrsig").cloned();
            let r = match s
                .client
                .get(q.clone())
                .header(USER_AGENT, &s.user_agent)
                .send()
                .await
            {
                Ok(v) => v,
                Err(_) => {
                    restore(state, id.clone(), s).await;
                    return Ok(login_status(
                        &id,
                        "error",
                        "刷新二维码失败，请稍后重试",
                        None,
                    ));
                }
            };
            if !r.status().is_success() {
                if cancelled(state, &id, &s).await {
                    s.clear_secrets();
                } else {
                    restore(state, id.clone(), s).await;
                }
                return Ok(login_status(
                    &id,
                    "error",
                    "刷新二维码失败，请稍后重试",
                    None,
                ));
            }
            let response_sig = r
                .cookies()
                .find(|cookie| cookie.name() == "qrsig" && !cookie.value().is_empty())
                .map(|cookie| cookie.value().to_owned());
            let sig = match response_sig.filter(|value| old_sig.as_ref() != Some(value)) {
                Some(value) => value,
                None => {
                    s.clear_secrets();
                    return Ok(login_status(&id, "error", "二维码刷新响应不完整", None));
                }
            };
            let image = match r.bytes().await {
                Ok(image) => image,
                Err(_) => {
                    if cancelled(state, &id, &s).await {
                        s.clear_secrets();
                    } else {
                        restore(state, id.clone(), s).await;
                    }
                    return Ok(login_status(
                        &id,
                        "error",
                        "读取刷新二维码失败，请稍后重试",
                        None,
                    ));
                }
            };
            s.ptqrtoken = ptqr_token(&sig);
            s.refresh_count += 1;
            if cancelled(state, &id, &s).await {
                s.clear_secrets();
                return Ok(login_status(&id, "cancelled", "二维码登录已取消", None));
            }
            state.sessions.lock().await.insert(id.clone(), s);
            Ok(login_status(
                &id,
                "refreshed",
                "二维码已自动刷新",
                Some(format!("data:image/png;base64,{}", BASE64.encode(image))),
            ))
        }
        "0" => {
            let mut u = match validate_success_url(&url) {
                Ok(url) => url,
                Err(_) => {
                    s.clear_secrets();
                    return Ok(login_status(&id, "error", "登录跳转校验失败", None));
                }
            };
            let callback = u
                .query_pairs()
                .find(|(k, _)| k == "uin")
                .and_then(|(_, v)| normalized_uin(&v))
                .ok_or("登录响应不完整")?;
            let mut final_status = None;
            for _ in 0..=5 {
                if !validate_login_hop(&u) {
                    s.clear_secrets();
                    return Ok(login_status(&id, "error", "登录跳转校验失败", None));
                }
                let r = match s
                    .client
                    .get(u.clone())
                    .header(USER_AGENT, &s.user_agent)
                    .send()
                    .await
                {
                    Ok(response) => response,
                    Err(_) => {
                        restore(state, id.clone(), s).await;
                        return Ok(login_status(
                            &id,
                            "error",
                            "确认 QQ 登录失败，请稍后重试",
                            None,
                        ));
                    }
                };
                if r.status().is_redirection() {
                    let location = match r
                        .headers()
                        .get(LOCATION)
                        .and_then(|value| value.to_str().ok())
                    {
                        Some(location) => location,
                        None => {
                            s.clear_secrets();
                            return Ok(login_status(&id, "error", "登录跳转响应不完整", None));
                        }
                    };
                    u = match u.join(location) {
                        Ok(next) if validate_login_hop(&next) => next,
                        _ => {
                            s.clear_secrets();
                            return Ok(login_status(&id, "error", "登录跳转校验失败", None));
                        }
                    };
                    continue;
                }
                final_status = Some(r.status());
                break;
            }
            if final_status.is_none_or(|status| !status.is_success()) {
                s.clear_secrets();
                return Ok(login_status(&id, "error", "确认 QQ 登录失败", None));
            }
            if cancelled(state, &id, &s).await {
                s.clear_secrets();
                return Ok(login_status(&id, "cancelled", "二维码登录已取消", None));
            }
            let c = jar_cookies(&s.jar, &Url::parse("https://qzone.qq.com/").unwrap());
            let Some(cu) = c
                .get("uin")
                .or_else(|| c.get("p_uin"))
                .and_then(|v| normalized_uin(v))
            else {
                s.clear_secrets();
                return Ok(login_status(&id, "error", "登录凭据不完整", None));
            };
            let Some(key) = c.get("p_skey").filter(|v| !v.is_empty()).cloned() else {
                s.clear_secrets();
                return Ok(login_status(&id, "error", "登录凭据不完整", None));
            };
            if cu != callback {
                s.clear_secrets();
                return Ok(login_status(&id, "error", "登录身份校验失败", None));
            }
            let _commit = state.lifecycle_commit.lock().await;
            if cancelled(state, &id, &s).await {
                s.clear_secrets();
                return Ok(login_status(&id, "cancelled", "二维码登录已取消", None));
            }
            *state.session.lock().await = Some(LoginSession {
                cookies: c,
                uin: Some(cu),
                g_tk: Some(bkn(&key)),
                user_agent: s.user_agent.clone(),
                web_attempt: None,
            });
            s.clear_secrets();
            Ok(login_status(&id, "success", "登录成功", None))
        }
        _ => {
            s.clear_secrets();
            Ok(login_status(
                &id,
                "error",
                "QQ 登录返回了无法识别的状态",
                None,
            ))
        }
    }
}

#[tauri::command]
pub async fn cancel_qr_login(
    state: tauri::State<'_, QLoginState>,
    id: QrSessionId,
) -> Result<(), String> {
    let _commit = state.lifecycle_commit.lock().await;
    {
        let mut epochs = state.cancel_epochs.lock().await;
        *epochs.entry(id.clone()).or_insert(0) += 1;
    }
    let removed = state.sessions.lock().await.remove(&id);
    if let Some(mut session) = removed {
        session.clear_secrets();
    }
    Ok(())
}

#[tauri::command]
pub async fn get_login_status(state: tauri::State<'_, QLoginState>) -> Result<LoginStatus, String> {
    let guard = state.session.lock().await;
    if guard
        .as_ref()
        .is_some_and(|session| session.uin.is_some() && session.g_tk.is_some())
    {
        return Ok(LoginStatus {
            status: "success",
            message: "已登录".into(),
            session_id: None,
            qr_image: None,
        });
    }
    Ok(LoginStatus {
        status: "loggedOut",
        message: "尚未登录".into(),
        session_id: None,
        qr_image: None,
    })
}

fn decode_legacy_text(bytes: &[u8]) -> String {
    String::from_utf8(bytes.to_vec()).unwrap_or_else(|_| GB18030.decode(bytes).0.into_owned())
}

fn parse_user_info(text: &str) -> Result<(String, Option<String>), String> {
    let start = text.find('{').ok_or("用户资料响应格式不正确")?;
    let end = text
        .rfind('}')
        .filter(|end| *end >= start)
        .ok_or("用户资料响应格式不正确")?;
    let response: UserInfoResponse =
        serde_json::from_str(&text[start..=end]).map_err(|_| "用户资料响应格式不正确")?;
    if response.code != 0 {
        return Err("QQ 用户资料接口返回错误".into());
    }
    let data = response.data.ok_or("QQ 用户资料接口未返回资料")?;
    let value = |names: &[&str]| {
        names
            .iter()
            .find_map(|name| data.get(*name))
            .and_then(|v| v.as_str())
            .map(str::to_owned)
    };
    Ok((
        value(&["nickname", "nick", "name"]).unwrap_or_else(|| "QQ 用户".into()),
        value(&["avatar", "face"]),
    ))
}

async fn request_user_info(
    state: &QLoginState,
    auth: &QzoneAuth,
    url: Url,
) -> Result<(String, Option<String>), String> {
    let response = state
        .client()
        .get(url)
        .header(USER_AGENT, &auth.user_agent)
        .header(REFERER, format!("https://user.qzone.qq.com/{}", auth.uin))
        .header(reqwest::header::COOKIE, &auth.cookie_header)
        .send()
        .await
        .map_err(|_| "获取 QQ 用户资料失败")?;
    if !response.status().is_success() {
        return Err("QQ 用户资料接口暂时不可用".into());
    }
    if response
        .content_length()
        .is_some_and(|length| length > MAX_PROFILE_BYTES as u64)
    {
        return Err("QQ 用户资料响应过大".into());
    }
    let mut response = response;
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| "读取 QQ 用户资料失败")? {
        if bytes.len().saturating_add(chunk.len()) > MAX_PROFILE_BYTES {
            return Err("QQ 用户资料响应过大".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    parse_user_info(&decode_legacy_text(&bytes))
}

#[tauri::command]
pub async fn get_qzone_login_user(
    state: tauri::State<'_, QLoginState>,
) -> Result<QzoneLoginUser, String> {
    let auth = state.qzone_auth().await?;
    let mut vip = Url::parse(
        "https://h5.qzone.qq.com/proxy/domain/vip.qzone.qq.com/fcg-bin/fcg_get_vipinfo_mobile",
    )
    .map_err(|_| "用户资料地址无效")?;
    vip.query_pairs_mut()
        .append_pair("get_all", "1")
        .append_pair("uin", &auth.uin)
        .append_pair("g_tk", &auth.g_tk.to_string());
    let (nickname, _) = match request_user_info(&state, &auth, vip).await {
        Ok(value) => value,
        Err(_) => {
            let mut legacy = Url::parse("https://h5.qzone.qq.com/proxy/domain/base.qzone.qq.com/cgi-bin/user/cgi_userinfo_get_all").map_err(|_| "用户资料地址无效")?;
            legacy
                .query_pairs_mut()
                .append_pair("uin", &auth.uin)
                .append_pair("vuin", &auth.uin)
                .append_pair("fupdate", "1")
                .append_pair("g_tk", &auth.g_tk.to_string());
            request_user_info(&state, &auth, legacy).await?
        }
    };
    let avatar_url = Url::parse_with_params(
        "https://q1.qlogo.cn/g",
        &[("b", "qq"), ("nk", auth.uin.as_str()), ("s", "100")],
    )
    .map_err(|_| "头像地址无效")?;
    let avatar_image = match state
        .client()
        .get(avatar_url)
        .header(USER_AGENT, &auth.user_agent)
        .send()
        .await
    {
        Ok(response)
            if response.status().is_success()
                && response
                    .content_length()
                    .is_none_or(|n| n <= MAX_AVATAR_BYTES as u64) =>
        {
            let content_type = response
                .headers()
                .get(CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .and_then(allowed_avatar_content_type);
            let Some(content_type) = content_type else {
                return Ok(QzoneLoginUser {
                    uin: auth.uin,
                    nickname,
                    avatar_image: None,
                });
            };
            let mut response = response;
            let mut bytes = Vec::new();
            let mut valid = true;
            loop {
                match response.chunk().await {
                    Ok(Some(chunk)) => {
                        if bytes.len().saturating_add(chunk.len()) > MAX_AVATAR_BYTES {
                            valid = false;
                            break;
                        }
                        bytes.extend_from_slice(&chunk);
                    }
                    Ok(None) => break,
                    Err(_) => {
                        valid = false;
                        break;
                    }
                }
            }
            valid.then(|| format!("data:{content_type};base64,{}", BASE64.encode(bytes)))
        }
        _ => None,
    };
    Ok(QzoneLoginUser {
        uin: auth.uin,
        nickname,
        avatar_image,
    })
}

fn clear_window_qq_cookies(window: &tauri::WebviewWindow) -> Result<(), String> {
    let cookies = window.cookies().map_err(|_| "读取 QQ Cookie 失败")?;
    for cookie in cookies {
        if allowed_qq_cookie_domain(cookie.domain())
            && REQUIRED_WEB_COOKIES.contains(&cookie.name())
        {
            window
                .delete_cookie(cookie)
                .map_err(|_| "清理 QQ Cookie 失败")?;
        }
    }
    Ok(())
}

fn allowed_avatar_content_type(value: &str) -> Option<&'static str> {
    let media_type = value.split(';').next()?.trim();
    if media_type.eq_ignore_ascii_case("image/jpeg") {
        Some("image/jpeg")
    } else if media_type.eq_ignore_ascii_case("image/png") {
        Some("image/png")
    } else if media_type.eq_ignore_ascii_case("image/webp") {
        Some("image/webp")
    } else if media_type.eq_ignore_ascii_case("image/gif") {
        Some("image/gif")
    } else {
        None
    }
}

fn clear_web_login_window(app: &tauri::AppHandle) -> Result<(), String> {
    let Some(window) = app.get_webview_window(WEB_LOGIN_WINDOW_LABEL) else {
        return Ok(());
    };
    let cookies_cleared = clear_window_qq_cookies(&window).is_ok();
    let window_closed = window.close().is_ok();
    if cookies_cleared && window_closed {
        Ok(())
    } else {
        Err("清理 QQ 登录状态失败".into())
    }
}

#[tauri::command]
pub async fn logout_qzone(
    app: tauri::AppHandle,
    state: tauri::State<'_, QLoginState>,
) -> Result<(), String> {
    state.clear_session().await;
    let mut failed = false;
    if let Some(main) = app.get_webview_window("main") {
        failed |= clear_window_qq_cookies(&main).is_err();
    }
    failed |= clear_web_login_window(&app).is_err();
    if failed {
        Err("登录状态已清除，但部分 WebView Cookie 清理失败".into())
    } else {
        Ok(())
    }
}

#[tauri::command]
pub async fn open_web_login(
    app: tauri::AppHandle,
    state: tauri::State<'_, QLoginState>,
) -> Result<LoginStatus, String> {
    // Serialize the whole replacement flow without holding the lifecycle lock while waiting
    // for Tauri to release a closed window's label.
    let _open = state.web_open.lock().await;
    {
        let _commit = state.lifecycle_commit.lock().await;
        if let Some(window) = app.get_webview_window(WEB_LOGIN_WINDOW_LABEL) {
            if state.active_web_attempt.load(Ordering::SeqCst) != 0 {
                window.set_focus().ok();
                return Ok(LoginStatus {
                    status: "webLoginOpened",
                    message: "登录窗口已打开，请在窗口中完成 QQ 登录".into(),
                    session_id: None,
                    qr_image: None,
                });
            }
            window
                .close()
                .map_err(|_| WEB_LOGIN_OPEN_ERROR.to_owned())?;
        }
    }

    for _ in 0..WEB_LOGIN_CLOSE_POLL_ATTEMPTS {
        if app.get_webview_window(WEB_LOGIN_WINDOW_LABEL).is_none() {
            break;
        }
        tokio::time::sleep(WEB_LOGIN_CLOSE_POLL_INTERVAL).await;
    }

    let _commit = state.lifecycle_commit.lock().await;
    if app.get_webview_window(WEB_LOGIN_WINDOW_LABEL).is_some() {
        state.active_web_attempt.store(0, Ordering::SeqCst);
        return Err(WEB_LOGIN_OPEN_ERROR.into());
    }

    let login_url = WEB_LOGIN_URL
        .parse::<Url>()
        .map_err(|_| WEB_LOGIN_OPEN_ERROR.to_owned())?;
    let builder = WebviewWindowBuilder::new(
        &app,
        WEB_LOGIN_WINDOW_LABEL,
        WebviewUrl::External(login_url),
    )
    .title("QQ 账号登录")
    .inner_size(800.0, 720.0);
    #[cfg(desktop)]
    let builder = builder.center();
    builder
        .build()
        .map_err(|_| WEB_LOGIN_OPEN_ERROR.to_owned())?;

    let attempt = state.web_generation.fetch_add(1, Ordering::SeqCst) + 1;
    state.active_web_attempt.store(attempt, Ordering::SeqCst);
    Ok(LoginStatus {
        status: "webLoginOpened",
        message: "请在打开的窗口中完成 QQ 登录".into(),
        session_id: None,
        qr_image: None,
    })
}

#[tauri::command]
pub async fn check_web_login(
    app: tauri::AppHandle,
    state: tauri::State<'_, QLoginState>,
) -> Result<LoginStatus, String> {
    let generation = state.active_web_attempt.load(Ordering::SeqCst);
    if generation == 0 {
        return Ok(LoginStatus {
            status: "webLoginCancelled",
            message: "网页登录已取消".into(),
            session_id: None,
            qr_image: None,
        });
    }
    let Some(window) = app.get_webview_window(WEB_LOGIN_WINDOW_LABEL) else {
        return Ok(LoginStatus {
            status: "webLoginCancelled",
            message: "登录窗口已关闭".into(),
            session_id: None,
            qr_image: None,
        });
    };

    let url = Url::parse("https://i.qq.com").map_err(|e| format!("{e}"))?;

    let (cookies, all_cookies) = tokio::task::spawn_blocking(move || {
        let cookies = window.cookies_for_url(url).unwrap_or_default();
        let all = window.cookies().unwrap_or_default();
        (cookies, all)
    })
    .await
    .map_err(|e| format!("读取 Cookie 线程异常: {e}"))?;

    let mut cookie_map: HashMap<String, String> = HashMap::new();
    for c in &cookies {
        if REQUIRED_WEB_COOKIES.contains(&c.name()) && !c.value().trim().is_empty() {
            cookie_map.insert(c.name().to_string(), c.value().to_string());
        }
    }
    // URL-scoped extraction is preferred. The fallback accepts only explicitly required
    // credentials from known QQ domains, never the WebView's complete cookie collection.
    if cookie_map.get("p_skey").is_none_or(|v| v.is_empty()) {
        for c in &all_cookies {
            let allowed_domain = allowed_qq_cookie_domain(c.domain());
            if allowed_domain
                && REQUIRED_WEB_COOKIES.contains(&c.name())
                && !c.value().trim().is_empty()
            {
                cookie_map
                    .entry(c.name().to_string())
                    .or_insert_with(|| c.value().to_string());
            }
        }
    }

    let p_skey = match cookie_map.get("p_skey").filter(|v| !v.is_empty()) {
        Some(v) => v.clone(),
        None => {
            return Ok(LoginStatus {
                status: "webLoginWaiting",
                message: "等待登录完成…".into(),
                session_id: None,
                qr_image: None,
            });
        }
    };

    let uin = cookie_map
        .get("uin")
        .and_then(|value| normalized_uin(value));
    let p_uin = cookie_map
        .get("p_uin")
        .and_then(|value| normalized_uin(value));
    if uin.is_some() && p_uin.is_some() && uin != p_uin {
        return Err("登录 Cookie 中的账号标识不一致".into());
    }
    let uin = uin.or(p_uin).ok_or("登录 Cookie 缺少有效账号标识")?;

    let g_tk = bkn(&p_skey);
    let user_agent = account_user_agent(&uin);

    let session = LoginSession {
        cookies: cookie_map,
        uin: Some(uin),
        g_tk: Some(g_tk),
        user_agent,
        web_attempt: Some(generation),
    };

    let _commit = state.lifecycle_commit.lock().await;
    if state.active_web_attempt.load(Ordering::SeqCst) != generation
        || app.get_webview_window(WEB_LOGIN_WINDOW_LABEL).is_none()
    {
        return Ok(LoginStatus {
            status: "webLoginCancelled",
            message: "网页登录已取消".into(),
            session_id: None,
            qr_image: None,
        });
    }
    *state.session.lock().await = Some(session);
    state.active_web_attempt.store(0, Ordering::SeqCst);
    if let Some(w) = app.get_webview_window(WEB_LOGIN_WINDOW_LABEL) {
        w.close().ok();
    }

    Ok(LoginStatus {
        status: "success",
        message: "登录成功".into(),
        session_id: None,
        qr_image: None,
    })
}

#[tauri::command]
pub async fn cancel_web_login(
    app: tauri::AppHandle,
    state: tauri::State<'_, QLoginState>,
) -> Result<(), String> {
    // Invalidate only the current WebView attempt. If its check committed while cancel
    // waited for the lifecycle lock, revoke that attempt's session but retain older auth.
    // Capture and invalidate the active attempt before waiting for a concurrent commit.
    // This value is the queued cancel target and survives check clearing the active slot.
    let captured = state.active_web_attempt.swap(0, Ordering::SeqCst);
    let attempt = (captured != 0).then_some(captured);
    let _commit = state.lifecycle_commit.lock().await;
    state.web_generation.fetch_add(1, Ordering::SeqCst);
    let mut session = state.session.lock().await;
    if attempt.is_some()
        && session
            .as_ref()
            .is_some_and(|session| session.web_attempt == attempt)
    {
        *session = None;
    }
    drop(session);
    clear_web_login_window(&app)
}
