use super::*;
use crate::locale;
use std::collections::HashMap;

pub(super) enum WorkItem {
    Local {
        path: PathBuf,
        name: String,
    },
    Migration {
        project: Project,
        root: PathBuf,
        ready_zip: Mutex<Option<PathBuf>>,
    },
}
impl WorkItem {
    pub(super) fn name(&self) -> &str {
        match self {
            Self::Local { name, .. } => name,
            Self::Migration { project, .. } => &project.name,
        }
    }
}

pub(super) fn prepare_items<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    mode: &str,
    names: Vec<String>,
    input: MigrateInput,
) -> Result<(Option<Session>, Session, Vec<WorkItem>), String> {
    let target = Session::snapshot(app, false)?;
    match mode {
        "local" => {
            let items = local_upload::prepare_local(app, names)?
                .into_iter()
                .map(|(path, name)| WorkItem::Local { path, name })
                .collect();
            Ok((None, target, items))
        }
        "migration" => {
            let source = Session::snapshot(app, true)?;
            let mut projects: HashMap<_, _> = source
                .projects()?
                .into_iter()
                .map(|p| (p.id.clone(), p))
                .collect();
            let workdir = PathBuf::from(input.workdir);
            let root = if let Ok(relative) = workdir.strip_prefix("~") {
                app.path()
                    .home_dir()
                    .map_err(|e| e.to_string())?
                    .join(relative)
            } else {
                workdir
            };
            fs::create_dir_all(&root).map_err(|e| e.to_string())?;
            let items = input
                .project_ids
                .iter()
                .map(|id| {
                    let project = projects.remove(id).ok_or("所选项目已不可用，请刷新列表")?;
                    Ok(WorkItem::Migration {
                        project,
                        root: root.clone(),
                        ready_zip: Mutex::default(),
                    })
                })
                .collect::<Result<Vec<_>, String>>()?;
            Ok((Some(source), target, items))
        }
        _ => Err("未知任务类型".into()),
    }
}

pub(super) fn process_item<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    source: Option<&Session>,
    target: &Session,
    item: &WorkItem,
    ctx: &queue::Context,
) -> Result<queue::Outcome, String> {
    let name = item.name();
    let event = |message: String, stage: Option<&str>| TaskEvent {
        batch_id: ctx.batch_id.clone(),
        kind: "progress".into(),
        project: Some(name.to_owned()),
        index: Some(ctx.id + 1),
        total: Some(ctx.total),
        message: locale::translate(message),
        status: None,
        path: None,
        stage: stage.map(str::to_owned),
    };
    let stage = |stage_name: &str, message: &str| {
        emit_event(app, event(message.to_owned(), Some(stage_name)))
    };
    ctx.checkpoint();
    let path = match item {
        WorkItem::Local { path, .. } => {
            let size = fs::metadata(path).map_err(|e| locale::translate(format!("无法读取 ZIP 属性：{e}")))?.len();
            if size > MAX_ZIP_BYTES {
                return Ok(queue::Outcome { status: "skipped".into(), message: locale::translate("ZIP 超过 50 MB，已跳过".into()), path: Some(path.display().to_string()) });
            }
            path.clone()
        },
        WorkItem::Migration {
            project,
            root,
            ready_zip,
        } => {
            let saved = ready_zip.lock().map_err(|_| locale::translate("ZIP 状态不可用".into()))?.clone();
            if let Some(path) = saved.filter(|p| p.is_file()) {
                path
            } else {
                stage("download", "正在下载 ZIP");
                // Reserve a unique directory atomically, including for same-name concurrent exports.
                let export = tempfile::Builder::new()
                    .prefix("export-")
                    .tempdir_in(root)
                    .map_err(|e| e.to_string())?;
                let path = export
                    .path()
                    .join(format!("{}.zip", handoff::safe_filename(name)));
                let size = download_zip(app, source.ok_or_else(|| locale::translate("缺少来源会话".into()))?, project, &path, ctx)?;
                let _ = export.keep();
                if size > MAX_ZIP_BYTES {
                    static HANDOFF_LOCK: Mutex<()> = Mutex::new(());
                    let _guard = HANDOFF_LOCK.lock().map_err(|_| locale::translate("待处理目录不可用".into()))?;
                    let dest = handoff::save_for_manual_recovery(root, name, &path, "ZIP 超过 50 MB")?;
                    return Ok(queue::Outcome {
                        status: "skipped".into(),
                        message: locale::translate(format!("ZIP 超过 50 MB，已跳过；保存于 {}", dest.display())),
                        path: Some(dest.display().to_string()),
                    });
                }
                *ready_zip.lock().map_err(|_| locale::translate("ZIP 状态不可用".into()))? = Some(path.clone());
                path
            }
        }
    };
    ctx.checkpoint();
    stage("upload", "正在上传 ZIP");
    let result = upload_zip(
        app,
        target,
        &path,
        name,
        event("ZIP 已读取，等待服务器确认导入".into(), Some("upload")),
        ctx,
    );
    let (status, message) = match result {
        Ok(()) => ("success", locale::translate("已确认创建项目".into())),
        Err(e) => ("failed", locale::translate(e)),
    };
    Ok(queue::Outcome {
        status: status.into(),
        message,
        path: Some(path.display().to_string()),
    })
}
