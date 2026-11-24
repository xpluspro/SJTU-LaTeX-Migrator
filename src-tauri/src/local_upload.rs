use super::AppState;
use serde::Serialize;
use std::path::PathBuf;
use tauri::{Manager, Runtime, WebviewWindow};

#[derive(Serialize)]
pub(crate) struct Choice {
    path: PathBuf,
    name: String,
    bytes: u64,
}

#[tauri::command]
pub(crate) async fn pick_local_zip<R: Runtime>(
    window: WebviewWindow<R>,
) -> Result<Option<Vec<Choice>>, String> {
    if window.label() != "main" {
        return Err("仅主窗口可以选择 ZIP".into());
    }
    let Some(handles) = rfd::AsyncFileDialog::new()
        .set_title("选择处理后的项目 ZIP")
        .add_filter("ZIP 项目", &["zip"])
        .pick_files()
        .await
    else {
        return Ok(None);
    };

    let choices: Vec<Choice> = handles
        .into_iter()
        .map(|handle| {
            let path = handle.path().to_owned();
            let bytes = path.metadata().map(|metadata| metadata.len()).unwrap_or(0);
            let name = path
                .file_stem()
                .map(|value| value.to_string_lossy().into_owned())
                .unwrap_or_default();
            Choice { path, name, bytes }
        })
        .collect();
    let paths = choices.iter().map(|choice| choice.path.clone()).collect();
    *window
        .state::<AppState>()
        .local_zip
        .lock()
        .map_err(|_| "文件选择状态不可用")? = paths;
    Ok(Some(choices))
}

fn valid_project_name(name: &str) -> bool {
    if name.chars().any(char::is_control) {
        return false;
    }
    let name = name.trim();
    !name.is_empty() && name != "." && name != ".." && !name.contains(['/', '\\'])
}

pub(super) fn prepare_local<R: Runtime>(
    app: &tauri::AppHandle<R>,
    names: Vec<String>,
) -> Result<Vec<(PathBuf, String)>, String> {
    let state = app.state::<AppState>();
    let mut selected = state.local_zip.lock().map_err(|_| "文件选择状态不可用")?;
    if names.is_empty() || names.len() != selected.len() {
        return Err("项目名称数量与所选 ZIP 数量不一致，请重新选择文件".into());
    }
    if names.iter().any(|name| !valid_project_name(name)) {
        return Err("请输入有效的项目名称（不能包含路径分隔符）".into());
    }
    let paths = std::mem::take(&mut *selected);
    Ok(paths
        .into_iter()
        .zip(names)
        .map(|(path, name)| (path, name.trim().to_owned()))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::valid_project_name;

    #[test]
    fn project_names_reject_paths_and_empty_values() {
        for name in ["", "  ", ".", "..", "a/b", r"a\\b", "a\n"] {
            assert!(!valid_project_name(name), "{name:?}");
        }
        assert!(valid_project_name("project"));
        assert!(valid_project_name("  project  "));
    }
}
