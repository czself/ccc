# TinyVim — 零配置终端 IDE

即下即用，打开即写。支持 C/C++ 编译运行，也支持 Python 语法检查和运行。

## 下载

从 [Releases](https://github.com/sz/tinyvim/releases) 下载对应平台的可执行文件：

| 平台 | 下载文件 |
|------|---------|
| Windows x86_64 | `tinyvim-x86_64-windows.exe` |
| Linux x86_64 | `tinyvim-x86_64-linux` |
| macOS x86_64 | `tinyvim-x86_64-macos` |
| macOS arm64 | `tinyvim-aarch64-macos` |

## 使用

```bash
# 打开已有文件
./tinyvim hello.c
./tinyvim hello.py

# 新建文件（Ctrl+S 保存时输入文件名）
./tinyvim
```

### 快捷键

| 按键 | 功能 |
|------|------|
| `Ctrl+S` | 保存 |
| `Ctrl+O` | 打开文件（文件浏览器） |
| `Ctrl+N` | 新建文件 |
| `Ctrl+X` | 剪切 |
| `Ctrl+C` | 复制 |
| `Ctrl+V` | 粘贴 |
| `Ctrl+Z` | 回退 |
| `Ctrl+F` | 搜索 |
| `Ctrl+E` | AI 修改当前文件 |
| `Ctrl+R` | AI 聊天问答 |
| `Ctrl+Space` | 手动触发补全 |
| `Ctrl+A` | 全选 |
| `Ctrl+Q` | 退出 |
| `Alt+h` / `Alt+l` | 按词移动 |
| 鼠标滚轮 | 滚动代码；在输出面板 / AI 预览区域滚动输出 |
| `F3` / `Shift+F3` | 下一个 / 上一个搜索结果 |
| `F1` | 显示 / 关闭帮助 |
| `F2` | 重新配置 AI Key / Base URL / Model |
| `F5` | 编译 / 检查 |
| `F6` | 编译 / 检查并运行 |
| `F8` / `Shift+F8` | 下一个 / 上一个错误 |
| `PageUp` / `PageDown` | 输出面板 / AI 预览翻页 |
| `Esc` | 取消选择 / 关闭输出面板 |

### AI 助手（Ctrl+E / Ctrl+R）

AI 助手会把当前文件内容作为上下文发送到 OpenAI-compatible 接口。

- `Ctrl+E`：Edit，按你的要求生成当前 buffer 的候选修改，先在底部预览，按 `PageUp` / `PageDown` 或鼠标滚轮查看长内容，再按 `y` 应用、`n` 放弃。`y/n` 后会继续留在 AI Edit 输入框，直到按 `Esc` 退出。默认不自动保存，满意后按 `Ctrl+S` 保存；不满意可以按 `Ctrl+Z` 回退。
- `Ctrl+R`：Chat，只聊天问答，不修改文件，回答显示在底部输出面板；回答后会继续留在 AI Chat 输入框，按 `Esc` 退出。

Chat 和 Edit 都会记住本次 TinyVim 运行期间、本轮 AI 会话里的上下文，方便连续追问或连续调整修改要求。会话历史只保存在内存里，不写入磁盘。

默认使用 DeepSeek 的 OpenAI-compatible 接口：

- Base URL: `https://api.deepseek.com`
- Model: `deepseek-v4-flash`

第一次按 `Ctrl+E` 或 `Ctrl+R` 时，TinyVim 会引导配置三项：

1. Base URL：预填 DeepSeek 地址，可修改
2. Model：预填 DeepSeek 模型，可修改
3. API Key：不预填，输入时会隐藏显示

如果 API Key 输错、接口返回 401/403、或者想换模型/地址，按 `F2` 可以随时重新配置。
如果设置了 `TINYVIM_AI_API_KEY` 或 `OPENAI_API_KEY` 环境变量，环境变量会优先生效；此时按 `F2` 保存的配置不会覆盖环境变量。

配置会保存到当前用户自己的配置目录：

- Windows: `%APPDATA%\tinyvim\ai.json`
- macOS: `~/Library/Application Support/tinyvim/ai.json`
- Linux: `$XDG_CONFIG_HOME/tinyvim/ai.json` 或 `~/.config/tinyvim/ai.json`

也可以用环境变量覆盖，适合便携包、学校机房或 CI：

Linux/macOS:

```sh
export TINYVIM_AI_API_KEY="你的 API Key"
export TINYVIM_AI_BASE_URL="https://api.deepseek.com"
export TINYVIM_AI_MODEL="deepseek-v4-flash"
```

Windows PowerShell:

```powershell
$env:TINYVIM_AI_API_KEY="你的 API Key"
$env:TINYVIM_AI_BASE_URL="https://api.deepseek.com"
$env:TINYVIM_AI_MODEL="deepseek-v4-flash"
```

Windows cmd:

```bat
set TINYVIM_AI_API_KEY=你的 API Key
set TINYVIM_AI_BASE_URL=https://api.deepseek.com
set TINYVIM_AI_MODEL=deepseek-v4-flash
```

### 文件浏览器（Ctrl+O）

| 按键 | 功能 |
|------|------|
| `↑` `↓` / `j` `k` | 移动选择 |
| `Enter` | 打开文件 / 进入目录 |
| `h` | 返回上级目录 |
| `n` | 新建文件 |
| `m` | 新建目录 |
| `r` | 重命名 |
| `Space` | 预览 |
| `d` | 删除选中项 |
| `Esc` | 关闭 |

## 编译原理

- 优先使用系统编译器（gcc/clang/MSVC）
- C 文件无系统编译器时，自动下载 **TCC**（~150KB）到缓存目录
- C++ 文件无系统编译器时：
  - **Windows** → 自动下载 **w64devkit**（便携 MinGW，约 70MB）
  - **Linux/macOS** → 自动下载 **Zig**，使用 `zig c++` 编译单文件 C++
- Python 文件：
  - `F5` 使用 `python -m py_compile` 做语法检查
  - `F6` 先检查语法，再运行脚本
  - 优先查找系统 Python：Linux/macOS 使用 `python3` / `python`，Windows 使用 `python` / `py` / `python3`
  - 没有系统 Python 时，自动下载 **uv** 到缓存目录，并由 uv 按需托管 Python
- 自动下载地址可用环境变量覆盖，方便配置镜像：
  - `TINYVIM_TCC_URL`
  - `TINYVIM_W64DEVKIT_URL`
  - `TINYVIM_ZIG_URL`
  - `TINYVIM_UV_URL`
- 没有网络或下载地址不可访问时，TinyVim 会尝试自动打开下载地址和缓存目录；底部 Output 会显示失败 URL、文件应放置的位置和可覆盖的镜像变量名。下载后把文件放到提示路径，再按 `F5` / `F6` 重试：
  - TCC Windows: `tcc.zip`
  - w64devkit Windows: `w64devkit.zip`
  - Zig Linux/macOS: `zig.tar.xz`
  - uv: `uv-install.ps1` / `uv-install.sh`，也可以直接放 `uv.exe` / `uv`
- `F6` 运行时会临时回到真实终端，C/C++ 的 `cin` / `scanf` 和 Python 的 `input()` 都可以交互输入；程序结束后按 `Enter` 回到 TinyVim

## 自行编译

```bash
cargo build --release
# 产物在 target/release/tinyvim
```
