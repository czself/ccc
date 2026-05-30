# TinyVim — 零配置终端 IDE

即下即用，无需安装任何依赖。打开即写，按 F5 编译。

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
| `Ctrl+Z` | 撤销 |
| `Ctrl+A` | 全选 |
| `Ctrl+Q` | 退出 |
| `F5` | 编译 |
| `F6` | 编译并运行 |
| `Esc` | 取消选择 / 关闭输出面板 |

### 文件浏览器（Ctrl+O）

| 按键 | 功能 |
|------|------|
| `↑` `↓` / `j` `k` | 移动选择 |
| `Enter` | 打开文件 / 进入目录 |
| `h` | 返回上级目录 |
| `n` | 新建文件 |
| `m` | 新建目录 |
| `d` | 删除选中项 |
| `Esc` | 关闭 |

## 编译原理

- 优先使用系统编译器（gcc/clang/MSVC）
- C 文件无系统编译器时，自动下载 **TCC**（~150KB）到缓存目录
- C++ 文件无系统编译器时：
  - **Windows** → 自动下载 **w64devkit**（便携 MinGW，~12MB）
  - **Linux/macOS** → 显示安装指引

## 自行编译

```bash
cargo build --release
# 产物在 target/release/tinyvim
```
