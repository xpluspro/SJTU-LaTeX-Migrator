# SJTU LaTeX Migrator

基于 **Tauri 2 + Rust** 的本地桌面迁移工具。不是 Tkinter，也不依赖 Playwright、Chrome Relay 或系统 Chrome Cookie。

## 流程

1. 在应用内点击“打开 OAuth 登录”，分别打开：
   - 老系统 OAuth：`https://latex.sjtu.edu.cn/jaccountlogin`
   - 新系统 OAuth：`https://latexpro.sjtu.edu.cn/oidc/login`
2. 在内置窗口完成 jAccount 登录，等待网页回到对应站点。工具自动确认会话；主窗口显示“已连接”，不用手动读取或复制 Cookie。
3. 如果在回跳前关掉了登录窗口，请点击对应站点的登录按钮重试。
4. 读取老系统项目列表。
5. 勾选需要迁移的项目。
6. 指定工作目录，逐个下载项目 ZIP。自动迁移和本地补传均按单个 ZIP 的实际文件大小执行 50 MB（50,000,000 字节）上限，等于上限允许上传。
7. 超过 50 MB 的 ZIP 直接跳过并保留，不进行自动压缩。任务完成后统一列出超限项目及可复制给 Agent 的处理提示词。

默认并发 2 个项目，可选 1–3；选 1 为串行。每行独立展示项目、阶段、传输字节与结果。下载和上传可以逐项或全部暂停、继续；暂停中的活动连接仍占用并发名额。已提交到服务端的导入不能撤回，网络等待时间也受连接超时限制。
任务结束后可点击“全部清除”移除当前进度、日志和提示词；不会删除 ZIP 或其他文件。清除后不能从原队列直接重试，可重新选择保留的 ZIP 补传。任务运行期间不能清除。


默认服务地址写在 `src-tauri/src/lib.rs` 的常量中，同时 UI 允许修改；后续换站点可以直接改默认值并发布新版本。

### 断线与手动处理

- 短暂断线时，下载会退避重试，从文件开头重新下载；不会把半个 ZIP 当成成功。上传只有在确认尚未建立连接时才会自动重发。若请求可能已到达新系统但未收到成功回执，请先检查新系统项目列表，避免重复创建。
- 自动迁移中超限的 ZIP 保存在工作目录下的 `待处理/`，本地补传的超限文件保留原位。将任务结束后的提示词交给 Agent，处理后检查 LaTeX 引用并重新打包到 50 MB 以内。
- 在“批量补传 ZIP”一次多选文件，确认项目名称和并发数后开始。只需新系统登录。每项单独校验大小和导入回执；单项重试和全部重试仅针对失败项。重试前请先核对目标站，避免重复创建。

## 构建环境

需要：

- Rust stable / Cargo
- Tauri CLI 2：`cargo install tauri-cli --version '^2' --locked`
- macOS 的 Xcode Command Line Tools；Windows 的 WebView2；Linux 的 WebKitGTK

## 运行与构建

在仓库根目录执行：

```bash
(cd src-tauri && cargo tauri dev)
(cd src-tauri && cargo tauri build)
```

也可以使用：

```bash
cargo fmt --manifest-path src-tauri/Cargo.toml
cargo check --manifest-path src-tauri/Cargo.toml
```

构建产物位于 `src-tauri/target/release/bundle/`。

## 接口说明

Rust 后端使用登录 Webview 的 Cookie 创建 `reqwest` 会话：

- 项目列表：`GET /project`，解析 `ol-prefetchedProjectsBlob` 或 `ol-projects`
- 老项目下载：`GET /project/:projectId/download/zip`
- 新项目导入：`POST /project/new/upload`，multipart 字段 `qqfile` 与 `name`；收到成功回执及新项目 ID 后才计为成功。

老、新系统均为 Overleaf-compatible 部署；实际验证的 OAuth 路由为：

- 老系统：`GET /jaccountlogin`，打开后跳转到上海交通大学统一身份认证页面
- 新系统：`GET /oidc/login`，打开后跳转到上海交通大学统一身份认证页面

导入接口属于站点实际页面使用的非公开接口；若后端升级造成接口变化，应根据新系统页面 Network 请求更新 `upload_zip`。

## 目录

- `src/index.html`：现代化单页 UI
- `src/styles.css`：布局、色彩与组件样式
- `src/app.js`：前端状态、Tauri command 调用和迁移事件展示
- `src-tauri/src/lib.rs`：OAuth Webview、Cookie、项目 API、ZIP 下载和上传
- `src-tauri/tauri.conf.json`：Tauri 2 窗口与权限配置
