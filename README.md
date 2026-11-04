# fullme

右上角 **320×160** 多页签小浏览器命令行工具（Tauri 实现，macOS，页签栏 24px + 内容区 320×136）。

```
fullme [url]...
```

- **命令立即返回**：`fullme` 会 spawn 一个独立守护子进程（全新 exec，脱离终端）承载 GUI，
  启动器在窗口就绪后马上返回，行为与 `open [url]` 一致 ——
  同步等待命令结束的调用方（终端 shell、TinTin++ `#system` 等）不会被卡住
- 首次执行：打开置于屏幕右上角（避开菜单栏/Dock）的 320×160 置顶小窗口，并为每个 URL 创建页签
- **不抢焦点**：打开时窗口照常出现，但不激活本应用 —— 前台与键盘焦点保持在调用方（如 TinTin++）
- 重复执行：进程通过单实例转发给已运行实例，**追加为新页签**并把窗口带到前台
- 不带 URL：仅聚焦窗口（首次启动则打开一个空白页签）
- **页签池**：默认容量 3 个，FIFO —— 池满时自动关闭最旧页签，窗口内始终不超过 3 个页签
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

- 顶部 24px 页签栏：点击切换页签，`×` 或右键关闭页签，`⟳` 刷新，`✕` 退出应用
- 左侧 `⠿` 为拖拽把手，可拖动窗口位置
- 页面内容区为真实 webview（非 iframe），不受 X-Frame-Options 限制
- 页签标题实时跟随页面 `document.title`

## 实现要点

| 需求 | 实现 |
| --- | --- |
| 多页签 | Tauri v2 `unstable` 多 webview：`Window::add_child` 每个页签一个子 webview，`show/hide` 切换 |
| 命令不阻塞 | 启动器 spawn 全新子进程承载 GUI（`FULLME_DAEMON=1`，Unix: exec / Windows: DETACHED_PROCESS），经 stdout 管道握手就绪后返回；Unix 用 flock 启动锁、Windows 靠插件命名互斥锁消除首启竞态。Windows 侧 spawn 前还须摘除启动器 stdio 的 `HANDLE_FLAG_INHERIT`，否则守护进程会连带复制调用方（WSL interop / 管道）的句柄，调用方要等 GUI 退出才读到 EOF（表现为命令不返回） |
| 单实例转发 | `tauri-plugin-single-instance`（macOS Unix socket），第二实例把 argv 转发后退出 |
| 页签池 FIFO | Rust 侧 `TabManager`（Vec 保序），池满 `remove(0)` 关闭最旧页签 |
| 右上角定位 | `Monitor::work_area()` 计算物理坐标，`set_position` + `set_always_on_top` |
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
        ├── main.rs      # CLI 解析、单实例、窗口定位
        └── tabs.rs      # 页签池、子 webview 管理、IPC 命令
```
