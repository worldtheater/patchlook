# Project State

## Purpose and Boundaries
- Patchlook 是 MIT 开源的 Git 查看器，Rust／GPUI Kit 实现，面向 macOS、Windows、Linux。本地仓库默认只读；显式启用读写后支持文件／文件夹／连续改动块的暂存、取消暂存，以及确认后丢弃未暂存块；当前仓库、当前窗口内可撤销／重做。支持 Linux 服务器经 SSH 只读浏览；远程不提供暂存、丢弃、撤销／重做或本地外部打开。应用不执行 commit、checkout、fetch、pull 或 push，外部变更通过手动刷新读取。
- 本地依赖 PATH 中的 Git；远程依赖客户端 OpenSSH、服务器 Git／POSIX shell／常用 Linux 工具，支持密码、指定私钥（含私钥密码）及默认密钥／agent，首次连接在应用中核对 SHA-256 指纹，确认后由 OpenSSH 保存到 known_hosts，后续自动校验，变更／撤销的密钥拒绝连接；密码仅用于当前连接，不持久化，不支持交互式 MFA 或修改服务器密码。文本 diff 要求 UTF-8；支持的图片独立预览，其他二进制展示变化信息。

## Current Architecture
- `src/main.rs` 在初始化 GPUI 前处理独立 ASKPASS 调用，其余启动初始化窗口、菜单、主题；包／二进制名 `patchlook`，标识 `dev.patchlook.app`。`build.rs` 将 ICO 嵌入 Windows EXE 资源 ID 1，失败中止构建；标题栏操作区隔离原生拖动命中。
- `src/git.rs` 封装 Git CLI；`RepositoryLocation` 区分本地路径与 SSH 元数据。本地直接传参，远程经 `git/transport.rs` POSIX 引用参数，复用分帧 SSH 会话；历史独立流式连接，临时 shell worker 退出清理。窗口内保留认证会话，子模块导航共享父连接，切仓库／刷新复用，切回无需重输密码；手动断开、重连替换、移除或关窗口时关闭对应会话，终止子进程并拒绝旧句柄读请求。`ssh_auth.rs` 以随机令牌保护的本机内存通道向同一可执行文件 ASKPASS 提供凭据，严格匹配目标提示，不落盘或放入参数／环境变量。普通启动只自动恢复本地仓库，远程保持断开；显式 CLI SSH 目标在 Root 安装后连接。读写 API 与 UI 双层禁止远程写入。NUL 分隔路径，禁用可选锁、外部 diff 和 textconv；完整上下文为 `i32::MAX / 2`、inter-hunk context 为 0，避免旧 Windows Git 溢出。
- `src/git/staging.rs` 以 add／reset 更新明确路径，以完整上下文补丁和 `git apply --cached` 更新改动块；丢弃反向应用到工作区，保留索引。部分操作在写入前统一核对状态与两侧显示补丁，拒绝过期选择，避免下层重复验证。只需改动列表时不查询分支；预览与进度复用同次读取的状态／补丁。`review.rs` 合并同路径条目，通常展示 HEAD → 工作区并标注块状态；索引有第三版本、抵消或无法匹配时分为 HEAD → 索引及索引 → 工作区两组，保留索引独有内容。
- `src/git/undo.rs` 记录写入前后的受影响索引条目或文件字节／权限；恢复前核对 HEAD 和相关快照，只恢复明确路径，不覆盖无关变化；成功的写入／撤销／重做返回受影响路径供刷新使用。仓库打开时读取一次当前对象格式的空 blob ID；仅相关索引条目指向空 blob 时查询并保存 `git add -N` 标记，区分真实空文件，兼容 SHA-1／SHA-256。工作区恢复以临时文件替换。
- `src/app.rs` 的工作区／历史 `ChangeState` 独立保存列表、选择、树、滚动、diff、错误与任务；`HistoryState` 供 Repos／History 共用。选中项、连接目标、已加载仓库分离；更换目标会取消旧加载，包括加载中点回当前仓库，避免旧结果覆盖选择；新 SSH 先展示 Recent 占位，成功替换快照，失败保留错误和可用旧内容。仅当前仓库保存界面快照，远程会话独立保留；切回重新读取数据。已加载 Tab 切换不读 Git，隐藏页结果写回所属状态；切仓库／刷新成功后使另一组失效，浏览分支只重载历史。写入后重算受影响路径、状态变化项及当前预览，其余统计／进度复用；已解析 review 提供两侧统计，二进制／元数据项用 numstat。手动刷新完整读取外部改动。
- 文本解析与虚拟滚动使用 Kit `DiffFile`／`DiffState`；视口／行高变化在绘制结束后通知，首次布局不依赖移动窗口。`src/image_diff.rs` 处理图片；`src/external_open.rs` 通过 NSWorkspace／Windows Shell／GIO 列出系统应用及图标，打开工作区最新文件；删除文件打开旧版本临时副本，保留至退出。
- `src/change_tree.rs` 压缩单一路径目录；已跟踪／未跟踪分组独立折叠、批量操作限定后代索引，避免同名目录跨组。`file_icons.rs` 内嵌 35 个 Material Icon Theme MIT SVG。`preferences.rs` 读写系统配置目录的 `preferences.json`，显式保存、临时文件替换；本地路径继续按字符串序列化，SSH 按完整连接元数据区分 Recent 和浏览分支。
- `strings.rs` 管理九语言资源；`themes/` 统一配色。浅色沿用 Kit 语法主题，仅正文及普通变量／属性／嵌入文本调深灰，深色自定义低饱和主题；正文取 `editor_foreground`。激活窗口重读系统语言，订阅系统外观。
- `src/ssh_host.rs` 将 ASKPASS 主机确认转到窗口，排队处理，接受／拒绝回到原 SSH 会话；关闭窗口拒绝待处理请求。交互连接强制 StrictHostKeyChecking=ask，不自动接受或覆盖；尊重默认及高级配置的 known_hosts。Kit Component 的 Dialog／AlertDialog 在实际 deferred 浮层内应用进入透明度，遮罩与弹窗同步淡入，沿用原曲线及减少动态效果设置。`src/about.rs` 复用 Kit Dialog／Presence，居中展示 logo、版本、简介、MIT、GitHub／反馈；淡出结束才卸载遮罩、恢复焦点，遵循减少动态效果。Windows／Linux 窗口内“≡”菜单仅含关于／退出，macOS 用系统菜单。
- `.github/workflows/desktop.yml` 在 main／PR 三平台检查及开发打包；匹配 Cargo 版本的 v 标签执行 macOS Developer ID 签名、公证、票据验证，三平台成功后创建 Release 草稿。`scripts/package.py` 默认 ad-hoc，正式签名失败不降级；手动输入 Apple submission ID 时仅查询既有公证。`website/` 为无前端依赖的九语言静态主页，Python 从最新正式 Release 生成版本、下载链接及同一附件的 SHA-256、复用既有 logo／截图；独立 Pages 工作流在站点变更和正式发布后更新，PR 仅构建。独立 `worldtheater/homebrew-tap` 提供 macOS Cask，使用 App 已附带公证票据的正式 ZIP，依赖 Git、Apple Silicon／macOS 15+；每六小时检查正式 Release，下载核对 SHA-256 并通过安装／签名／公证校验后自动更新，不同步草稿。

## Project Conventions
- `.gitattributes` 强制 shell 脚本使用 LF；SSH worker 经 `include_str!` 原样嵌入，不能随 Windows 检出转换为 CRLF。所有平台运行嵌入脚本换行回归。
- GPUI 统一从 `gpui_kit` 导入，复用控件与主题；九语言 `locales/*.json` 键及占位符一致。Kit 0.8.0 固定 fork revision `907fa984b7a1f316fc767ccf3e25c10b54932518`，已推送；维护 Cargo.lock，不提交本机 path override；升级见 `docs/development.md`。
- Git 底层未暂存为索引对工作区，已暂存为 HEAD 对索引；根提交按新增，合并提交比较第一父提交。历史／搜索固定首次解析的分支 tip；关键词匹配完整说明子串（忽略大小写）或 SHA 前缀，流式扫描，每页 100 条加一条前瞻。
- 界面测试共享进程级 locale，CI 与本地均串行运行测试，避免跨用例干扰。常规检查：`cargo fmt --all -- --check`、`cargo clippy --locked --all-targets -- -D warnings`、`cargo test --locked -- --test-threads=1`；构建变更另跑 Release；打包测试用 Python 3.11+ 执行 `python -m unittest discover -s scripts -p 'test_*.py'`。
- 自动化测试用临时仓库／配置；交互验收用 Release。用户授权 `../patchlook_test` 作为可直接暂存／取消／丢弃的测试场地，恢复说明在其 README。SSH 使用 `scripts/ssh_test.py up/down` 管理独立 Docker fixture，密钥／配置仅在忽略的 `dist/ssh-test/`，端口只绑定本机，仓库不挂载宿主机；真实 SSH 测试默认忽略，运行方式见开发指南。动画、短暂切换等视觉效果由用户验收。
- 缓存统一根 `target/`；dev 为 `line-tables-only`＋`split-debuginfo = "packed"`，test 继承 dev，保留增量，常用检查保持同一依赖特性组合。

## Implementation State
### Implemented
- 仓库菜单／Recent 加号和欢迎页支持 SSH：主机／端口／用户名／绝对仓库路径分开，密码或密钥登录，私钥密码可选，高级 SSH 配置默认收起。首次 SHA-256 指纹须明确确认；变更密钥拒绝，无覆盖入口。表单实时校验，端口／用户名可留空，支持默认密钥／agent。六类非秘密字段各保留 20 条历史，可筛选、键盘选择、逐条删除；秘密不进入历史，默认遮蔽，可用眼睛按钮显示，切换认证时清空并恢复遮蔽。CLI 支持 `--ssh host /path --ssh-config file`。Recent 按名称排序，Repos 在父仓库下可折叠显示直接子模块；父仓库保留双行名称／路径，子模块为 30 px 单行，以主题色树线表达嵌套，选中背景边界统一，完整路径保留在 tooltip；无子模块的顶层仓库沿用原内边距，有子模块时才提供折叠箭头栏，箭头居中并仅以颜色响应 hover。进入后复用普通仓库的 Changes／History／暂存与撤销规则；父子关系及各自浏览分支持久化，子模块不重复显示在顶层，移除父记录会移除后代记录。打开／刷新仓库时从索引 gitlink 发现子模块，未初始化条目禁用；进入时核对独立 Git 目录，禁止回落到父仓库。嵌套子模块按进入逐层发现；SSH 复用父连接，断开或重连同步清理共享连接的条目。只有本地／远程同时存在时显示分类标题及独立折叠箭头；折叠仅窗口内保留，新连接展开远程组。远程行显示连接中／已连接／已断开／失败；更多菜单可断开／重连，后台会话可单独断开，切回已连接条目不再认证。加／减按钮只增删记录。分支整行点击切换浏览，眼睛标记当前查看分支，移除重复的行尾箭头；浏览分支按仓库保存且不改变 HEAD，Changes 始终工作区；顶部分支随视图，History 列表标题及树根不重复分支。
- Repos 三栏、Changes、History 共享缓存；Tab 保留选择／滚动／树展开／搜索。Repos Recent／Branches 默认等高，拖动比例持久化。历史 Load more 为尾部 item，未筛选超过 20 条常驻搜索框，Enter 查询、清空恢复、Esc 失焦；标题高亮关键词，SHA 不高亮。提交说明可选择及显隐；短 SHA 点击复制完整值，原地 Copied 两秒。
- 文件树／平铺、统计、键盘父子导航；所有文件选择均打开 diff。目录行及文本行框 22 px，图标与文字居中、紧凑间距；文件夹轻填充，图标树 16 px／平铺 20 px。修改／改名蓝、新增绿、删除灰、冲突警示；根路径显示 `.`，移动项显示相对来源。路径 tooltip 只绑名称，勾选不触发文件 tooltip、选择或折叠。
- 工作区同文件一条，空框／横线／对号表示未暂存／部分／全部，部分显示已暂存 x/y；文件夹／分组勾选仅作用于后代。Diff 右侧行号栏勾选连续块，左侧 `»` 确认丢弃，无单行入口或额外正文缩进。已暂存块保留 30% 背景强度，操作 hover／按下背景半透明；行号栏不触发整行选择，正文拖选保留。历史无写入口；二进制、非普通文件仅整文件操作，冲突交其他工具处理。
- 文件名右侧撤销／重做：一次文件／目录／块操作算一步，跨文件共用栈；新操作清空重做，刷新保留，切仓库／关窗口清空。可恢复丢弃的新文件，外部 HEAD／相关内容冲突拒绝恢复。暂存状态改变但补丁不变时仅刷新控件／背景；补丁变化以稳定源版本保留展开内容、视口偏移及横向滚动。
- Diff 支持并排／行内、语法与行内高亮、折行、同步滚动、复制、上下文折叠及块导航。单次选文件立即加载，120 ms 内快速切换才尾随防抖；详情保留旧内容至目标就绪，Repos 目录进新文件先显示目标加载状态。顶栏文件类型图标及外部打开菜单，无整文件收起按钮。图片支持尺寸、透明棋盘、适应／1:1／缩放滚动。
- 持久化外观、diff 布局、语言、文件列表样式、行号、折行、提交说明、Recent、各仓库浏览分支和 Repos 侧栏比例；其他分栏、窗口、Tab／选择／滚动／搜索不持久化。每窗口默认只读，标题栏仅绿灯闭锁／橙灯开锁，悬停说明模式；锁紧邻语言，点击不显示聚焦框。写入中禁用模式切换／外部打开／换仓库，但前两者保持外观；切回只读保留索引。
- About、多语言、图标、MIT 与文件图标许可随包分发；中英文 README 有真实浅／暗色截图、暂存／恢复说明及 Homebrew 安装／升级命令。主页覆盖九语言，默认英文，保留 `/zh/`；语言菜单标记当前选项，主题按钮直接切浅／暗色，首次跟随系统、手动选择持久化。顶部下载入口打开居中 dialog，平台按系统初始化，可键盘切换；macOS 提供 Homebrew 备选命令；各平台下载按钮下将 SHA-256 标签、校验值及复制按钮单行居中排列；窄屏省略显示，复制完整值，缺失或格式错误时构建失败。支持遮罩／Esc 关闭、焦点恢复、背景滚动锁定及减少动态效果。产品标题、canonical／hreflang、分享元数据及软件 JSON-LD 与可见内容／正式 Release 同源；三条原生折叠问答说明平台、写入边界和 SSH，可直接抓取。Sitemap 互链九语言；robots 仅在域名根部署时生成，项目 Pages 需从站长工具提交 sitemap。

### In Progress
- [paused] 提交说明局部高亮及实时拖选：用户决定等待 Kit Base 上游支持。当前 SelectableText 缺局部高亮 API，拖选变化未触发重绘；列表高亮已完成。

### Next
暂无已确认的后续范围。

## Known Problems and Risks
- P2 [risk]: diff 布局、上下文、滚动依赖 Kit fork；升级核对上游合入，保留消费方 `tests/diff_layout.rs`、`tests/scroll_bounce.rs` 回归。

## Open Decisions
暂无。

## Verification
- 最新正式版 [v0.1.6](https://github.com/worldtheater/patchlook/releases/tag/v0.1.6)（`96e3530`）已于 2026-11-02 发布，GitHub API 已确认非草稿；[正式构建 41443213925](https://github.com/worldtheater/patchlook/actions/runs/41443213925) 三平台检查、测试、构建及发布任务全部成功，四附件 SHA-256 已核对。macOS ZIP 内 App 及 DMG 容器的签名、公证票据已验证，ZIP 内 App 通过 Gatekeeper；DMG 内 App 未附带票据，Homebrew 解包后在云端 macOS 15 被拒绝，因此 Cask 使用 ZIP。Windows 内嵌 SSH worker 为 LF。Tap 已提交推送；更新脚本六项回归、真实 ZIP 下载／哈希及升级模拟、本机安装／卸载和在线审核通过，[Tap CI](https://github.com/worldtheater/homebrew-tap/actions/runs/41582810615) 在 macOS 15 完成安装、审核、签名、票据及 Gatekeeper 检查。
- 主页九项构建回归及 JS 语法已通过，覆盖九语言、链接／元数据、JSON-LD、sitemap、校验值与下载附件映射。桌面／320 px、浅暗色、弹窗／键盘／焦点恢复及复制实际内容已检查，下载浮层获用户认可；动画由用户验收。Pages 使用 GitHub Actions，校验值版 `fe30a43` 的[部署 41618166767](https://github.com/worldtheater/patchlook/actions/runs/41618166767)成功，九语言及资源与构建产物一致；Google 验证标签版 `4e2b99e` 的[部署 41642674355](https://github.com/worldtheater/patchlook/actions/runs/41642674355)成功，线上英文／中文 head 已核对。Search Console 验证由用户完成；尚无 sitemap 提交、实际收录或 AI 引用证据。
- 子模块导航已由用户验收；macOS fmt、Clippy、118 项常规串行回归及 9 项真实 Docker SSH／界面测试通过，覆盖未初始化与嵌套模块、特殊路径、父子切换／折叠、独立暂存／真实提交／撤销、持久化和 SSH 连接共享；测试容器支持重复启动。紧凑树状布局调整后，fmt／Clippy、子模块折叠与往返切换回归及 Release 构建通过；用户已确认当前界面及 SSH 子模块浏览正常；新增 Windows／Linux 客户端行为尚未实机验收。此前 macOS 已通过 Python 打包测试及 Release 构建；SSH 功能本地检查及 Windows 云端检查／打包已完成。SSH 覆盖密码／普通及加密私钥、主机信任及变更拒绝、凭据隔离、表单／历史／启动恢复、特殊路径／二进制／图片／分页、取消及断线重连、跨仓库复用、后台断开、窗口清理和远程写入拦截。用户已确认 macOS 经局域网连接 Windows WSL Ubuntu SSH 可用；用户也已确认 Windows 客户端 SSH 连接可用；Linux 客户端与网络延迟未实机验收。真实 Git 提交树／字节回归覆盖部分暂存、第三版本、目录／改名／二进制、CRLF、丢弃恢复、意向添加、SHA-1／SHA-256，撤销检查权限及外部冲突。遮罩动画在固定 fork revision 下构建，视觉效果由用户验收。
- 当前优化将普通部分暂存块的 Git 调用由 18 次降至 12 次（首次暂存 17 → 11）；同一行内／并排按钮样例，本机 macOS 调用链中位数约 183 → 116 ms，不代表 Windows 耗时。52 文件／37 二进制样例及文本／空文件／重命名／第三版本计数均与 Git 对照一致。写入前过期校验、写入后快照、撤销外部冲突检查保留。代码 `52ebf84` 已推送并纳入 v0.1.5 正式版；用户测试仓库和运行中的预览未改动。Windows 实际响应和视觉效果由用户验收。
- 旧 Windows Git 的 hunk 溢出已模拟复现并验证修复；LF／CRLF、历史／工作区／索引均覆盖。Windows／Linux 菜单的动作、焦点和拖动隔离有回归。首次布局修复已由用户确认 macOS 正常；未量化帧耗时，跨平台视觉及分栏拖动后重启恢复仍属手工验收边界。

## Source Pointers
- `README.md`、`README.zh-CN.md`、`docs/images/`、`docs/development.md` — 产品、截图、依赖、缓存及签名发布。
- `website/`、`.github/workflows/pages.yml` — 九语言静态主页、版本化下载与 GitHub Pages 构建发布；[worldtheater/homebrew-tap](https://github.com/worldtheater/homebrew-tap) — 独立维护的 macOS Cask、校验和正式版本自动同步。
- `Cargo.toml`、`Cargo.lock`、`src/git{.rs,/transport.rs,/ssh_worker.sh,/staging.rs,/review.rs,/undo.rs,/submodules.rs}`、`tests/{staging_commit,submodules}.rs` — 依赖及 Git 写入／恢复／提交契约。
- `src/{app,remote_repository,ssh_auth,ssh_host,change_tree,image_diff,file_icons,external_open,preferences,strings,about}.rs`、`locales/`、`themes/`、`assets/` — 界面与资源。
- `build.rs`、`tests/`、`scripts/{package,test_package}.py`、`.github/workflows/desktop.yml` — 平台与交付验证。
- `scripts/ssh_test.py`、`scripts/ssh-test/`、`tests/ssh_repository.rs` — SSH fixture、隔离密钥配置、真实远程只读及断线测试。
- `../patchlook_test/README.md`、该仓库 `tools/restore_scenarios.py` — 手工测试场景；恢复需显式 `--discard-changes`，仅作用于样例路径。
