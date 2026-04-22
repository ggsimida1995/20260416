# Project File Compare

`pywebview` 桌面工具，用于复用本机已登录的 Chrome Hollysys 会话，批量下载项目关闭资料，并在本地完成文件比对与台账写入。

## 本地运行

```bash
python3 -m venv .venv
source .venv/bin/activate
pip install -r requirements.txt
python3 main.py
```

## Windows 打包

1. 在 Windows 机器上安装 Python 3.9+。
2. 安装依赖：

```bat
py -m pip install -r requirements.txt
py -m pip install pyinstaller
```

3. 运行打包脚本：

```bat
build_exe.bat
```

4. 打包定义固定在仓库内的 `ProjectFileCompare.spec`，`build_exe.bat` 只是一个包装入口。
5. 产物位于 `dist\ProjectFileCompare.exe`。
6. 如果要区分架构，先切到对应 Python 架构环境再打包：
   `x86` 用 32 位 Python
   `x64` 用 64 位 Python

## GitHub Actions 打包

如果你当前在 macOS，但要产出 Windows 包：

1. 把代码推到 GitHub。
2. 打开 `Actions`。
3. 运行 `Build Windows Package`。
4. 构建完成后直接下载产物：
   `ProjectFileCompare-exe-x86`
   `ProjectFileCompare-exe-x64`
   `ProjectFileCompare-installer-x86`
   `ProjectFileCompare-installer-x64`

## Windows ARM64

当前仓库还不能稳定产出原生 `arm64` Windows 包。

原因：
- GitHub Actions 已支持 `windows-11-arm`
- `actions/setup-python` 也支持 `arm64`
- 但当前依赖里的 `PyMuPDF` 暂时没有可用的 `win_arm64` 轮子，原生 ARM 打包会在装依赖阶段失败

如果要出原生 `arm64` 包，需要先处理这条依赖：
- 换掉 `PyMuPDF`
- 或者单独维护 ARM64 可安装方案

## Windows 安装包

如果你希望给别人发安装包，而不是直接发裸 `exe`：

1. 先安装 Inno Setup 6。
2. 运行：

```bat
build_installer.bat
```

3. 安装脚本在 `installer\ProjectFileCompare.iss`。
4. 安装包产物会输出到 `installer\Output\ProjectFileCompare-Setup.exe`。

## Windows 运行前提

- 建议安装 Microsoft Edge WebView2 Runtime。`pywebview` 在 Windows 下优先使用 Edge Chromium 渲染内核。
- 程序会优先读取本机 Chrome 用户数据目录中的 `Default` 和 `Profile N` 配置，自动查找 Hollysys Cookie。
- 当前版本已支持 Windows 的 `Local State + DPAPI/AES-GCM` Cookie 解密链路。
- 打包版默认会把运行配置、处理记录、调试汇总和默认资料目录放到 `%LOCALAPPDATA%\ProjectFileCompare`，不会再写回安装目录。

## Windows 首次运行检查

第一次在 Windows 上打开，建议按这个顺序自检：

1. 先确认 Chrome 已经登录过 Hollysys，并且能直接打开 [待办页](https://www.hollysys.net/sys/aggregation/)。
2. 程序首页会先自动跑一轮 `启动自检`，直接显示桌面内核、运行目录、资料目录和 Hollysys 会话状态。
3. 如果状态是 `未检测到`，先看界面里的 `Cookie DB` 路径是否落在你实际使用的 Chrome 配置目录。
4. 如果状态是 `需要重登`，直接回 Chrome 里重新登录 Hollysys，再回程序点一次 `刷新会话`。
5. 如果状态是 `检测失败`，优先看 `结果` 文案；大多数问题都会直接写在这里。
6. 通过后再选保存目录，并分别测试一次 `下载资料` 和 `本地比对`。

## 常见排障

- `Cookie 解密失败`
  先确认当前 Windows 用户就是打开 Chrome 的那个用户，并且 Chrome 里仍能直接进入 Hollysys。
- `App-Bound Encryption（v20）`
  当前 Chrome 会话启用了新的应用绑定加密，外部程序无法直接复用这份 Cookie。这不是程序界面问题，而是浏览器加密策略限制。
- `未检测到`
  一般是没在这个 Chrome 配置里登录过 Hollysys，或者实际登录的是别的浏览器/别的 Windows 用户。
- `需要重登`
  Cookie 还在，但 Hollysys 服务端会话已经过期。

## Hollysys 会话限制

- 程序不会替你登录 Hollysys，只会复用当前机器上已经登录过的 Chrome 会话。
- 如果界面提示 `Cookie 解密失败`，先确认 Chrome 里能正常打开 [Hollysys 待办页](https://www.hollysys.net/sys/aggregation/)。
- 如果错误里出现 `App-Bound Encryption（v20）`，说明当前 Chrome 会话启用了新的应用绑定加密，外部程序不能直接复用这份 Cookie。这个场景下需要改为可读的浏览器配置，或改用“浏览器内登录/自动化控制浏览器”的方案。

## 为什么打包脚本显式排除 Qt

这套桌面 UI 已经迁移到 `pywebview`，不再依赖 `PySide6/Qt`。`ProjectFileCompare.spec` 里显式排除了 Qt 相关模块，目的是避免历史环境里残留的 Qt 依赖被 PyInstaller 一并打进安装包，导致体积变大或运行时混入旧链路。

## 验证

```bash
python3 -m pytest -q
```
