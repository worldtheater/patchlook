<p align="center">
  <img src="https://raw.githubusercontent.com/worldtheater/patchlook/main/assets/app-icon/patchlook-v2.png" alt="Patchlook 标志" width="112">
</p>

<h1 align="center">Patchlook</h1>

<p align="center"><a href="README.md">English</a> · 简体中文</p>

<p align="center">适用于 macOS、Windows 和 Linux 的原生 Git 查看器，默认只读。</p>

<p align="center">
  <a href="https://worldtheater.github.io/patchlook/zh/">主页</a> ·
  <a href="https://github.com/worldtheater/patchlook/releases">下载发行版</a> ·
  <a href="#从源码构建">从源码构建</a> ·
  <a href="https://github.com/worldtheater/patchlook/issues">反馈问题</a>
</p>

查看本地仓库或通过 SSH 浏览 Linux 服务器上的仓库，审阅改动、提交历史及文件版本。需要暂存本地改动时，可切换到读写模式。

![Patchlook 在浅色与暗色主题下查看自身仓库的提交历史](docs/images/patchlook-light-dark.webp)

## 功能

- **查看改动** — 浏览已暂存、未暂存及未跟踪的文件，查看新增和删除行数。
- **选择性暂存** — 在读写模式下，按文件或连续改动块勾选暂存、取消暂存，也可以在确认后丢弃不需要的改动块。
- **浏览历史** — 查看分支和提交，按提交说明或 SHA 前缀搜索整个分支的历史。
- **SSH 远程浏览** — 直接查看 Linux 服务器仓库中的改动和历史，无需克隆到本机；远程仓库始终只读。
- **对比代码** — 切换并排或行内 diff，支持语法高亮和行内差异高亮。
- **聚焦变化** — 折叠未修改的内容、自动换行，或同步滚动前后两个版本。
- **对比图片** — 并排预览前后版本，适应窗口大小，或缩放、平移查看细节。
- **使用外部程序打开** — 选择系统推荐的应用或其他程序，打开本地文件的最新版本；已删除文件使用删除前版本的临时副本。
- **个性化设置** — 支持浅色、暗色和跟随系统主题，提供九种界面语言。

## 安装

macOS（Apple Silicon、macOS 15 及以上）可以通过 [Homebrew](https://github.com/worldtheater/homebrew-tap) 安装：

```sh
brew install --cask worldtheater/tap/patchlook
```

Git 会自动安装。后续运行 `brew update && brew upgrade --cask patchlook` 升级。如果之前手动安装过，请先退出应用，将现有的 `Patchlook.app` 移出「应用程序」目录；偏好设置会保留。

前往 [Releases](https://github.com/worldtheater/patchlook/releases) 下载已发布的安装包，也可以[从源码构建](#从源码构建)。

| 平台 | 系统要求 | 安装方式 |
| --- | --- | --- |
| macOS | macOS 15 及以上，Apple Silicon | 打开 DMG，将 **Patchlook** 拖入 **Applications（应用程序）**。 |
| Windows | Windows 10 及以上，x64 | 解压 ZIP，运行 `patchlook.exe`。 |
| Linux | x86_64，X11 或 Wayland，支持 Vulkan 的显卡 | 解压 tar.gz，运行 `./patchlook`。 |

浏览本地仓库需要 [Git](https://git-scm.com/downloads) 在 `PATH` 中；SSH 连接需要 OpenSSH（`ssh` 在 `PATH` 中），Linux 服务器需要 Git、POSIX shell 和常见命令行工具，无需安装 Patchlook。

macOS 正式发行版已完成签名和公证。Linux 安装包在 Ubuntu 24.04 上构建，需要兼容的系统库。

## 快速开始

1. 点击 **打开仓库（Open repository）** 选择本地仓库，或[通过 SSH 连接远程仓库](#连接-linux-服务器)。
2. 在 **改动（Changes）** 中查看工作区和暂存区的变化，在 **历史（History）** 中浏览提交。
3. 在 **仓库（Repos）** 中切换仓库或浏览其他分支。

源码构建还支持在 **仓库（Repos）** 中展开父仓库，点击下方的子模块，按独立仓库浏览。未初始化的子模块会显示状态且不可进入；通过 Git 初始化后刷新父仓库即可。在子模块内部暂存只影响它自己的暂存区，在父仓库中勾选子模块条目只记录其提交号变化。

使用其他工具修改仓库后，点击刷新以更新内容。分支浏览使用所打开仓库的 Git 引用；需要获取远端更新时，请使用你常用的 Git 客户端。

文本 diff 要求文件使用 UTF-8 编码。支持的图片格式会显示预览，其他二进制文件仅列出变更信息，不显示内容差异。

源码构建还提供 **新建文本比对**：通过文件菜单（Windows/Linux 为 **≡** 菜单）、欢迎页或 **⌘N / Ctrl+N** 打开独立窗口。在左右两侧粘贴文本，或打开／拖入 UTF-8 文件，然后点击播放图标开始比对（**⌘Enter / Ctrl+Enter**）；铅笔图标可返回输入区，编辑或粘贴替换任一侧内容；刷新（**⌘R / Ctrl+R**）会重新读取已打开的文件并更新结果，保留粘贴的文本。内容仅用于本次窗口，不会保存，也不会覆盖原文件。

### 连接 Linux 服务器

**v0.1.6 起支持**，远程仓库始终只读。

1. 打开仓库菜单或 Recent 上方的 **+** 菜单，选择 **通过 SSH 连接…**，填写主机、端口、用户名及仓库的 Linux 绝对路径。
2. 选择 **密码** 或 **SSH 密钥**。不指定私钥文件时，使用默认密钥或 SSH agent。
3. 首次连接时与可信来源核对服务器指纹，再点击 **信任并连接**；确认后的密钥会自动记住，用于后续校验。

密码和私钥密码不会保存。切换仓库会保留连接，远程条目菜单可断开或重连；关闭窗口会结束连接。SSH 配置文件可在 **高级选项** 中按需指定，普通连接无需填写。

### 暂存改动

在本地仓库中，点击标题栏的锁图标切换到 **读写**，然后进入 **改动**；悬停锁图标可查看当前模式。只读时显示闭锁和绿色指示灯，读写时显示开锁和橙色指示灯。勾选文件可暂存整个文件，也可以使用 diff 右侧行号栏中的空心方框按钮，暂存一整段连续改动。取消勾选已暂存的文件或改动块，即可取消相应内容的暂存。同一文件只显示一条，部分暂存时显示改动块进度；已暂存的改动块保留在原位，diff 背景略微减淡。若同一段在暂存后又被修改，则明确分为“HEAD → 暂存区”和“暂存区 → 工作区”两组比较，保留三个版本的信息。

目录视图将已跟踪改动与未跟踪文件分为可折叠的两组。文件夹和分组都可以批量勾选，范围仅限各自的后代文件。空框或部分选中时点击会暂存剩余改动；全部选中时点击会取消该文件夹下的全部暂存。

这些操作会更新真实的 Git 暂存区。二进制文件、符号链接、子模块引用变化和仅元数据变化的条目按整文件处理；未解决的冲突请使用其他 Git 工具解决并暂存。如果文本文件被外部工具修改，请先刷新，再按块暂存。

未暂存 diff 左侧行号栏的 `»` 撤销按钮会在确认后**丢弃整段改动**，恢复为暂存区中的内容，其他改动和暂存区保持不变。丢弃新增文件的改动块会删除该文件。单纯暂存或取消暂存不会修改工作区文件。

文件名旁的 **撤销** 和 **重做** 按钮可恢复本窗口内的暂存与丢弃操作，包括因丢弃而删除的新增文件。每次点击算一步，文件夹批量操作也算一步，记录覆盖当前仓库的所有文件；切换仓库或关闭窗口后清空。若 HEAD 或相关内容已被外部工具修改，会停止恢复，不覆盖这些改动，也不影响无关文件。

切回 **只读** 会保留已有的暂存内容。每次新建应用窗口都默认只读。需要检查并提交当前暂存区中的全部改动时，在终端运行：

```sh
git diff --cached
git commit -m "提交说明"
```

使用不带 `-a` 和文件参数的普通 `git commit`，即可仅提交暂存区的内容。Patchlook 不执行提交、切换分支或推送。

## 快捷键

| 操作 | macOS | Windows / Linux |
| --- | --- | --- |
| 打开仓库 | `⌘ O` | `Ctrl O` |
| 刷新 | `⌘ R` / `F5` | `Ctrl R` / `F5` |
| 复制所选内容 | `⌘ C` | `Ctrl C` |
| 全选当前 diff 面板中的内容 | `⌘ A` | `Ctrl A` |
| 在文件树中导航 | 方向键 | 方向键 |
| 退出 | `⌘ Q` | `Ctrl Q` |

## 从源码构建

安装 Rust 1.95 及以上版本、Git 和[对应平台的构建依赖](https://github.com/worldtheater/patchlook/blob/main/docs/development.md#prerequisites)，然后运行：

```sh
git clone https://github.com/worldtheater/patchlook.git
cd patchlook
cargo run --locked --release
```

测试、打包、发布及 GPUI Kit 补丁的说明见[开发指南](https://github.com/worldtheater/patchlook/blob/main/docs/development.md)（英文）。

使用 [Rust](https://www.rust-lang.org/) 和 [GPUI Kit](https://github.com/longbridge/gpui-kit) 构建。

采用 [MIT 开源协议](LICENSE)。
