use std::sync::{LazyLock, RwLock};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Locale {
    ZhCn,
    En,
}

impl Default for Locale {
    fn default() -> Self {
        Self::ZhCn
    }
}

static LOCALE: LazyLock<RwLock<Locale>> = LazyLock::new(|| RwLock::new(Locale::ZhCn));

impl Locale {
    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value {
            "zh-CN" | "zh" => Some(Self::ZhCn),
            "en" | "en-US" => Some(Self::En),
            _ => None,
        }
    }

    pub(crate) fn code(self) -> &'static str {
        match self {
            Self::ZhCn => "zh-CN",
            Self::En => "en",
        }
    }
}

pub(crate) fn set(locale: Locale) {
    *LOCALE.write().unwrap_or_else(|error| error.into_inner()) = locale;
}

pub(crate) fn current() -> Locale {
    *LOCALE.read().unwrap_or_else(|error| error.into_inner())
}

pub(crate) fn translate(text: String) -> String {
    if current() == Locale::ZhCn {
        return text;
    }
    if let Some(path) = text.strip_prefix("ZIP 超过 50 MB，已跳过；保存于 ") {
        return format!("ZIP exceeds 50 MB and was skipped; saved at {path}");
    }
    let value = text;
    for (zh, en) in [
        ("登录状态锁失败", "Login state lock failed"),
        ("仅主窗口可以操作登录会话", "Only the main window can manage login sessions"),
        ("未知登录目标", "Unknown login target"),
        ("请输入有效的系统地址", "Enter a valid system address"),
        ("系统地址无效", "Invalid system address"),
        ("请使用 HTTPS 系统地址，不要包含凭据、查询参数或片段（本机测试可用 HTTP）", "Use an HTTPS address without credentials, query parameters or fragments (HTTP is allowed for local testing)"),
        ("无法关闭旧登录窗口", "Could not close the old login window"),
        ("无法关闭已完成的登录窗口", "Could not close the completed login window"),
        ("无法显示登录窗口", "Could not display the login window"),
        ("登录地址无效", "Invalid login address"),
        ("无法打开 OAuth 登录窗口", "Could not open the OAuth login window"),
        ("老系统 OAuth 登录", "Legacy system OAuth login"),
        ("新系统 OAuth 登录", "New system OAuth login"),
        ("选择 ZIP 工作目录", "Choose ZIP work directory"),
        ("选择处理后的项目 ZIP", "Choose prepared project ZIPs"),
        ("ZIP 项目", "ZIP projects"),
        ("仅主窗口可以选择 ZIP", "Only the main window can select ZIP files"),
        ("文件选择状态不可用", "File selection state is unavailable"),
        ("项目名称数量与所选 ZIP 数量不一致，请重新选择文件", "Project name count does not match the selected ZIP files; select them again"),
        ("请输入有效的项目名称（不能包含路径分隔符）", "Enter a valid project name without path separators"),
        ("ZIP 超过 50 MB，已跳过", "ZIP exceeds 50 MB and was skipped"),
        ("请先在老系统 OAuth 窗口完成登录", "Complete login in the legacy system OAuth window first"),
        ("请先在新系统 OAuth 窗口完成登录", "Complete login in the new system OAuth window first"),
        ("登录 Cookie 格式无效", "Invalid login cookie format"),
        ("未找到项目数据。请确认老系统登录已完成并打开项目首页。", "Project data was not found. Confirm legacy-system login and open the project homepage."),
        ("读取项目任务失败", "Failed to read project task"),
        ("状态锁失败", "State lock failed"),
        ("读取项目失败", "Failed to read projects"),
        ("重试次数已用尽", "Retry attempts exhausted"),
        ("未找到上传凭证，请重新登录新系统", "Upload token not found; log in to the new system again"),
        ("请检查登录状态和权限", "Check login status and permissions"),
        ("工作目录无效", "Invalid work directory"),
        ("未知错误", "Unknown error"),
        ("下载失败", "Download failed"),
        ("上传失败", "Upload failed"),
        ("已尝试", "after"),
        ("次", "attempts"),
        ("可重试", "retryable"),
        ("字节数重置为 0", "bytes reset to 0"),
        ("收到网页而非 ZIP，请重新登录老系统", "received a web page instead of a ZIP; log in to the legacy system again"),
        ("读取 ZIP 内容时出错", "error reading ZIP contents"),
        ("无效的 ZIP，请检查登录状态", "invalid ZIP; check login status"),
        ("无法读取 ZIP", "Unable to read ZIP"),
        ("无法读取 ZIP 属性", "Unable to read ZIP metadata"),
        ("ZIP 路径不是普通文件", "ZIP path is not a regular file"),
        ("ZIP 文件为空", "ZIP file is empty"),
        ("ZIP 超过 50 MB（50,000,000 字节）限制", "ZIP exceeds the 50 MB (50,000,000 byte) limit"),
        ("不是有效的 ZIP 文件", "not a valid ZIP file"),
        ("ZIP 压缩包为空", "ZIP archive is empty"),
        ("上传结果不确定", "Upload result is uncertain"),
        ("请先在新系统确认目标项目是否已创建；若未创建，请使用本地 ZIP 上传。", "Confirm whether the target project was created in the new system; if not, upload the local ZIP."),
        ("新系统未返回有效的导入结果", "The new system did not return a valid import result"),
        ("新系统未确认创建项目", "The new system did not confirm project creation"),
        ("队列中没有这个项目", "This project is not in the queue"),
        ("任务已在运行", "The task is already running"),
        ("已成功或跳过的任务不能重试", "A successful or skipped task cannot be retried"),
        ("任务已暂停，请先继续", "Task is paused; resume it first"),
        ("已达到本批次并发上限", "The batch concurrency limit has been reached"),
        ("已完成的任务不能暂停或继续", "A completed task cannot be paused or resumed"),
        ("正在准备新批次", "Preparing a new batch"),
        ("无效或已过期的批次编号", "Invalid or expired batch ID"),
        ("上一批次尚未结束", "The previous batch has not finished"),
        ("批次编号已用尽", "Batch IDs are exhausted"),
        ("仅主窗口可以操作任务队列", "Only the main window can operate the task queue"),
        ("并发任务数必须在 1 到 3 之间", "Concurrent tasks must be between 1 and 3"),
        ("任务模式与所选项目不一致", "Task mode does not match the selected projects"),
        ("未知任务类型", "Unknown task type"),
        ("没有可执行的项目", "There are no executable projects"),
        ("后台任务异常退出，请先核对目标站的导入结果", "The background task exited unexpectedly; verify the import result on the target site first"),
        ("所选项目已不可用，请刷新列表", "The selected project is unavailable; refresh the list"),
        ("ZIP 状态不可用", "ZIP state is unavailable"),
        ("缺少来源会话", "Source session is missing"),
        ("待处理目录不可用", "Recovery directory is unavailable"),
        ("已跳过", "skipped"),
        ("已确认创建项目", "Project creation confirmed"),
        ("正在下载 ZIP", "Downloading ZIP"),
        ("正在上传 ZIP", "Uploading ZIP"),
        ("ZIP 已读取，等待服务器确认导入", "ZIP read; waiting for server import confirmation"),
    ] {
        if value == zh { return en.to_owned(); }
        if let Some(rest) = value.strip_prefix(zh) {
            if rest.starts_with('：') || rest.starts_with(':') || rest.starts_with('（') {
                return format!("{en}{rest}");
            }
        }
    }
    value
}
