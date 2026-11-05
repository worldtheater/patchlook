# Project State

## Purpose and Boundaries
- Patchlook 是 MIT 开源的 Rust／GPUI Kit 桌面 Git 查看器，面向 macOS、Windows、Linux。本地默认只读；启用读写后支持文件／文件夹／连续改动块的暂存、取消暂存和确认后丢弃，以及当前仓库／窗口内的撤销重做。Linux 服务器经 SSH 只读浏览。应用不执行 commit、checkout、fetch、pull 或 push；外部改动手动刷新。
- 独立文本比对无需仓库或 Git：粘贴文本、打开／拖入 UTF-8 文件，后台比对；内容只存在窗口内，不保存、不写回原文件。Git 文本 diff 同样要求 UTF-8，支持的图片独立预览，其他二进制仅展示变化信息。
- Git 浏览本地依赖 PATH 中的 Git；SSH 依赖客户端 OpenSSH 与服务端 Git／POSIX shell／常用 Linux 工具。支持密码、普通／加密私钥及默认密钥／agent；秘密不持久化，不支持交互式 MFA、修改服务器密码或远程写操作。

## Current Architecture
- `main.rs` 在 GPUI 初始化前处理 ASKPASS 子进程，其余初始化主题、菜单和窗口；包名 `patchlook`，标识 `dev.patchlook.app`。`build.rs` 将 ICO 嵌入 Windows EXE，失败中止构建；标题栏控件隔离原生拖动命中。`text_compare.rs` 独立管理两侧 EditorState、导入任务和 DiffState，输入／结果共用空间；`text_diff.rs` 用 similar 在内存生成完整上下文 patch，再交 Kit 解析；`ui.rs` 共享图标按钮、语言菜单、主题按钮及分隔线；DisplaySettings 同步同进程窗口的语言／外观，仓库窗口同步内存偏好，文本窗口读最新偏好后只更新显示字段，避免覆盖仓库记录。
- `git.rs` 封装 Git CLI，RepositoryLocation 区分本地与 SSH；`git/transport.rs` 引用远程 POSIX 参数、复用分帧 SSH 会话，历史独立流式连接。NUL 分隔路径、禁用可选锁／外部 diff／textconv，完整上下文取 `i32::MAX / 2`，避免 Windows Git 溢出。窗口保留认证会话，切回复用；手动断开／重连／移除／关窗口终止对应会话并拒绝旧句柄。启动仅恢复本地仓库，远程须显式连接。
- `ssh_auth.rs` 以随机令牌保护本机内存通道供同一可执行文件 ASKPASS 取凭据，严格匹配目标提示；秘密不放参数、环境或磁盘。`ssh_host.rs` 排队展示首次 SHA-256 指纹确认，明确接受后由 OpenSSH 写入 known_hosts；后续严格验证，变更／撤销密钥拒绝连接，不自动覆盖；关闭窗口拒绝待处理请求。
- `git/staging.rs` 用 add／reset 更新明确路径，用 `git apply --cached` 更新块；丢弃反向应用工作区，保留索引。写前核对状态及两侧补丁，拒绝过期选择；`review.rs` 合并同路径，通常显示 HEAD → 工作区并标注块状态，第三版本／抵消／无法映射时分成 HEAD → 索引、索引 → 工作区。`git/undo.rs` 保存相关索引或文件字节／权限，恢复前核对 HEAD 与快照，只恢复明确路径；区分意向添加与真实空文件，兼容 SHA-1／SHA-256。
- `app.rs` 的 Working／History ChangeState 分离，Repos／History 共用历史快照；缓存列表、选择、树、滚动、diff及任务。已加载 Tab 不重读 Git；切仓库／刷新使另一快照失效，切浏览分支只重载历史。选择／目标／已加载仓库分离，取消过期任务；数据就绪后替换，Repos 进入新文件先展示目标加载态。写后只重算受影响路径、状态变化项及当前预览，复用其余统计／进度；仅当前仓库保留界面快照。
- `git/submodules.rs` 从索引 gitlink 发现直接子模块，在子模块目录内解析独立 `.git`，避免 Windows 反斜杠导致 gitfile 相对路径误判，禁止回落父仓库；进入后复用普通仓库功能，嵌套按进入逐层发现。SSH 子模块共享父认证会话，断开／重连清理共享连接。Preferences 持久化父子关系及各自浏览分支，子模块不重复列在顶层；移除父记录连同后代，不删除文件。
- 文本 diff 解析、虚拟滚动、复制及主题来自 Kit DiffFile／DiffState；视口／行高变化在绘制后通知，不依赖移动窗口触发首次布局。`change_tree.rs` 分离已跟踪／未跟踪树，批量操作限后代；`image_diff.rs` 处理图片；`external_open.rs` 用平台 API 列出应用图标，打开工作区最新文件，删除文件打开退出时清理的旧版本副本。
- `strings.rs` 管理九语言资源；`themes/` 统一配色，浅色沿用 Kit 语法色、正文深灰，暗色低饱和。Preferences 在系统配置目录原子替换 JSON。About 及 SSH Dialog 复用 Kit 弹窗／Presence，遮罩同步淡入淡出、遵循减少动态效果。Windows／Linux 窗口内菜单提供文本比对、关于、退出；macOS 系统菜单。
- `desktop.yml` 在 main／PR 三平台检查与开发打包，匹配 Cargo 版本的 v 标签执行正式 macOS 签名／公证与三平台 Release 草稿。`scripts/package.py` 默认 ad-hoc，正式签名失败不降级。`website/` 无前端依赖，Python 生成九语言主页及最新正式 Release 的下载版本／链接／附件 SHA-256；Pages 在站点变更、正式发布／编辑时更新。独立 Homebrew tap 每六小时检查正式版本，下载并验证后更新 Cask；使用带已附加公证票据的 macOS ZIP，不同步草稿。

## Project Conventions
- GPUI 统一从 `gpui_kit` 导入，复用主题／控件；界面文案在九份 `locales/*.json`，键及占位符一致。Kit 0.8.0 固定 fork revision `907fa984b7a1f316fc767ccf3e25c10b54932518`（已推送），维护 Cargo.lock，不提交本机 path override；升级及补丁归属见 `docs/development.md`。
- `.gitattributes` 强制 shell 脚本 LF；嵌入 SSH worker 不得随 Windows 检出转 CRLF，所有平台测试内嵌脚本换行。
- Git 基线：未暂存比较索引与工作区，暂存比较 HEAD 与索引；根提交按新增，合并提交比较第一父提交。历史／搜索固定首次解析的分支 tip；搜索完整消息忽略大小写的子串或 SHA 前缀，每页 100 条加一条前瞻。
- 测试共享进程级 locale，须串行：`cargo fmt --all -- --check`、`cargo clippy --locked --all-targets -- -D warnings`、`cargo test --locked -- --test-threads=1`；交互验收使用 Release，打包改动另跑 Python 3.11+ 的 scripts 单元测试。系统应用查询／本机认证 socket 测试需要系统能力，不以沙箱失败判功能失败。
- 自动化使用临时仓库／配置；用户授权 `../patchlook_test` 交互测试，按其 README 恢复。SSH fixture 由 `scripts/ssh_test.py up/down` 管理，密钥／配置仅在忽略的 `dist/ssh-test/`，端口绑定本机、不挂载宿主仓库；真实 SSH 测试默认忽略。动画／短暂视觉变化由用户验收。
- 缓存统一 `target/`；dev 为 line-tables-only、packed split-debuginfo，test 继承 dev，保留增量，常用检查保持相同特性组合。

## Implementation State
### Implemented
- 独立文本比对：菜单／欢迎页／⌘N 或 Ctrl+N 打开，两侧粘贴、选择或拖入文件，也可一次拖入两个；交换左右，开始比对（⌘Enter／Ctrl+Enter）、返回编辑、关闭窗口快捷键。默认并排，支持行内、语法语言选择、折行及相同内容折叠；相同文本明确提示。标题栏复用主窗口的语言／主题切换；操作为图标按钮与 tooltip，左右名称／打开入口与比对控制合为一行 32 px 工具栏，左右各占内容区一半，结果操作不挤动标题；开始比对、文件夹与编辑（铅笔）按钮常驻；结果页可返回输入区编辑导入文件的临时副本或粘贴替换任一侧内容，输入页编辑按钮禁用，无上下差异按钮。标题栏刷新（⌘R／Ctrl+R／F5）重读文件，保留粘贴侧；结果页刷新／换文件会后台生成新 diff，全部成功后统一替换，失败保留原内容。只读文件快照，无 Git 写入口和内容持久化。代码及中英文使用说明已完成。
- 仓库连接：本地选择、SSH 主机／端口／用户名／路径分开填写，密码或密钥，高级配置可选；秘密可切明文，切认证清空。六类非秘密字段各留 20 条可删除历史。Recent 按名称排序，远程先显示连接占位；连接状态及更多菜单支持断开／重连，切仓库保留认证。仅两种仓库同时存在时显示可折叠“本地／远程”标题。
- 子模块：父仓库双行名称／路径、子模块 30 px 单行，树线表达层级、选中背景边界一致，完整路径 tooltip。普通根条目无箭头占位，有子模块才留栏，箭头仅颜色响应 hover；未初始化禁用。用户已验收本地与 SSH 效果，已提交 `e5d8001`。
- Repos 三栏、Changes／History 保留 Tab 选择、滚动和筛选；Recent／Branches 默认等高，拖动比例持久化。分支整行切浏览、眼睛标记查看项，不改 HEAD；顶部分支随视图，History 不重复标分支。历史搜索超过 20 条常驻，Load more 为尾项；消息匹配高亮、SHA 不高亮；提交说明可选／显隐，点击 SHA 复制后原位短暂提示。
- 文件树／平铺与统计：主流类型图标、状态色、根路径 `.`、移动来源；键盘父子／相邻导航，选文件均打开 diff。目录行 22 px、图标树 16／平铺 20 px；轻填充文件夹、紧凑勾选间距，路径 tooltip 只绑名称。120 ms 内连续选文件尾随防抖，单次立即加载。
- 本地写入：同文件单条，空框／横线／对号表示暂存程度，部分显示 x/y；目录／分组批量操作限后代。右侧行号栏勾选连续块，左侧 `»` 确认丢弃，无单行入口或额外缩进；已暂存背景保留 30%，hover 半透明，行号不触发整行选择。撤销／重做每次点击一步，跨文件共用栈，刷新保留，切仓库／关窗口清空，外部冲突拒绝覆盖；二进制／非普通文件仅整文件操作，冲突交外部工具。
- Git diff：并排／行内、语法与行内高亮、折行、同步滚动、复制、上下文折叠；改动状态改变而文本不变时不替换 diff，补丁变动保留稳定源视口。图片支持适应／1:1／缩放。持久化外观、语言、diff 布局、文件列表样式、行号／统计／折行／说明、Recent、浏览分支和 Repos 侧栏比例；其余窗口／Tab／选择／搜索不保存。每窗口默认绿灯闭锁只读，橙灯开锁读写；锁按钮无文字、tooltip 说明，操作中禁用保持外观。
- About、logo、许可证、中英文 README及真实浅暗截图；九语言主页提供语言菜单、直接切主题、平台下载浮层、Homebrew、单行 SHA-256 和复制，缺失／无效附件哈希阻止构建。SEO 元数据／JSON-LD／sitemap 与内容同源，Google 验证标签已部署；项目 Pages 的 sitemap 需自行提交站长工具。

### In Progress
- [paused] 提交说明局部高亮及实时拖选：用户决定等待 Kit Base 上游；当前 SelectableText 缺局部高亮 API，拖选变化未重绘。列表高亮已完成。

### Next
暂无已确认的后续范围。

## Known Problems and Risks
- P2 [risk]: diff 布局／上下文／滚动依赖 Kit fork；升级核对上游合入，并保留消费方 `tests/diff_layout.rs`、`tests/scroll_bounce.rs` 回归。

## Open Decisions
暂无。

## Verification
- v0.1.7 的三平台 CI 已通过 fmt、Clippy、常规串行回归、打包脚本测试及 Release 构建；macOS 128 项、Windows 126 项 Rust 测试通过，真实 SSH fixture 测试仍按配置忽略。覆盖文本比对、粘贴／文件拖入、再次编辑、刷新原子替换、标题对齐、多窗口显示偏好同步及源文件不变；Windows 子模块初始化误判修复后的 3 项集成回归均通过。此前 macOS 独立 Release 已检查浅／暗色和语言菜单；Windows／Linux 新窗口行为尚未实机验收。
- 既有证据：子模块 118 项常规及 9 项 Docker SSH／界面测试通过，用户确认本地与 SSH 可用；暂存／撤销用真实提交树覆盖第三版本、改名、二进制、CRLF、意向添加、SHA-1／SHA-256及外部冲突。SSH 已覆盖密码／加密私钥、主机信任／变更拒绝、凭据隔离、断线重连及连接复用；用户确认 macOS → WSL、Windows SSH 可用。新功能之外未重跑这些真实远程测试。
- [v0.1.7 草稿](https://github.com/worldtheater/patchlook/releases) 指向 `65d90e2`，中英文说明已保存；[构建 42053716637](https://github.com/worldtheater/patchlook/actions/runs/42053716637) 三平台及 Release 上传成功，四份附件已下载校验摘要和归档。macOS ZIP 内 App 签名／公证票据／Gatekeeper 及 DMG 票据本地复核通过，版本为 0.1.7。尚未公开发布；最新正式版仍为 v0.1.6，主页／Homebrew 只跟随正式发布。Cask 使用带 App 公证票据的 ZIP，独立 tap 自动校验安装、签名、公证后更新。
- 主页构建及桌面／窄屏交互、复制哈希已验证；校验值版和 Google 验证标签版 `4e2b99e` 已部署，英文／中文 head 核对完成。Search Console 验证、sitemap 提交及实际收录／AI 引用无完成证据。

## Source Pointers
- `src/{main,app,text_compare,text_diff,ui,preferences,strings}.rs`、`locales/`、`themes/` — 窗口、状态、文本比对、偏好和多语言。
- `src/git{.rs,/transport.rs,/ssh_worker.sh,/staging.rs,/review.rs,/undo.rs,/submodules.rs}`、`src/{ssh_auth,ssh_host,remote_repository}.rs`、`tests/` — Git／SSH及写入契约、回归。
- `src/{change_tree,image_diff,file_icons,external_open,about}.rs`、`assets/` — 列表、预览、系统打开与关于。
- `README*.md`、`docs/development.md`、`scripts/`、`.github/workflows/`、`website/` — 依赖、构建／发行、SSH fixture和主页；独立 `worldtheater/homebrew-tap` 维护 Cask。
