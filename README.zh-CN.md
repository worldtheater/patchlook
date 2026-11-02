<p align="center">
  <img src="https://raw.githubusercontent.com/worldtheater/patchlook/main/assets/app-icon/patchlook-v2.png" alt="Patchlook 标志" width="112">
</p>

<h1 align="center">Patchlook</h1>

<p align="center"><a href="README.md">English</a> · 简体中文</p>

<p align="center">适用于 macOS、Windows 和 Linux 的原生 Git 查看器，默认只读。</p>

<p align="center">
  <a href="https://github.com/worldtheater/patchlook/releases">下载发行版</a> ·
  <a href="#从源码构建">从源码构建</a> ·
  <a href="https://github.com/worldtheater/patchlook/issues">反馈问题</a>
</p>

查看本地改动、浏览提交历史、对比文件版本。Patchlook 默认以只读模式启动，需要暂存改动时可切换到读写模式。

![Patchlook 在浅色与暗色主题下查看自身仓库的提交历史](docs/images/patchlook-light-dark.webp)

## 功能

- **查看改动** — 浏览已暂存、未暂存及未跟踪的文件，查看新增和删除行数。
- **选择性暂存** — 在读写模式下，按文件或连续改动块勾选暂存、取消暂存，也可以在确认后丢弃不需要的改动块。
- **浏览历史** — 查看分支和提交，按提交说明或 SHA 前缀搜索整个分支的历史。
- **SSH 远程浏览** — 直接查看 Linux 服务器仓库中的改动和历史，无需克隆到本机；远程仓库始终只读。
- **对比代码** — 切换并排或行内 diff，支持语法高亮和行内差异高亮。
- **聚焦变化** — 折叠未修改的内容、自动换行，或同步滚动前后两个版本。
- **对比图片** — 并排预览前后版本，适应窗口大小，或缩放、平移查看细节。
- **使用外部程序打开** — 从文件标题栏选择系统推荐的应用或其他程序，打开工作区中的最新文件；对于已删除文件，打开删除前版本的临时副本。
- **个性化设置** — 支持浅色、暗色和跟随系统主题，提供九种界面语言。

## 安装

前往 [Releases](https://github.com/worldtheater/patchlook/releases) 下载已发布的安装包，也可以[从源码构建](#从源码构建)。

| 平台 | 系统要求 | 安装方式 |
| --- | --- | --- |
| macOS | macOS 15 及以上，Apple Silicon | 打开 DMG，将 **Patchlook** 拖入 **Applications（应用程序）**。 |
| Windows | Windows 10 及以上，x64 | 解压 ZIP，运行 `patchlook.exe`。 |
| Linux | x86_64，X11 或 Wayland，支持 Vulkan 的显卡 | 解压 tar.gz，运行 `./patchlook`。 |

需要安装 [Git](https://git-scm.com/downloads)，并确保可通过 `PATH` 找到。macOS 正式发行版已完成签名和公证。Linux 安装包在 Ubuntu 24.04 上构建，需要兼容的系统库。

## 快速开始

1. 点击 **打开仓库（Open repository）**，选择本地 Git 仓库。
2. 在 **改动（Changes）** 中查看工作区和暂存区的变化，在 **历史（History）** 中浏览提交。
3. 在 **仓库（Repos）** 中切换仓库或浏览其他分支。

使用其他工具修改仓库后，点击刷新以更新内容。分支浏览使用本地 Git 引用；需要获取远端更新时，请使用你常用的 Git 客户端。

文本 diff 要求文件使用 UTF-8 编码。支持的图片格式会显示预览，其他二进制文件仅列出变更信息，不显示内容差异。

### 连接 Linux 服务器

点击标题栏的仓库按钮，或 Recent 上方的 **+**，选择 **通过 SSH 连接…**，分别填写主机、端口、用户名和仓库的 Linux 绝对路径，选择 **密码** 或 **SSH 密钥** 认证。密钥方式可选择私钥文件并填写可选的私钥密码；不选文件则使用默认密钥或 SSH agent。**高级选项** 中可指定 SSH 配置文件。端口、用户名留空时先使用 SSH 配置，再使用 OpenSSH 默认值；连接后会保存到最近仓库列表。

客户端需要安装 OpenSSH，确保 `ssh` 在 `PATH` 中；服务器需要 Git、POSIX shell 和常见 Linux 命令行工具。首次连接时，应用会显示服务器主机密钥的 SHA-256 指纹。与可信来源核对后点击 **信任并连接**，OpenSSH 会记入常规的 `known_hosts` 文件（或高级 SSH 配置指定的位置），后续自动校验。取消不会保存，已记录的密钥发生变化时会阻止连接。普通连接无需填写 SSH 配置文件。密码和私钥密码仅在当前连接的内存中使用，不会保存到 Recent 或配置文件；刷新和切换仓库都会保留已连接的 SSH 会话，切回来无需再次输入密码。远程条目的菜单可手动断开或重连；移除条目或关闭窗口也会关闭对应会话。重启应用后远程记录保持断开，点击时再连接。跳板机可在 SSH 配置中设置，但需要独立认证，例如密钥或 agent；暂不支持交互式多因素认证或修改服务器密码。

支持查看远程工作区、暂存区、未跟踪文件、分支、历史、搜索及图片预览；外部修改后手动刷新。远程仓库的锁始终关闭，不提供暂存、丢弃、撤销／重做或用本地外部程序打开远程文件的操作。服务器无需安装 Patchlook。

### 暂存改动

点击标题栏的锁图标切换到 **读写**，然后进入 **改动**；悬停锁图标可查看当前模式。只读时显示闭锁和绿色指示灯，读写时显示开锁和橙色指示灯。勾选文件可暂存整个文件，也可以使用 diff 右侧行号栏中的空心方框按钮，暂存一整段连续改动。取消勾选已暂存的文件或改动块，即可取消相应内容的暂存。同一文件只显示一条，部分暂存时显示改动块进度；已暂存的改动块保留在原位，diff 背景略微减淡。若同一段在暂存后又被修改，则明确分为“HEAD → 暂存区”和“暂存区 → 工作区”两组比较，保留三个版本的信息。

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
