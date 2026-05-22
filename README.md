# 项目资料比对助手

Rust + Tauri 桌面工具，用于复用本机已登录的 Chrome 会话，批量下载项目关闭资料，并在本地完成资料比对、SQLite 记录和结果导出。

## 运行目录

程序使用设置里的保存目录作为业务数据目录。目录结构如下：

```text
file/
  <项目编号>/ # 下载和本地比对扫描的项目资料
  success/   # 成功台账模板
  export/    # 失败记录导出
  cache/     # SQLite 提取缓存、日志、比对记录
```

全局设置持久化在系统应用数据目录的 SQLite 中，不再使用 Python 配置文件。
设置里选择的保存目录就是项目目录，程序不会再自动追加 `project` 子目录。

## 本地开发

```bash
npm --prefix web install
npm --prefix web run tauri:dev
```

## 本地构建

```bash
npm --prefix web run build
cargo check --manifest-path src-tauri/Cargo.toml
npm --prefix web run tauri:build
```

macOS 产物示例：

```text
src-tauri/target/release/bundle/macos/项目资料比对助手.app
src-tauri/target/release/bundle/dmg/项目资料比对助手_0.1.0_aarch64.dmg
```

## GitHub Actions 打包

打 `v*` tag（如 `v0.1.2`）触发 Release workflow，矩阵同时产出 Windows x64 / Windows arm64 / macOS Apple Silicon 三份产物并写入同一个 GitHub Release，自动更新（`latest.json`）覆盖全部三个平台。也可在 Actions 页面 `workflow_dispatch` 手动运行。

## macOS 首次启动

`.dmg` 未做 Apple 公证，首次启动会被 Gatekeeper 拦截。任选其一绕过：

- **右键打开**：Finder 里右键 `.app` → 「打开」→ 弹窗里再点「打开」。
- **去隔离属性**（终端）：

  ```bash
  xattr -dr com.apple.quarantine /Applications/ProjectFileCompare.app
  ```

确认信任后，后续自动更新无需再操作。

## 功能状态

- 设置、运行日志、成功/失败记录写入 SQLite。
- XLS/XLSX、DOC/DOCX、PDF 读取和提取缓存已迁移到 Rust。
- PDF 渲染、签字区裁剪、红章检测使用 bundled PDFium。
- 比对成功先写入 SQLite，用户点击导出时再批量写入成功台账。
- 比对失败从 SQLite 导出文本记录。
- 下载流程已迁移到 Rust，macOS Chrome Cookie 已接入；Windows Cookie 解密仍建议用真实机器复测。

## 验证

```bash
npm --prefix web run typecheck
npm --prefix web run build
cargo test --manifest-path src-tauri/Cargo.toml
```
