# fullme

右下角 **320×160** 多页签小浏览器命令行工具（Tauri 实现，macOS，页签栏 24px + 内容区 320×136）。

```
fullme [url]...
```

- **命令立即返回**：`fullme` 会 spawn 一个独立守护子进程（全新 exec，脱离终端）承载 GUI，
  启动器在窗口就绪后马上返回，行为与 `open [url]` 一致 ——
  同步等待命令结束的调用方（终端 shell、TinTin++ `#system` 等）不会被卡住
- 首次执行：打开置于屏幕右下角（避开菜单栏/Dock）的 320×160 置顶小窗口，并为每个 URL 创建页签
- **不抢焦点**：打开时窗口照常出现，但不激活本应用 —— 前台与键盘焦点保持在调用方（如 TinTin++）
- 重复执行：进程通过单实例转发给已运行实例，**追加为新页签**并把窗口带到前台
- 不带 URL：仅聚焦窗口（首次启动则打开一个空白页签）
- **页签池**：默认容量 3 个，FIFO —— 池满时自动关闭最旧页签，窗口内始终不超过 3 个页签
- **缩小/恢复**：标题栏 `─` 缩小（仅保留 24px 标题栏）、`⤢` 恢复（320×160），两种尺寸都重新贴住屏幕右下角；
  正常大小空闲 3 分钟自动缩小、缩小持续 20 分钟自动恢复（如此循环）；**新建页签或点击刷新时自动恢复正常**并重新起算 3 分钟
- 运行日志：`~/Library/Logs/fullme.log`（`tail -f` 查看）

## 构建

需要 Rust 工具链（无需 Node，前端为纯静态文件并被编译进二进制）：

```bash
cd src-tauri
cargo build --release
```

产物：`src-tauri/target/release/fullme`

安装到 PATH（任选其一）：

```bash
cp src-tauri/target/release/fullme ~/.local/bin/     # 或 /usr/local/bin
```

### Windows 构建

代码已做跨平台适配（`cfg(windows)` 分支），需在 **Windows 实机**构建：

前置（一次性）：

1. [rustup](https://rustup.rs) 安装 Rust（默认 MSVC 工具链，自带 `x86_64-pc-windows-msvc` target）
2. Visual Studio 2022 Build Tools：勾选「使用 C++ 的桌面开发」（MSVC 链接器 + Windows SDK，提供 `rc.exe`）
3. WebView2 Runtime（Win10/11 通常已自带）

不需要 Node（前端为纯静态文件，直接 cargo 构建）。

```powershell
cd fullme\src-tauri
cargo build --release
# 产物：target\release\fullme.exe（release 不弹控制台黑框）

copy target\release\fullme.exe %USERPROFILE%\bin\   # 放到任一 PATH 目录
```

行为差异（相对 macOS）：

- 日志：`%LOCALAPPDATA%\fullme.log`
- 单实例：命名互斥锁（`CreateMutexW` 原子创建，天然无并发竞态，无需启动锁）
- 不抢焦点：Windows 后台进程本就无法抢占前台，无需额外开关
- 资源图标：`icons/icon.ico` 已生成并随 exe 嵌入

**WSL 中调用**：exe 放到 Windows PATH 后，WSL 内可直接 interop 调用：

```bash
fullme.exe https://www.baidu.com   # GUI 出现在 Windows 桌面；单实例互斥锁为会话级全局，转发正常
```

也可以在 WSL 侧装一个 `fullme` 入口（任意目录直接敲 `fullme`）：

```bash
mkdir -p ~/.local/bin ~/.local/share/fullme
cp src-tauri/target/x86_64-pc-windows-msvc/release/fullme.exe ~/.local/share/fullme/
printf '#!/bin/sh\nexec "$HOME/.local/share/fullme/fullme.exe" "$@"\n' > ~/.local/bin/fullme
chmod +x ~/.local/bin/fullme   # ~/.local/bin 已由默认 .profile/.bashrc 加入 PATH
```

### Windows 交叉构建（WSL/Linux，无需 Windows 实机）

在 WSL 里直接产出 `fullme.exe`（已验证配方）：

```bash
# 1) Rust + MSVC 目标
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal --no-modify-path
source ~/.cargo/env && rustup target add x86_64-pc-windows-msvc

# 2) cargo-xwin（自动下载 MSVC CRT/SDK）+ llvm-rc（资源编译）+ clang（预处理），免 root 解包
cargo install cargo-xwin
mkdir -p ~/opt/bin ~/opt/llvm-20 && cd /tmp
apt-get download llvm-20 clang-20 libclang-cpp20 libclang1-20 libclang-common-20-dev
for d in *.deb; do dpkg-deb -x "$d" ~/opt/llvm-20; done
ln -sf ~/opt/llvm-20/usr/lib/llvm-20/bin/{llvm-rc,clang,llvm-ar,llvm-lib} ~/opt/bin/

# 3) 构建
cd src-tauri
PATH="$HOME/.cargo/bin:$HOME/opt/bin:$PATH" \
  cargo xwin build --release --target x86_64-pc-windows-msvc
# 产物：target/x86_64-pc-windows-msvc/release/fullme.exe（PE32+ GUI，图标/版本资源已嵌入）
```

注意：`cargo xwin build` **必须显式带 `--target x86_64-pc-windows-msvc`** ——
不带 target 会按主机平台构建（走到 gtk/glib 依赖上失败）；资源编译器 `llvm-rc`
与 C 预处理器 `clang` 需在 PATH 中（embed-resource 对 msvc 目标在非 Windows 上默认找 `llvm-rc`）。

（可选）在 macOS 上做 Windows 目标的编译检查——需一个 `llvm-rc` 桩让 tauri-winres 的
build script 通过（真实构建在 Windows 上走系统 `rc.exe`）：

```bash
rustup target add x86_64-pc-windows-msvc
# llvm-rc 桩：识别为 "Resource Converter"，/fo 时生成占位文件即可
# 然后：
PATH="<桩目录>:$PATH" CC_x86_64_pc_windows_msvc=/usr/bin/clang \
  cargo check --target x86_64-pc-windows-msvc
```

## 使用

```bash
fullme https://www.baidu.com          # 打开百度
fullme https://www.bing.com           # 追加 Bing 页签
fullme https://news.ycombinator.com   # 追加第 3 个页签
fullme https://tauri.app              # 池满，FIFO 关闭最旧的百度页签
fullme                                # 只聚焦窗口，不新增页签
fullme --help                         # 帮助
```

URL 可省略 scheme（自动补 `https://`），也支持 `about:` / `file:` / `data:` 等。

## 界面

- 顶部 24px 页签栏：点击切换页签，`×` 或右键关闭页签，`─` 缩小 / `⤢` 恢复窗口，`⟳` 刷新，`✕` 退出应用
- 左侧 `⠿` 为拖拽把手，可拖动窗口位置
- **缩小模式**：窗口缩到 320×24（仅标题栏），内容区被裁掉但页签照常加载，且仍贴住右下角；
  自动循环：正常空闲 **3 分钟** → 缩小，缩小持续 **20 分钟** → 恢复（调试可用环境变量
  `FULLME_IDLE_SECS=<秒>` / `FULLME_COMPACT_SECS=<秒>` 分别覆盖两个超时），
  新建页签、点击刷新、手动 `⤢` 都会立即恢复 320×160 并重新起算 3 分钟
- 页面内容区为真实 webview（非 iframe），不受 X-Frame-Options 限制
- 页签标题实时跟随页面 `document.title`

## 实现要点

| 需求 | 实现 |
| --- | --- |
| 多页签 | Tauri v2 `unstable` 多 webview：`Window::add_child` 每个页签一个子 webview，`show/hide` 切换 |
| 命令不阻塞 | 启动器 spawn 全新子进程承载 GUI（`FULLME_DAEMON=1`，Unix: exec / Windows: DETACHED_PROCESS），经 stdout 管道握手就绪后返回；Unix 用 flock 启动锁、Windows 靠插件命名互斥锁消除首启竞态。Windows 侧 spawn 前还须摘除启动器 stdio 的 `HANDLE_FLAG_INHERIT`，否则守护进程会连带复制调用方（WSL interop / 管道）的句柄，调用方要等 GUI 退出才读到 EOF（表现为命令不返回） |
| 单实例转发 | `tauri-plugin-single-instance`（macOS Unix socket），第二实例把 argv 转发后退出 |
| 页签池 FIFO | Rust 侧 `TabManager`（Vec 保序），池满 `remove(0)` 关闭最旧页签 |
| 右下角定位 | `Monitor::work_area()` 计算物理坐标，`x = 右边框 - 宽`、`y = 下边框 - 高`，`set_position` + `set_always_on_top`；**每次缩小/恢复后重新贴角**（高度随模式变化 160↔24，只改尺寸不改位置会让窗口悬空），优先贴窗口当前所在显示器，失败退回主屏 |
| 缩小/恢复 | `window_mode.rs`：`set_size` 在 320×160 / 320×24（仅标题栏）间切换；状态只记 `模式 + since（该模式进入时刻）`，`Mutex+Condvar` 后台线程按 `since + 当前模式超时`（正常 180s / 缩小 1200s）算 deadline **循环切换**，任一 `set_mode` 都会刷新 `since` 并 notify 重算 —— 新建页签（`add_tab`）、刷新（`reload_active`）会恢复并重新起算，前端经 `window-mode-updated` 事件切换 `─`/`⤢` 图标 |
| 前端同步 | Rust 持有状态，`tabs-updated` 事件推送快照，前端 `invoke` 切换/关闭 |

> 实现注意：不能在加载了 AppKit/WebKit 的进程里 `fork` 后继续跑 Tauri ——
> fork 污染会导致主线程 `CFRunLoop` 派发失效（`add_child` 永久卡死），
> 必须用 `Command::spawn` + exec 出全新进程。调试可用 `FULLME_FOREGROUND=1` 跳过守护。
>
> 实现注意（跨平台）：`TAB_MANAGER` 锁**不可**跨 `add_child/show/hide/close/reload` 持有 ——
> WebView2 创建期间 wry 经 `wait_with_pump` 泵消息，前端 `get_tabs` IPC 会重入同锁，
> 同线程重入 `std::Mutex` 即永久自锁（症状：setup 卡在 `add_child`、事件循环停摆、
> 单实例转发 10s 超时）。页签操作一律「锁内改数据、锁外碰 webview」。

## 目录结构

```
fullme/
├── frontend/            # 页签栏 UI（无打包器，withGlobalTauri 全局 API）
│   ├── index.html
│   ├── styles.css
│   └── app.js
└── src-tauri/
    ├── tauri.conf.json  # 窗口 320x160、无边框、置顶、不抢焦点
    ├── capabilities/    # 权限（事件、窗口拖拽）
    └── src/
        ├── main.rs         # CLI 解析、单实例、窗口定位
        ├── tabs.rs         # 页签池、子 webview 管理、IPC 命令
        └── window_mode.rs  # 窗口正常/缩小模式 + 3min/20min 循环切换线程
```
