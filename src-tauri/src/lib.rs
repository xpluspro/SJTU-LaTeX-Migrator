use html_escape::decode_html_entities;
use reqwest::blocking::{
    multipart::{Form, Part},
    Client, RequestBuilder,
};
use reqwest::header::{HeaderMap, HeaderValue, COOKIE, REFERER, USER_AGENT};
use serde::{Deserialize, Serialize};
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use tauri::{Emitter, Manager};
use zip::ZipArchive;

mod auth;
mod retry;

mod handoff;
mod local_upload;
mod queue;
mod work;
use work::{prepare_items, process_item, WorkItem};

const DEFAULT_SOURCE: &str = "https://latex.sjtu.edu.cn";
const DEFAULT_TARGET: &str = "https://latexpro.sjtu.edu.cn";
const MAX_ZIP_BYTES: u64 = 50_000_000;

#[derive(Default)]
struct AppState {
    source_url: Mutex<String>,
    target_url: Mutex<String>,
    source_auth: auth::SiteAuth,
    target_auth: auth::SiteAuth,
    local_zip: Mutex<Vec<PathBuf>>,
    queue: queue::Registry,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Project {
    id: String,
    name: String,
    owner: Option<String>,
    last_updated: Option<String>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct TaskEvent {
    batch_id: String,
    kind: String,
    project: Option<String>,
    index: Option<usize>,
    total: Option<usize>,
    message: String,
    status: Option<String>,
    path: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ConfigureInput {
    source_url: String,
    target_url: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct MigrateInput {
    project_ids: Vec<String>,
    workdir: String,
}

fn app_state<R: tauri::Runtime>(app: &tauri::AppHandle<R>) -> tauri::State<'_, AppState> {
    app.state::<AppState>()
}

fn cookie_header<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    source: bool,
) -> Result<String, String> {
    let state = app_state(app);
    let auth = if source {
        &state.source_auth
    } else {
        &state.target_auth
    };
    let cookie = auth.cookie()?;
    cookie.ok_or_else(|| {
        if source {
            "请先在老系统 OAuth 窗口完成登录"
        } else {
            "请先在新系统 OAuth 窗口完成登录"
        }
        .into()
    })
}

fn client(cookie: &str) -> Result<Client, String> {
    let mut headers = HeaderMap::new();
    headers.insert(
        USER_AGENT,
        HeaderValue::from_static("SJTU-LaTeX-Migrator/0.2"),
    );
    let mut value = HeaderValue::from_str(cookie).map_err(|_| "登录 Cookie 格式无效")?;
    value.set_sensitive(true);
    headers.insert(COOKIE, value);
    Client::builder()
        .default_headers(headers)
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(std::time::Duration::from_secs(30))
        .timeout(std::time::Duration::from_secs(600))
        .build()
        .map_err(|e| e.to_string())
}

fn with_csrf(builder: RequestBuilder, csrf: Option<&str>) -> RequestBuilder {
    match csrf {
        Some(token) => builder.header("x-csrf-token", token),
        None => builder,
    }
}

#[tauri::command]
async fn configure<R: tauri::Runtime>(
    window: tauri::WebviewWindow<R>,
    input: ConfigureInput,
) -> Result<(), String> {
    auth::configure(&window, input).await
}

#[tauri::command]
async fn open_oauth<R: tauri::Runtime>(
    window: tauri::WebviewWindow<R>,
    side: String,
) -> Result<(), String> {
    auth::open(&window, &side).await
}

#[tauri::command]
async fn auth_status<R: tauri::Runtime>(
    window: tauri::WebviewWindow<R>,
    side: String,
) -> Result<&'static str, String> {
    auth::status(&window, &side).await
}

fn csrf_from_html(body: &str) -> Option<String> {
    let pattern = regex::Regex::new(
        r#"(?i)<meta[^>]+name=[\"']ol-csrfToken[\"'][^>]+content=[\"']([^\"']+)"#,
    )
    .ok()?;
    pattern
        .captures(body)
        .and_then(|caps| caps.get(1))
        .map(|m| decode_html_entities(m.as_str()).to_string())
}

fn projects_from_html(body: &str) -> Result<Vec<Project>, String> {
    let names = ["ol-prefetchedProjectsBlob", "ol-projects"];
    for name in names {
        let pattern = regex::Regex::new(&format!(
            r#"(?i)<meta[^>]+name=[\"']{name}[\"'][^>]+content=[\"']([^\"']+)"#
        ))
        .map_err(|e| e.to_string())?;
        if let Some(captures) = pattern.captures(body) {
            let raw = decode_html_entities(captures.get(1).unwrap().as_str()).to_string();
            let value: serde_json::Value =
                serde_json::from_str(&raw).map_err(|e| format!("项目数据解析失败：{e}"))?;
            let list = value
                .get("projects")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_else(|| value.as_array().cloned().unwrap_or_default());
            return Ok(list
                .into_iter()
                .filter_map(|item| {
                    let id = item
                        .get("id")
                        .or_else(|| item.get("_id"))?
                        .as_str()?
                        .to_owned();
                    let name = item.get("name")?.as_str()?.to_owned();
                    if item
                        .get("archived")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false)
                        || item
                            .get("trashed")
                            .and_then(|v| v.as_bool())
                            .unwrap_or(false)
                    {
                        return None;
                    }
                    Some(Project {
                        id,
                        name,
                        owner: item
                            .get("owner")
                            .and_then(|v| v.get("email"))
                            .and_then(|v| v.as_str())
                            .map(str::to_owned),
                        last_updated: item
                            .get("lastUpdated")
                            .and_then(|v| v.as_str())
                            .map(str::to_owned),
                    })
                })
                .collect());
        }
    }
    Err("未找到项目数据。请确认老系统登录已完成并打开项目首页。".into())
}

#[tauri::command]
async fn list_projects<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
) -> Result<Vec<Project>, String> {
    tauri::async_runtime::spawn_blocking(move || Session::snapshot(&app, true)?.projects())
        .await
        .map_err(|e| format!("读取项目任务失败：{e}"))?
}

struct Session {
    base: String,
    http: Client,
    csrf: Mutex<Option<String>>,
}

impl Session {
    fn snapshot<R: tauri::Runtime>(
        app: &tauri::AppHandle<R>,
        source: bool,
    ) -> Result<Self, String> {
        // configure also takes the URL lock before changing this side's credentials.
        let (base, cookie) = {
            let state = app_state(app);
            let url = if source {
                &state.source_url
            } else {
                &state.target_url
            };
            let url = url.lock().map_err(|_| "状态锁失败")?;
            (url.clone(), cookie_header(app, source)?)
        };
        Ok(Self {
            base,
            http: client(&cookie)?,
            csrf: Mutex::new(None),
        })
    }

    fn project_page(&self) -> Result<String, String> {
        let url = format!("{}/project", self.base);
        for attempt in 0..retry::MAX_ATTEMPTS {
            eprintln!("读取项目：尝试 {}/{}", attempt + 1, retry::MAX_ATTEMPTS);
            let response = match self.http.get(&url).send() {
                Ok(response) => response,
                Err(error)
                    if retry::retryable_get_error(&error) && attempt + 1 < retry::MAX_ATTEMPTS =>
                {
                    retry::wait_before_retry(attempt);
                    continue;
                }
                Err(error) => {
                    return Err(format!(
                        "读取项目失败（已尝试 {} 次）：{error}",
                        attempt + 1
                    ))
                }
            };
            if retry::retryable_status(response.status()) && attempt + 1 < retry::MAX_ATTEMPTS {
                retry::wait_before_retry(attempt);
                continue;
            }
            check_status(&response, "读取项目")?;
            match response.text() {
                Ok(body) => return Ok(body),
                Err(error)
                    if retry::retryable_get_error(&error) && attempt + 1 < retry::MAX_ATTEMPTS =>
                {
                    retry::wait_before_retry(attempt);
                }
                Err(error) => {
                    return Err(format!(
                        "读取项目失败（已尝试 {} 次）：{error}",
                        attempt + 1
                    ))
                }
            }
        }
        Err("读取项目失败：重试次数已用尽".into())
    }

    fn csrf_token(&self) -> Result<String, String> {
        let mut cached = self.csrf.lock().map_err(|_| "状态锁失败")?;
        if let Some(token) = cached.as_ref() {
            return Ok(token.clone());
        }
        let token =
            csrf_from_html(&self.project_page()?).ok_or("未找到上传凭证，请重新登录新系统")?;
        *cached = Some(token.clone());
        Ok(token)
    }

    fn invalidate_csrf(&self) {
        if let Ok(mut cached) = self.csrf.lock() {
            *cached = None;
        }
    }

    fn projects(&self) -> Result<Vec<Project>, String> {
        projects_from_html(&self.project_page()?)
    }
}

fn check_status(response: &reqwest::blocking::Response, operation: &str) -> Result<(), String> {
    let status = response.status();
    if status.is_redirection() || matches!(status.as_u16(), 401 | 403) {
        return Err(format!(
            "{operation}失败：HTTP {status}，请检查登录状态和权限"
        ));
    }
    if !status.is_success() {
        return Err(format!("{operation}失败：HTTP {status}"));
    }
    Ok(())
}

mod transfer;

fn download_zip<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    source: &Session,
    project: &Project,
    path: &Path,
    ctx: &queue::Context,
) -> Result<u64, String> {
    let parent = path.parent().ok_or("工作目录无效")?;
    let url = format!("{}/project/{}/download/zip", source.base, project.id);
    let mut last_error = String::from("未知错误");
    for attempt in 0..retry::MAX_ATTEMPTS {
        ctx.checkpoint();
        eprintln!(
            "下载 {}：尝试 {}/{}（已读取 0 字节）",
            project.name,
            attempt + 1,
            retry::MAX_ATTEMPTS
        );
        let mut response = match source.http.get(&url).send() {
            Ok(response) => response,
            Err(error)
                if retry::retryable_get_error(&error) && attempt + 1 < retry::MAX_ATTEMPTS =>
            {
                last_error = error.to_string();
                eprintln!(
                    "下载尝试 {} 失败（可重试，字节数重置为 0）：{last_error}",
                    attempt + 1
                );
                retry::wait_before_retry(attempt);
                continue;
            }
            Err(error) => return Err(format!("下载失败（已尝试 {} 次）：{error}", attempt + 1)),
        };
        if retry::retryable_status(response.status()) && attempt + 1 < retry::MAX_ATTEMPTS {
            last_error = format!("HTTP {}", response.status());
            eprintln!(
                "下载尝试 {} 失败（可重试，字节数重置为 0）：{last_error}",
                attempt + 1
            );
            retry::wait_before_retry(attempt);
            continue;
        }
        check_status(&response, "下载")?;
        if is_html(&response) {
            return Err("下载失败：收到网页而非 ZIP，请重新登录老系统".into());
        }
        // Do not overwrite a previously saved ZIP with an incomplete download.
        let mut file = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
        let mut progress = transfer::Progress::new(response.content_length(), |bytes, total| {
            transfer::emit(app, ctx, &project.name, "download", bytes, total);
        });
        let mut buffer = [0_u8; 64 * 1024];
        let mut retry_body = None;
        loop {
            ctx.checkpoint();
            match response.read(&mut buffer) {
                Ok(0) => break,
                Ok(read) => {
                    file.write_all(&buffer[..read]).map_err(|e| e.to_string())?;
                    progress.advance(read);
                }
                Err(error) if retry::retryable_body_error(&error) => {
                    retry_body = Some(error);
                    break;
                }
                Err(error) => return Err(format!("下载失败：读取 ZIP 内容时出错：{error}")),
            }
        }
        if let Some(error) = retry_body {
            last_error = error.to_string();
            if attempt + 1 < retry::MAX_ATTEMPTS {
                eprintln!(
                    "下载尝试 {} 失败（可重试，已读取 {} 字节，字节数重置为 0）：{last_error}",
                    attempt + 1,
                    progress.bytes()
                );
                retry::wait_before_retry(attempt);
                continue;
            }
            return Err(format!(
                "下载失败（已尝试 {} 次）：{last_error}",
                attempt + 1
            ));
        }
        progress.finish();
        ZipArchive::new(file.as_file()).map_err(|_| "下载失败：无效的 ZIP，请检查登录状态")?;
        let bytes = progress.bytes();
        file.persist_noclobber(path).map_err(|e| e.to_string())?;
        return Ok(bytes);
    }
    Err(format!(
        "下载失败（已尝试 {} 次）：{last_error}",
        retry::MAX_ATTEMPTS
    ))
}

fn is_html(response: &reqwest::blocking::Response) -> bool {
    response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|header| header.to_str().ok())
        .and_then(|header| header.split(';').next())
        .is_some_and(|mime| mime.trim().eq_ignore_ascii_case("text/html"))
}


fn validate_upload_zip(path: &Path) -> Result<(File, u64), String> {
    let mut file = File::open(path).map_err(|e| format!("无法读取 ZIP：{e}"))?;
    let metadata = file
        .metadata()
        .map_err(|e| format!("无法读取 ZIP 属性：{e}"))?;
    if !metadata.is_file() {
        return Err("上传失败：ZIP 路径不是普通文件".into());
    }
    let length = metadata.len();
    if length == 0 {
        return Err("上传失败：ZIP 文件为空".into());
    }
    if length > MAX_ZIP_BYTES {
        return Err("上传失败：ZIP 超过 50 MB（50,000,000 字节）限制".into());
    }
    let archive =
        ZipArchive::new(&mut file).map_err(|_| "上传失败：不是有效的 ZIP 文件".to_string())?;
    if archive.len() == 0 {
        return Err("上传失败：ZIP 压缩包为空".into());
    }
    drop(archive);
    file.seek(SeekFrom::Start(0))
        .map_err(|e| format!("无法准备 ZIP：{e}"))?;
    Ok((file, length))
}

fn upload_uncertain_error(error: impl std::fmt::Display) -> String {
    format!(
        "上传结果不确定（{error}）。请先在新系统确认目标项目是否已创建；若未创建，请使用本地 ZIP 上传。"
    )
}

fn upload_zip<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    target: &Session,
    path: &Path,
    project_name: &str,
    wait_event: TaskEvent,
    ctx: &queue::Context,
) -> Result<(), String> {
    let (validated_file, length) = validate_upload_zip(path)?;
    let csrf = target.csrf_token()?;
    let filename = format!("{project_name}.zip");
    let mut last_error = String::from("未知错误");
    for attempt in 0..retry::MAX_ATTEMPTS {
        ctx.checkpoint();
        eprintln!(
            "上传 {}：尝试 {}/{}（已读取 0 字节）",
            project_name,
            attempt + 1,
            retry::MAX_ATTEMPTS
        );
        let mut file = validated_file
            .try_clone()
            .map_err(|e| format!("无法读取 ZIP：{e}"))?;
        file.seek(SeekFrom::Start(0))
            .map_err(|e| format!("无法准备 ZIP：{e}"))?;
        let wait_event = wait_event.clone();
        let handle = app.clone();
        let name = project_name.to_owned();
        let gate = ctx.clone();
        let reader = transfer::UploadReader::new(
            transfer::GatedReader {
                inner: file,
                ctx: gate.clone(),
            },
            length,
            move |bytes, total| {
                transfer::emit(&handle, &gate, &name, "upload", bytes, total);
                if Some(bytes) == total {
                    emit_event(&handle, wait_event.clone());
                }
            },
        );
        let part = Part::reader_with_length(reader, length)
            .file_name(filename.clone())
            .mime_str("application/zip")
            .map_err(|e| e.to_string())?;
        // Overleaf UploadsRouter / ProjectUploadController: multer.single('qqfile'),
        // basename(body.name, '.zip'), then { success: true, project_id } on import.
        let form = Form::new()
            .text("name", filename.clone())
            .part("qqfile", part);
        let request = target
            .http
            .post(format!("{}/project/new/upload", target.base))
            .header(REFERER, format!("{}/project", target.base))
            .header(reqwest::header::ACCEPT, "application/json")
            .multipart(form);
        let response = match with_csrf(request, Some(&csrf)).send() {
            Ok(response) => response,
            Err(error) if retry::safe_upload_retry(&error) && attempt + 1 < retry::MAX_ATTEMPTS => {
                last_error = error.to_string();
                eprintln!(
                    "上传尝试 {} 未连接到服务器（字节数重置为 0）：{last_error}",
                    attempt + 1
                );
                retry::wait_before_retry(attempt);
                continue;
            }
            Err(error) if retry::safe_upload_retry(&error) => {
                return Err(format!("上传失败（已尝试 {} 次）：{error}", attempt + 1));
            }
            Err(error) => return Err(upload_uncertain_error(error)),
        };
        let status = response.status();
        if !status.is_success() {
            if status.is_redirection() || matches!(status.as_u16(), 401 | 403 | 419) {
                target.invalidate_csrf();
            }
            return Err(upload_uncertain_error(format!("HTTP {status}")));
        }
        if is_html(&response) {
            target.invalidate_csrf();
            return Err(upload_uncertain_error("收到网页而非项目编号"));
        }
        return confirm_upload(response);
    }
    Err(format!(
        "上传失败（已尝试 {} 次）：{}",
        retry::MAX_ATTEMPTS,
        last_error
    ))
}

fn confirm_upload(reader: impl Read) -> Result<(), String> {
    #[derive(Deserialize)]
    struct Receipt {
        success: bool,
        project_id: Option<String>,
    }
    let receipt: Receipt = serde_json::from_reader(std::io::BufReader::new(reader))
        .map_err(|_| upload_uncertain_error("新系统未返回有效的导入结果"))?;
    if receipt.success && receipt.project_id.is_some_and(|id| !id.trim().is_empty()) {
        Ok(())
    } else {
        Err(upload_uncertain_error("新系统未确认创建项目"))
    }
}

#[cfg(test)]
mod upload_tests {
    use super::{confirm_upload, Session};
    use reqwest::blocking::Client;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::Mutex;

    #[test]
    fn only_an_explicit_import_receipt_confirms_success() {
        for body in [
            "<html>Login</html>",
            r#"{"success":false,"project_id":"123"}"#,
            r#"{"success":true}"#,
            r#"{"success":true,"project_id":" "}"#,
            r#"{"project_id":"123"}"#,
        ] {
            assert!(confirm_upload(body.as_bytes()).is_err(), "{body}");
        }
        assert!(confirm_upload(&br#"{"success":true,"project_id":"123"}"#[..]).is_ok());
    }

    #[test]
    fn csrf_token_fetches_dashboard_once_per_session() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0_u8; 1024];
            stream.read(&mut request).unwrap();
            let body = r#"<meta name="ol-csrfToken" content="cached-token">"#;
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            )
            .unwrap();
        });
        let session = Session {
            base: format!("http://{address}"),
            http: Client::builder().build().unwrap(),
            csrf: Mutex::new(None),
        };
        assert_eq!(session.csrf_token().unwrap(), "cached-token");
        assert_eq!(session.csrf_token().unwrap(), "cached-token");
        server.join().unwrap();
    }
}

fn emit_event<R: tauri::Runtime>(app: &tauri::AppHandle<R>, event: TaskEvent) {
    let _ = app.emit_to("main", "migration-event", event);
}

#[tauri::command]
async fn pick_workdir() -> Result<Option<String>, String> {
    Ok(rfd::AsyncFileDialog::new()
        .set_title("选择 ZIP 工作目录")
        .pick_folder()
        .await
        .map(|handle| handle.path().to_string_lossy().to_string()))
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(AppState {
            source_url: Mutex::new(DEFAULT_SOURCE.into()),
            target_url: Mutex::new(DEFAULT_TARGET.into()),
            ..Default::default()
        })
        .invoke_handler(tauri::generate_handler![
            configure,
            open_oauth,
            auth_status,
            list_projects,
            queue::prepare_queue,
            queue::pause_queue_item,
            queue::run_queue_item,
            local_upload::pick_local_zip,
            pick_workdir
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
