# SJTU LaTeX Migrator

将 SJTU LaTeX 老系统的项目迁移到 LaTeX Pro，支持批量选择项目和断点后补传。

## 下载与安装

前往 [最新版本下载页](https://github.com/xpluspro/SJTU-LaTeX-Migrator/releases/latest)，在 **Assets** 中选择适合你的安装包：

| 电脑 | 下载文件 | 安装方法 |
| --- | --- | --- |
| Mac（M1、M2、M3 等 Apple 芯片） | `macOS-Apple-Silicon.dmg` | 打开文件，将应用拖入「应用程序」。 |
| Windows（64 位 Intel / AMD） | `Windows-x64-Setup.exe` | 双击文件，按提示安装。 |
| Ubuntu / Debian（64 位） | `Linux-x64.deb` | 用软件安装器打开，或运行 `sudo apt install ./文件名.deb`。 |
| 其他 Linux（64 位 Intel / AMD） | `Linux-x64.AppImage` | 运行 `chmod +x 文件名.AppImage`，然后双击或在终端运行该文件。 |

表中是文件名的结尾，完整文件名中还包含版本号。其他架构可以尝试[从源码构建](#从源码构建)。

> [!WARNING]
> 安装包尚未签名，macOS 版本也未经公证。首次打开时系统可能阻止运行或显示安全警告。请确认安装包来自本项目的 Releases 页面。

## 开始迁移

1. 打开应用，分别在老系统和新系统点击「打开 OAuth 登录」，完成 jAccount 登录，等待两侧显示「已连接」。
2. 点击「读取项目列表」，勾选要迁移的项目。
3. 选择 ZIP 工作目录，点击「开始迁移」。应用会下载项目 ZIP 并上传至新系统。
4. 任务结束后查看迁移记录，并到新系统检查项目。

默认同时处理 2 个项目，可将并发数改为 1–3；选择 1 即逐个迁移。任务可以暂停、继续或重试。应用右上角可切换中文和英文。

### 超过 50 MB 的项目

超过 50 MB（50,000,000 字节）的 ZIP 不会自动上传，原文件会保留在工作目录的 `待处理/` 中。任务结束后，将应用提供的提示词交给 AI 工具处理；检查处理后的项目，再使用「批量补传 ZIP」上传。

> [!IMPORTANT]
> 如果上传中断或结果不确定，重试前先到新系统检查项目是否已经创建，以免重复导入。

## 从源码构建

安装 Rust、[Tauri 2 的平台依赖](https://v2.tauri.app/start/prerequisites/)和 Tauri CLI：

```bash
cargo install tauri-cli --version '^2' --locked
```

在仓库根目录运行：

```bash
cd src-tauri
cargo tauri dev    # 开发运行
cargo tauri build  # 构建安装包
```

构建产物位于 `src-tauri/target/release/bundle/`。其他平台和架构需满足 Tauri 的构建要求，未经本项目发布流程验证。
