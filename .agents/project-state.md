# Project State

## Purpose and Boundaries
- Patchlook 是 MIT 开源的本地 Git 查看器，Rust／GPUI Kit 实现，面向 macOS、Windows、Linux。默认只读；显式启用读写后支持文件／文件夹／连续改动块的暂存、取消暂存，以及确认后丢弃未暂存块；当前仓库、当前窗口内可撤销／重做。应用不执行 commit、checkout、fetch、pull 或 push，外部变更通过手动刷新读取。
- 依赖 PATH 中的 Git。文本 diff 要求 UTF-8；支持的图片独立预览，其他二进制展示变化信息。

## Current Architecture
- `src/main.rs` 初始化窗口、菜单、主题；包／二进制名 `patchlook`，标识 `dev.patchlook.app`。`build.rs` 将 ICO 嵌入 Windows EXE 资源 ID 1，失败中止构建；标题栏操作区隔离原生拖动命中。
- `src/git.rs` 封装 Git CLI，参数不经 shell，NUL 分隔路径，禁用可选锁、外部 diff 和 textconv。完整上下文取 `i32::MAX / 2`，inter-hunk context 固定 0，避免旧版 Windows Git 的整数溢出。
- `src/git/staging.rs` 以 add／reset 更新明确路径，以完整上下文补丁和 `git apply --cached` 更新改动块；丢弃反向应用到工作区，保留索引。部分操作在写入前统一核对状态与两侧显示补丁，拒绝过期选择，避免下层重复验证。只需改动列表时不查询分支；预览与进度复用同次读取的状态／补丁。`review.rs` 合并同路径条目，通常展示 HEAD → 工作区并标注块状态；索引有第三版本、抵消或无法匹配时分为 HEAD → 索引及索引 → 工作区两组，保留索引独有内容。
- `src/git/undo.rs` 记录写入前后的受影响索引条目或文件字节／权限；恢复前核对 HEAD 和相关快照，只恢复明确路径，不覆盖无关变化；成功的写入／撤销／重做返回受影响路径供刷新使用。保留 `git add -N` 意向添加标记，避免撤销暂存后误提交空文件；工作区恢复以临时文件替换。
- `src/app.rs` 的工作区／历史 `ChangeState` 独立保存列表、选择、树、滚动、diff、错误和任务；`HistoryState` 供 Repos／History 共用。只缓存当前仓库；已加载 Tab 切换不读 Git，隐藏页结果写回所属状态；切仓库／刷新成功后替换并使另一组失效，浏览分支只重载历史。列表、统计和首个预览后台准备后整体替换，失败保留可用快照。写入后读取新状态，重算受影响路径、状态变化项及当前预览，其他行复用已有统计／进度，避免随部分暂存或未跟踪文件数量反复读取 diff；手动刷新仍完整读取外部改动。
- 文本解析与虚拟滚动使用 Kit `DiffFile`／`DiffState`；视口／行高变化在绘制结束后通知，首次布局不依赖移动窗口。`src/image_diff.rs` 处理图片；`src/external_open.rs` 通过 NSWorkspace／Windows Shell／GIO 列出系统应用及图标，打开工作区最新文件；删除文件打开旧版本临时副本，保留至退出。
- `src/change_tree.rs` 压缩单一路径目录；已跟踪／未跟踪分组独立折叠、批量操作限定后代索引，避免同名目录跨组。`file_icons.rs` 内嵌 35 个 Material Icon Theme MIT SVG。`preferences.rs` 读写系统配置目录的 `preferences.json`，显式保存、临时文件替换。
- `strings.rs` 管理九语言资源；`themes/` 统一配色。浅色沿用 Kit 语法主题，仅正文及普通变量／属性／嵌入文本调深灰，深色自定义低饱和主题；正文取 `editor_foreground`。激活窗口重读系统语言，订阅系统外观。
- `src/about.rs` 复用 Kit Dialog／Presence，居中展示 logo、版本、简介、MIT、GitHub／反馈；淡出结束才卸载遮罩、恢复焦点，遵循减少动态效果。Windows／Linux 窗口内“≡”菜单仅含关于／退出，macOS 用系统菜单。
- `.github/workflows/desktop.yml` 在 main／PR 三平台检查及开发打包；匹配 Cargo 版本的 v 标签执行 macOS Developer ID 签名、公证、票据验证，三平台成功后创建 Release 草稿。`scripts/package.py` 默认 ad-hoc，正式签名失败不降级；手动输入 Apple submission ID 时仅查询既有公证。

## Project Conventions
- GPUI 统一从 `gpui_kit` 导入，复用控件与主题；九语言 `locales/*.json` 键及占位符一致。Kit 0.8.0 固定 fork revision `0917a5564505fceacfbbf966d90c0c19a7af9820`，已推送；维护 Cargo.lock，不提交本机 path override；升级见 `docs/development.md`。
- Git 底层未暂存为索引对工作区，已暂存为 HEAD 对索引；根提交按新增，合并提交比较第一父提交。历史／搜索固定首次解析的分支 tip；关键词匹配完整说明子串（忽略大小写）或 SHA 前缀，流式扫描，每页 100 条加一条前瞻。
- 界面测试共享进程级 locale，CI 与本地均串行运行测试，避免跨用例干扰。常规检查：`cargo fmt --all -- --check`、`cargo clippy --locked --all-targets -- -D warnings`、`cargo test --locked -- --test-threads=1`；构建变更另跑 Release；打包测试用 Python 3.11+ 执行 `python -m unittest discover -s scripts -p 'test_*.py'`。
- 自动化测试用临时仓库／配置；交互验收用 Release。用户授权 `../patchlook_test` 作为可直接暂存／取消／丢弃的测试场地，恢复说明在其 README。动画、短暂切换等视觉效果由用户验收。
- 缓存统一根 `target/`；dev 为 `line-tables-only`＋`split-debuginfo = "packed"`，test 继承 dev，保留增量，常用检查保持同一依赖特性组合。

## Implementation State
### Implemented
- Recent 按名称排序，访问不改顺序；加／减按钮添加移除记录，不删磁盘仓库。浏览分支按仓库保存，不改变 HEAD；Changes 始终工作区。顶部分支随视图，History 列表标题及树根不重复分支。
- Repos 三栏、Changes、History 共享缓存；Tab 保留选择／滚动／树展开／搜索。Repos Recent／Branches 默认等高，拖动比例持久化。历史 Load more 为尾部 item，未筛选超过 20 条常驻搜索框，Enter 查询、清空恢复、Esc 失焦；标题高亮关键词，SHA 不高亮。提交说明可选择及显隐；短 SHA 点击复制完整值，原地 Copied 两秒。
- 文件树／平铺、统计、键盘父子导航；所有文件选择均打开 diff。目录行及文本行框 22 px，图标与文字居中、紧凑间距；文件夹轻填充，图标树 16 px／平铺 20 px。修改／改名蓝、新增绿、删除灰、冲突警示；根路径显示 `.`，移动项显示相对来源。路径 tooltip 只绑名称，勾选不触发文件 tooltip、选择或折叠。
- 工作区同文件一条，空框／横线／对号表示未暂存／部分／全部，部分显示已暂存 x/y；文件夹／分组勾选仅作用于后代。Diff 右侧行号栏勾选连续块，左侧 `»` 确认丢弃，无单行入口或额外正文缩进。已暂存块保留 30% 背景强度，操作 hover／按下背景半透明；行号栏不触发整行选择，正文拖选保留。历史无写入口；二进制、非普通文件仅整文件操作，冲突交其他工具处理。
- 文件名右侧撤销／重做：一次文件／目录／块操作算一步，跨文件共用栈；新操作清空重做，刷新保留，切仓库／关窗口清空。可恢复丢弃的新文件，外部 HEAD／相关内容冲突拒绝恢复。暂存状态改变但补丁不变时仅刷新控件／背景；补丁变化以稳定源版本保留展开内容、视口偏移及横向滚动。
- Diff 支持并排／行内、语法与行内高亮、折行、同步滚动、复制、上下文折叠及块导航。单次选文件立即加载，120 ms 内快速切换才尾随防抖；详情保留旧内容至目标就绪，Repos 目录进新文件先显示目标加载状态。顶栏文件类型图标及外部打开菜单，无整文件收起按钮。图片支持尺寸、透明棋盘、适应／1:1／缩放滚动。
- 持久化外观、diff 布局、语言、文件列表样式、行号、折行、提交说明、Recent、各仓库浏览分支和 Repos 侧栏比例；其他分栏、窗口、Tab／选择／滚动／搜索不持久化。每窗口默认只读，标题栏仅绿灯闭锁／橙灯开锁，悬停说明模式；锁紧邻语言，点击不显示聚焦框。写入中禁用模式切换／外部打开／换仓库，但前两者保持外观；切回只读保留索引。
- About、多语言、图标、MIT 与文件图标许可随包分发；中英文 README 有真实浅／暗色截图及暂存／恢复说明。

### In Progress
- [active] 准备 v0.1.5 正式 Release 草稿：版本已更新，下一步推送标签，完成三平台构建及 macOS 签名／公证，核对资产和发布说明；保持 Draft，不公开发布。
- [paused] 提交说明局部高亮及实时拖选：用户决定等待 Kit Base 上游支持。当前 SelectableText 缺局部高亮 API，拖选变化未触发重绘；列表高亮已完成。

### Next
暂无已确认的后续范围。

## Known Problems and Risks
- P2 [risk]: diff 布局、上下文、滚动依赖 Kit fork；升级核对上游合入，保留消费方 `tests/diff_layout.rs`、`tests/scroll_bounce.rs` 回归。

## Open Decisions
暂无。

## Verification
- 当前 macOS 通过 fmt、Clippy、94 项串行 Rust 测试、Release 构建及 4 项 Python 打包测试。真实提交测试经生产 API及并排／行内按钮选择后执行普通 git commit，核对完整提交树／字节，覆盖部分选择、取消、原有暂存、第三版本、目录增删改名／二进制／特殊路径、CRLF／无末尾换行、丢弃与撤销重做、初始提交和意向添加。恢复测试覆盖权限、外部冲突、只读及历史栈边界。
- 暂存刷新性能优化已完成本地验证，本地 Release 已更新，`dist/Patchlook.app` 已替换、重新 ad-hoc 签名并启动供用户验收；用户测试仓库未改动。52 文件／37 二进制（含未跟踪及部分暂存）样例中，写入后刷新约 80–90 ms、完整刷新约 870–920 ms，统计／进度结果一致；这是本机 macOS 对照，未代表 Windows 耗时。小样例单次操作 Git 调用由 32 降为 18。视觉效果由用户验收，本轮无原生 UI 自动操作。最新性能优化已通过三平台云端检查和打包，用户已确认 Windows 暂存响应明显改善；Kit 独立测试此前受未缓存依赖阻挡，消费方覆盖所用交互。
- 开发构建 `c5f4424`（运行 `41075606168`）三平台检查、测试、Release 编译和打包通过；Windows 94 项 Rust 测试和 4 项 Python 打包测试通过。Windows x64 ZIP 已核对 artifact SHA256、压缩完整性、PE 架构和许可证；本地位于 `dist/windows-test-c5f4424/`，GitHub artifact `12987961156`。该包包含暂存刷新性能优化，沿用 0.1.4 元数据，不替换正式 Release。
- 最新正式版 v0.1.4（`c45ae39`）三平台 CI／打包通过并公开发布 Latest：macOS arm64 DMG／ZIP、Windows amd64 ZIP、Linux x86_64 tar.gz；macOS Developer ID、Hardened Runtime、公证、票据与 Gatekeeper 验证通过。v0.1.3 保持旧草稿。
- 旧 Windows Git 的 hunk 溢出已模拟复现并验证修复；LF／CRLF、历史／工作区／索引均覆盖。Windows／Linux 菜单的动作、焦点和拖动隔离有回归。首次布局修复已由用户确认 macOS 正常；未量化帧耗时，跨平台视觉及分栏拖动后重启恢复仍属手工验收边界。

## Source Pointers
- `README.md`、`README.zh-CN.md`、`docs/images/`、`docs/development.md` — 产品、截图、依赖、缓存及签名发布。
- `Cargo.toml`、`Cargo.lock`、`src/git{.rs,/staging.rs,/review.rs,/undo.rs}`、`tests/staging_commit.rs` — 依赖及 Git 写入／恢复／提交契约。
- `src/{app,change_tree,image_diff,file_icons,external_open,preferences,strings,about}.rs`、`locales/`、`themes/`、`assets/` — 界面与资源。
- `build.rs`、`tests/`、`scripts/{package,test_package}.py`、`.github/workflows/desktop.yml` — 平台与交付验证。
- `../patchlook_test/README.md`、该仓库 `tools/restore_scenarios.py` — 手工测试场景；恢复需显式 `--discard-changes`，仅作用于样例路径。
