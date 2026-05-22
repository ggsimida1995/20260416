# Rust + Tauri 重构说明

## 当前状态

当前根目录已新增 Tauri 重构项目：

- `web/`：Vite + TypeScript 前端。
- `src-tauri/`：Rust 后端、SQLite、文件扫描、比对、导出。
- `web/package.json`：Tauri/Vite 前端构建入口。

已验证：

```bash
cargo check --manifest-path src-tauri/Cargo.toml
npm --prefix web run typecheck
npm --prefix web run build
npm --prefix web audit
npm --prefix web run tauri:build
```

当前 macOS 产物：

- `src-tauri/target/release/bundle/macos/项目资料比对助手.app`
- `src-tauri/target/release/bundle/dmg/项目资料比对助手_0.1.0_aarch64.dmg`

## 已迁移能力

- 设置持久化到 SQLite。
- 运行日志、成功/失败记录写入 SQLite。
- XLS/DOC/PDF 提取结果写入 SQLite 缓存，文件未变化时直接复用。
- “清理缓存”只清理提取缓存，不删除运行日志、比对记录和待导出成功数据。
- 设置里的保存目录直接作为项目目录扫描，不再追加 `project` 子目录。
- `.xlsx/.xls` 读取，使用 `calamine`。
- `.docx` 原生文本读取。
- `.doc` 通过 `textutil`、`antiword`、`soffice` 兜底读取。
- PDF 文本层提取，使用 bundled PDFium。
- PDF 签字区裁剪、远程 AI/OCR 识别、红章检测已迁移到 Rust，PDF 渲染使用 bundled PDFium。
- 下载流程已接入 Rust/Tauri 命令，当前 macOS 版读取本机 Chrome Cookie 后下载附件到设置里的保存目录。
- 本地字段比对。
- 中文姓名拼音近似匹配。
- 比对成功先写入 SQLite，导出时批量写入成功台账。
- 比对失败从 SQLite 导出文本记录。
- Tauri `.app/.dmg` 打包。

## 尚未完全等价的能力

- 下载已支持 macOS Chrome Cookie；Windows Chrome `v10/v11` Cookie 已补基础 DPAPI/AES-GCM 解密，`v20` App-Bound Cookie 会提示不可直接解密，仍需 Windows 实机验证。
- PDFium 已随 Rust 产物打包，但仍需用真实扫描 PDF 验证裁剪区域和红章阈值。

旧 Python 版本已移除，当前以 Rust + Tauri 为唯一主线。

## 构建

```bash
npm --prefix web install
npm --prefix web run tauri:build
```

开发模式：

```bash
npm --prefix web run tauri:dev
```
