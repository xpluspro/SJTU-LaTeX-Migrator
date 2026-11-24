use super::{client, AppState, ConfigureInput};
use regex::Regex;
use std::io::Read;
use std::sync::{LazyLock, Mutex, MutexGuard};
use std::time::Duration;
use tauri::{async_runtime, Manager, Runtime, WebviewUrl, WebviewWindow, WebviewWindowBuilder};
use url::{Host, Url};

// Serialize opening, probing and retiring each side's popup, off the UI thread.
// Migration only needs the short-lived cookie lock (while snapshotting its URL).
#[derive(Default)]
pub(super) struct SiteAuth {
    popup: async_runtime::Mutex<Option<String>>,
    cookie: Mutex<Option<String>>,
}

impl SiteAuth {
    pub(super) fn cookie(&self) -> Result<Option<String>, String> {
        Ok(lock(&self.cookie)?.clone())
    }

    fn reconfigure(
        &self,
        current: &mut String,
        next: String,
        popup: &mut Option<String>,
    ) -> Result<Option<String>, String> {
        if *current == next {
            return Ok(None);
        }
        *lock(&self.cookie)? = None;
        *current = next;
        Ok(popup.take())
    }
}

fn lock<T>(mutex: &Mutex<T>) -> Result<MutexGuard<'_, T>, String> {
    mutex.lock().map_err(|_| "登录状态锁失败".into())
}

fn main_only<R: Runtime>(window: &WebviewWindow<R>) -> Result<(), String> {
    if window.label() != "main" {
        return Err("仅主窗口可以操作登录会话".into());
    }
    Ok(())
}

fn site<'a>(state: &'a AppState, side: &str) -> Result<(&'a SiteAuth, &'a Mutex<String>), String> {
    match side {
        "source" => Ok((&state.source_auth, &state.source_url)),
        "target" => Ok((&state.target_auth, &state.target_url)),
        _ => Err("未知登录目标".into()),
    }
}

fn allowed_url(url: &Url) -> bool {
    url.scheme() == "https"
        || (url.scheme() == "http"
            && match url.host() {
                Some(Host::Domain(name)) => name == "localhost",
                Some(Host::Ipv4(ip)) => ip.is_loopback(),
                Some(Host::Ipv6(ip)) => ip.is_loopback(),
                None => false,
            })
}

fn normalize_url(value: &str) -> Result<String, String> {
    let value = value.trim();
    if value.is_empty()
        || value.starts_with('/')
        || value.contains('\\')
        || value.chars().any(char::is_control)
    {
        return Err("请输入有效的系统地址".into());
    }
    let url = if value.contains("://") {
        Url::parse(value)
    } else {
        Url::parse(&format!("https://{value}"))
    }
    .map_err(|_| "系统地址无效")?;
    if !allowed_url(&url)
        || url.host().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(
            "请使用 HTTPS 系统地址，不要包含凭据、查询参数或片段（本机测试可用 HTTP）".into(),
        );
    }
    Ok(url.as_str().trim_end_matches('/').to_owned())
}

pub(super) async fn configure<R: Runtime>(
    window: &WebviewWindow<R>,
    input: ConfigureInput,
) -> Result<(), String> {
    main_only(window)?;
    let source = normalize_url(&input.source_url)?;
    let target = normalize_url(&input.target_url)?;
    let state = window.state::<AppState>();
    let mut source_popup = state.source_auth.popup.lock().await;
    let mut target_popup = state.target_auth.popup.lock().await;
    // Both settings and session invalidations are one update for transfer snapshots.
    let retired = {
        let mut src = lock(&state.source_url)?;
        let mut dst = lock(&state.target_url)?;
        [
            state
                .source_auth
                .reconfigure(&mut src, source, &mut source_popup)?,
            state
                .target_auth
                .reconfigure(&mut dst, target, &mut target_popup)?,
        ]
    };
    for label in retired.into_iter().flatten() {
        if let Some(popup) = window.get_webview_window(&label) {
            popup.destroy().map_err(|_| "无法关闭旧登录窗口")?;
        }
    }
    Ok(())
}

pub(super) async fn open<R: Runtime>(window: &WebviewWindow<R>, side: &str) -> Result<(), String> {
    main_only(window)?;
    let state = window.state::<AppState>();
    let (auth, address) = site(&state, side)?;
    let mut active = auth.popup.lock().await;
    if let Some(existing) = active
        .as_deref()
        .and_then(|label| window.get_webview_window(label))
    {
        if lock(&auth.cookie)?.is_some() {
            existing.destroy().map_err(|_| "无法关闭已完成的登录窗口")?;
        } else {
            existing
                .show()
                .and_then(|_| existing.set_focus())
                .map_err(|_| "无法显示登录窗口")?;
            return Ok(());
        }
    }
    let (path, title) = if side == "source" {
        ("/jaccountlogin", "老系统 OAuth 登录")
    } else {
        ("/oidc/login", "新系统 OAuth 登录")
    };
    let login = Url::parse(&format!("{}{path}", *lock(address)?)).map_err(|_| "登录地址无效")?;
    // Never recycle a label: a delayed close must not destroy a subsequent attempt.
    static NEXT_POPUP: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    let n = NEXT_POPUP.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let label = format!("oauth-{side}-{n}");
    WebviewWindowBuilder::new(window.app_handle(), &label, WebviewUrl::External(login))
        .title(title)
        .inner_size(1080.0, 760.0)
        .resizable(true)
        .center()
        .incognito(true)
        .on_navigation(allowed_url)
        .build()
        .map_err(|_| "无法打开 OAuth 登录窗口")?;
    *lock(&auth.cookie)? = None;
    *active = Some(label);
    Ok(())
}

fn at_dashboard(actual: &Url, dashboard: &Url) -> bool {
    actual.origin() == dashboard.origin()
        && actual.path().trim_end_matches('/') == dashboard.path()
        && actual.username().is_empty()
        && actual.password().is_none()
}

fn matches_project_scope(cookie: &tauri::webview::Cookie<'_>, url: &Url) -> bool {
    // Some WebKit versions do not filter paths, and shared parent-domain SSO
    // cookies are not project-host credentials. Do not forward either.
    let exact_host = cookie
        .domain()
        .zip(url.host_str())
        .is_some_and(|(domain, host)| host.eq_ignore_ascii_case(domain.trim_start_matches('.')));
    let path = cookie.path().unwrap_or("/");
    let path_matches = url
        .path()
        .strip_prefix(path)
        .is_some_and(|tail| tail.is_empty() || path.ends_with('/') || tail.starts_with('/'));
    exact_host && path_matches && (cookie.secure() != Some(true) || url.scheme() == "https")
}

fn cookie_for_dashboard<R: Runtime>(popup: &WebviewWindow<R>, url: &Url) -> Result<String, String> {
    // Use the configured project URL, never the current/IdP URL or global cookie store.
    let cookies = popup
        .cookies_for_url(url.clone())
        .map_err(|_| "无法读取登录窗口会话")?;
    let mut header = String::new();
    for cookie in &cookies {
        if !matches_project_scope(cookie, url) {
            continue;
        }
        if !header.is_empty() {
            header.push_str("; ");
        }
        header.push_str(cookie.name());
        header.push('=');
        header.push_str(cookie.value());
    }
    Ok(header)
}

fn authenticated_dashboard(body: &str) -> bool {
    // Overleaf's base layout also emits CSRF metadata on the anonymous login page.
    // Require a populated logged-in Mongo user id AND the project-list metadata.
    static USER: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(
        r#"(?i)<meta\s+name\s*=\s*["']ol-user_id["'][^>]*?\scontent\s*=\s*["'][0-9a-f]{24}["']"#,
    ).unwrap()
    });
    static PROJECTS: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(
            r#"(?i)<meta\s+name\s*=\s*["']ol-(?:projects|prefetchedProjectsBlob)["'](?:\s|/?>)"#,
        )
        .unwrap()
    });
    USER.is_match(body) && PROJECTS.is_match(body)
}

fn probe<R: Runtime>(popup: &WebviewWindow<R>, dashboard: &Url) -> Result<Option<String>, String> {
    let returned = || popup.url().is_ok_and(|url| at_dashboard(&url, dashboard));
    if !returned() {
        return Ok(None);
    }
    let cookie = cookie_for_dashboard(popup, dashboard)?;
    if cookie.is_empty() || !returned() {
        return Ok(None);
    }
    let response = client(&cookie)?
        .get(dashboard.clone()) // canonical path, no callback query, no redirects
        .timeout(Duration::from_secs(10))
        .send()
        .map_err(|_| "无法验证项目首页，请重新尝试登录")?;
    if response.status() != reqwest::StatusCode::OK || !at_dashboard(response.url(), dashboard) {
        return Ok(None);
    }
    let mut body = String::new();
    const LIMIT: u64 = 16 * 1024 * 1024;
    response
        .take(LIMIT + 1)
        .read_to_string(&mut body)
        .map_err(|_| "无法读取项目首页")?;
    if body.len() as u64 > LIMIT {
        return Err("项目首页过大，无法验证登录".into());
    }
    Ok((authenticated_dashboard(&body) && returned()).then_some(cookie))
}

pub(super) async fn status<R: Runtime>(
    window: &WebviewWindow<R>,
    side: &str,
) -> Result<&'static str, String> {
    main_only(window)?;
    let state = window.state::<AppState>();
    let (auth, address) = site(&state, side)?;
    let mut active = auth.popup.lock().await;
    if lock(&auth.cookie)?.is_some() {
        // Retry cleanup without invalidating a verified session.
        if let Some(popup) = active
            .as_deref()
            .and_then(|label| window.get_webview_window(label))
        {
            if popup.destroy().is_ok() {
                *active = None;
            }
        } else {
            *active = None;
        }
        return Ok("connected");
    }
    let Some(popup) = active
        .as_deref()
        .and_then(|label| window.get_webview_window(label))
    else {
        *active = None;
        return Ok("closed");
    };
    let dashboard =
        Url::parse(&format!("{}/project", *lock(address)?)).map_err(|_| "项目地址无效")?;
    let candidate = popup.clone();
    let result = async_runtime::spawn_blocking(move || probe(&candidate, &dashboard))
        .await
        .map_err(|_| "登录验证任务失败")?;
    if window.get_webview_window(popup.label()).is_none() {
        *active = None;
        return Ok("closed");
    }
    let Some(cookie) = result? else {
        return Ok("waiting");
    };
    *lock(&auth.cookie)? = Some(cookie);
    // Publish the session before closing; keep the label if cleanup needs a retry.
    if popup.destroy().is_ok() {
        *active = None;
    }
    Ok("connected")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn excludes_idp_parent_domain_and_unrelated_path_cookies() {
        let url = Url::parse("https://latex.example.edu/site/project").unwrap();
        for (host, path, expected) in [
            ("latex.example.edu", "/", true),
            ("latex.example.edu", "/site", true),
            ("latex.example.edu", "/si", false),
            ("latex.example.edu", "/oidc", false),
            (".example.edu", "/", false),
            ("idp.example.edu", "/", false),
        ] {
            let cookie = tauri::webview::Cookie::build(("sid", "test"))
                .domain(host)
                .path(path)
                .build();
            assert_eq!(matches_project_scope(&cookie, &url), expected);
        }
    }

    #[test]
    fn changing_address_retires_the_session_and_popup() {
        let auth = SiteAuth::default();
        *auth.cookie.lock().unwrap() = Some("sid=test".into());
        let mut address = String::from("https://old.example");
        let mut popup = Some(String::from("oauth-source-1"));
        let unchanged = address.clone();
        assert!(auth
            .reconfigure(&mut address, unchanged, &mut popup)
            .unwrap()
            .is_none());
        assert!(auth.cookie().unwrap().is_some());
        assert_eq!(
            auth.reconfigure(&mut address, "https://new.example".into(), &mut popup)
                .unwrap()
                .as_deref(),
            Some("oauth-source-1")
        );
        assert!(popup.is_none());
        assert!(auth.cookie().unwrap().is_none());
    }

    #[test]
    fn configured_project_origin_and_path_only() {
        let base = normalize_url(" HTTPS://LATEX.EXAMPLE:443/site/ ").unwrap();
        assert_eq!(base, "https://latex.example/site");
        let expected = Url::parse(&format!("{base}/project")).unwrap();
        assert!(at_dashboard(
            &Url::parse("https://latex.example/site/project/?page=1").unwrap(),
            &expected
        ));
        for wrong in [
            "https://idp.example/site/project",
            "https://latex.example:444/site/project",
            "http://latex.example/site/project",
            "https://latex.example/login?next=/site/project",
            "https://latex.example/project",
            "https://latex.example/site/project/abc",
        ] {
            assert!(!at_dashboard(&Url::parse(wrong).unwrap(), &expected));
        }
        for bad in [
            "",
            "ftp://latex.example",
            "https://a@latex.example",
            "https://latex.example?q",
            "https://latex.example/#f",
        ] {
            assert!(normalize_url(bad).is_err());
        }
        assert!(normalize_url("http://127.0.0.1:8000").is_ok());
    }

    #[test]
    fn distinguishes_empty_dashboard_from_anonymous_session() {
        let projects = r#"<meta name="ol-projects" content="[]">"#;
        let identity = r#"<meta name="ol-user_id" content="507f1f77bcf86cd799439011">"#;
        assert!(authenticated_dashboard(&format!("{projects}{identity}")));
        assert!(!authenticated_dashboard(identity));
        assert!(!authenticated_dashboard(&format!(
            "{projects}<meta name='ol-user_id' content=''>"
        )));
        assert!(!authenticated_dashboard(
            "<meta name='ol-csrfToken' content='anonymous'>"
        ));
    }
}
