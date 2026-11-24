use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

pub(crate) const HANDOFF_DIR: &str = "待处理";

/// Return a path that keeps the requested basename while never replacing an existing file.
pub(crate) fn unique_path(dir: &Path, filename: &str) -> PathBuf {
    let requested = Path::new(filename);
    let stem = requested
        .file_stem()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty())
        .unwrap_or("项目");
    let extension = requested
        .extension()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty())
        .map(|value| format!(".{value}"))
        .unwrap_or_default();
    let first = dir.join(format!("{stem}{extension}"));
    if !collides(dir, &first) {
        return first;
    }
    let mut number = 2_u32;
    loop {
        let candidate = dir.join(format!("{stem} ({number}){extension}"));
        if !collides(dir, &candidate) {
            return candidate;
        }
        number = number.saturating_add(1);
    }
}

fn collides(dir: &Path, candidate: &Path) -> bool {
    let Some(name) = candidate.file_name().and_then(|value| value.to_str()) else {
        return true;
    };
    let wanted = name.to_lowercase();
    fs::read_dir(dir)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .filter_map(|entry| entry.file_name().into_string().ok())
        .any(|existing| existing.to_lowercase() == wanted)
}

pub(crate) fn safe_filename(value: &str) -> String {
    let result: String = value
        .chars()
        .map(|character| {
            if character.is_control() || "/\\:*?\"<>|".contains(character) {
                '_'
            } else {
                character
            }
        })
        .collect();
    let trimmed = result.trim_matches([' ', '.']);
    if trimmed.is_empty() || trimmed == "." || trimmed == ".." {
        "项目".into()
    } else {
        trimmed.to_owned()
    }
}

pub(crate) fn save_for_manual_recovery(
    workdir: &Path,
    project: &str,
    source_zip: &Path,
    reason: &str,
) -> Result<PathBuf, String> {
    let directory = workdir.join(HANDOFF_DIR);
    fs::create_dir_all(&directory).map_err(|error| format!("创建待处理目录失败：{error}"))?;
    let filename = format!("{}.zip", safe_basename(project));
    let destination = unique_path(&directory, &filename);
    let mut destination_file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&destination)
        .map_err(|error| format!("保存待处理 ZIP 失败：{error}"))?;
    let mut source_file =
        fs::File::open(source_zip).map_err(|error| format!("读取待处理 ZIP 失败：{error}"))?;
    if let Err(error) = io::copy(&mut source_file, &mut destination_file) {
        let _ = fs::remove_file(&destination);
        return Err(format!("保存待处理 ZIP 失败：{error}"));
    }
    if let Err(error) = destination_file.flush() {
        let _ = fs::remove_file(&destination);
        return Err(format!("保存待处理 ZIP 失败：{error}"));
    }

    let summary = directory.join("迁移说明.txt");
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&summary)
        .map_err(|error| format!("写入待处理说明失败：{error}"))?;
    writeln!(
        file,
        "项目：{project}\n原因：{reason}\n本地 ZIP：{}\n原始 ZIP：{}\n处理方式：处理后打开迁移工具，点击“批量补传 ZIP”选择 ZIP；上传结果不确定时不要自动重试，先在目标站核对项目列表。\n---",
        destination.display(),
        source_zip.display(),
    )
    .map_err(|error| format!("写入待处理说明失败：{error}"))?;
    Ok(destination)
}

fn safe_basename(value: &str) -> String {
    let result: String = value
        .chars()
        .map(|character| {
            if character.is_control() || "/\\:*?\"<>|".contains(character) {
                '_'
            } else {
                character
            }
        })
        .collect();
    let trimmed = result.trim_matches([' ', '.']);
    if trimmed.is_empty() || trimmed == "." || trimmed == ".." {
        "项目".into()
    } else {
        trimmed.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::unique_path;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn unique_paths_keep_basename_without_overwriting() {
        let directory = tempdir().unwrap();
        let first = unique_path(directory.path(), "项目.zip");
        fs::write(&first, b"existing").unwrap();
        let second = unique_path(directory.path(), "项目.zip");
        assert_eq!(
            second.file_name().unwrap().to_str().unwrap(),
            "项目 (2).zip"
        );
    }
}
