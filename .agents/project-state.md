# Project State

## Purpose and Boundaries
- Patchlook 是 Rust／GPUI Kit 实现的本地 Git 查看器，目标为 macOS、Windows、Linux，采用 MIT 协议，无后台服务。默认只读浏览工作区、暂存区、分支与提交；显式切换读写模式后可暂存／取消暂存，并在确认后丢弃未暂存改动块；暂存操作本身不修改工作区，不执行提交、checkout、fetch、pull 或 push。外部变更通过手动刷新读取。
- 文本 diff 要求 UTF-8；支持的图片独立预览，其他二进制只展示变化信息。运行需要 PATH 中可用的 Git。

## Current Architecture
- Cargo 包／二进制为 `patchlook`，应用标识 `dev.patchlook.app`；`src/main.rs` 初始化窗口、主题、菜单和快捷键。`build.rs` 将多尺寸 ICO 以 GPUI 需要的资源 ID 1 嵌入 Windows EXE，资源编译失败会中止构建。标题栏按钮隔离原生拖动命中，语言下拉以完整触发区隔离，空白区域保留拖动。
- `src/git.rs` 封装 Git CLI，参数不经过 shell；NUL 分隔保留特殊路径，关闭可选锁、外部 diff 和 textconv；全文件上下文使用 `i32::MAX / 2` 并固定 inter-hunk context 为 0，避免旧版 Windows Git 的 32 位整数溢出和重叠 hunk。`src/git/staging.rs` 校验访问模式，以 Git add／reset 更新整文件索引，文件夹批量操作合并明确的文件路径并一次执行，以完整上下文补丁和 `git apply --cached` 更新连续改动块；丢弃用反向块补丁作用于工作区，保持索引和其他改动不变。两类部分操作均核对显示补丁，拒绝过期选择。`src/git/review.rs` 合并同路径的工作区条目，按源坐标及块内容匹配暂存状态：通常展示 HEAD 对工作区的完整 diff；暂存区包含第三版本、抵消后的索引独有改动或无法一一匹配时，明确展示 HEAD 对暂存区和暂存区对工作区两组比较，避免隐藏索引内容。块写入同时校验已暂存和未暂存快照，仍使用真实版本补丁执行。
- `src/app.rs` 管理共享仓库与页面。`ChangeState` 分别持有工作区／历史的列表、选择、树、滚动、diff、错误与任务；`HistoryState` 管理 Repos／History 共用的提交、搜索和分页。只缓存当前仓库，隐藏页结果写回所属状态；已加载 Tab 切换不读 Git。切仓库／刷新成功后替换当前页并使另一组状态失效，浏览分支只重载历史。
- 新列表、统计与首个预览在后台一起准备，再替换可见内容；读取失败保留快照，预览失败不丢列表。文本 patch 含完整上下文，解析、选择与虚拟滚动由 Kit `DiffFile`／`DiffState` 提供；Diff 视口／行高测量变化在绘制结束后通知，保证首次布局自行完成，不依赖高亮完成或窗口移动带动刷新；图片经 `Repository::file_versions` 和 `src/image_diff.rs` 读取、解码、渲染。
- `src/external_open.rs` 封装外部打开：macOS 使用 NSWorkspace／UTType，Windows 使用 Shell，Linux 使用 GIO。推荐程序携带系统图标，无法取得时用通用图标。普通文件打开工作区最新路径；删除条目读取对应 diff 的原版本到临时副本，由应用保留至退出，不恢复工作区。菜单捕获目标，切换加载期间禁用。
- `src/change_tree.rs` 压缩单一路径目录，按显示文件索引保留身份；工作区同路径只显示一条，已跟踪改动／未跟踪文件为可独立折叠的分组。分组／目录按后代索引限定批量操作，不因同名目录跨组选中文件；展开状态以分组与路径区分，语言变化同步分组标题。`src/file_icons.rs` 共用文件类型映射，内嵌 35 个 Material Icon Theme MIT SVG，未知类型用文档图标。
- `src/preferences.rs` 在系统 `patchlook` 配置目录读写 `preferences.json`，读取与显式保存分离，写入通过临时文件重命名完成。
- `src/strings.rs` 管理九语言资源并与 Kit 共用 locale；`themes/` 提供统一配色，浅色沿用 Kit 默认语法配色，仅将正文和普通变量／属性／嵌入文本调为深灰，深色使用自定义低饱和度主题；Diff 正文读取主题的 `editor_foreground`（当前 Syntect 后端的 JSON 字段名）。窗口重新激活重读系统语言，系统外观通过订阅更新。
- `src/about.rs` 复用 Kit Base Dialog／Presence，在窗口内居中显示关于页和协议；淡出结束才卸载遮罩、恢复焦点，遵循减少动态效果设置。版本、仓库与协议标识来自 Cargo 元数据。
- `.github/workflows/desktop.yml` 在 main／PR 执行三平台检查和开发打包；匹配 Cargo 版本的 v 标签触发 macOS Developer ID 签名、公证和票据验证，三平台通过后创建 Release 草稿。`scripts/package.py` 默认 ad-hoc，正式签名失败不降级。手动工作流的 `notary_submission_id` 只查询既有 Apple 提交，不构建或上传。

## Project Conventions
- GPUI 统一从 `gpui_kit` 导入，复用控件与主题；界面文案集中于 `locales/*.json`，九语言键及占位符一致。
- GPUI Kit 0.8.0 固定到 fork revision `0917a5564505fceacfbbf966d90c0c19a7af9820`；维护 `Cargo.lock`，不提交本机 path override。补丁归属和升级步骤见 `docs/development.md`。
- Git 基准：底层未暂存为索引对工作区，暂存为 HEAD 对索引；工作区审阅通常合并为 HEAD 对工作区并标注每块的暂存状态，三个版本无法合并时明确分开展示；根提交按新增，合并提交比较第一父提交。历史／搜索续页固定首次解析的分支 tip；关键词匹配提交说明子串（忽略大小写）或 SHA 前缀，流式扫描可达历史，每页最多 100 条加一条前瞻。
- 检查：`cargo fmt --all -- --check`、`cargo clippy --locked --all-targets -- -D warnings`、`cargo test --locked`；构建／依赖变更另做 `cargo build --locked --release`，打包测试为 `python -m unittest discover -s scripts -p 'test_*.py'`。自动化 Git 和界面测试使用临时仓库／配置；交互验收使用 Release。用户另指定兄弟仓库 `../patchlook_test` 为可直接进行暂存／取消暂存／丢弃验证的专用测试场地；样例与恢复方式见该仓库 `README.md`。
- 动画、瞬时状态切换等难以自动验收的视觉效果由用户直接验收；完成实现、必要行为回归和构建后提供预览，不代替用户做这类视觉验收。
- 缓存统一用根 `target/`，dev 为 `line-tables-only`＋`split-debuginfo = "packed"`，test 继承 dev，保留增量编译；常用检查保持同一测试依赖特性组合，清理步骤见开发文档。

## Implementation State
### Implemented
- 最近仓库按名称忽略大小写排序，同名按路径排序；访问不改变顺序。Recent 加／减号可添加、移除记录，移除后选择相邻仓库，最后一项清空内容，不删磁盘文件。浏览分支按仓库保存，不改变实际 HEAD；Changes 始终对应工作区。窗口顶部分支跟随视图：Repos／History 显示浏览分支，Changes 显示工作区分支，悬停说明两者含义；History 的提交列表标题和文件树根节点不再重复显示分支名。
- Repos 三栏概览、Changes、History 共用上述状态；Tab 切换保留选择、滚动、目录展开和搜索，历史预览按需加载一次。Repos 的 Recent／Branches 默认等高，拖动结束保存比例，重启／缩放窗口后恢复。
- 历史分页的 Load more 是尾部条目；未筛选提交超过 20 条时常驻搜索框，筛选后仍保留，Enter／搜索按钮查询完整分支，清空恢复普通历史，Esc 失焦。列表标题高亮匹配词并保留 UTF-8 边界，SHA 前缀不高亮。提交说明可复制、显示／隐藏；短 SHA 悬停、点击复制完整号，原地显示 Copied 两秒。
- 文件列表支持目录和平铺、统计、键盘定位。左键先折叠再选父目录，右键先展开再选下一可见项；所有选中文件行为均打开 diff。目录行高 22 px，行内图标与文字统一居中、紧凑间距；文件夹采用轻填充图标，与空心勾选按钮区分，目录图标 16 px、平铺 20 px；文件名修改／重命名蓝色、新增／复制绿色、删除灰色、冲突警示色，根目录路径显示 `.`。移动／改名项显示相对来源 `← 来源`，同名移动只显示原目录，悬停／读屏提供完整说明。读写模式以有 hover 反馈的空心方框／带框对号按钮表示文件和连续改动块的暂存状态，适配亮暗主题；同一文件只显示一条，以空框／横线／对号表示未暂存／部分／全部暂存，部分文件右侧显示“已暂存 x/y”改动块进度。已暂存块保留 diff 背景，以未暂存块 30% 的背景强度显示（含行内强调），取消暂存后恢复强调；全部暂存时也保留完整 diff。文件夹（含根目录）用空框／横线／对号表示未暂存／部分／全部暂存，未全选时一键暂存所有后代的剩余改动，全选后取消全部暂存；未跟踪文件独立分组，分组勾选仅作用于组内文件；勾选点击不触发目录折叠或文件选择。Diff 右侧行号栏勾选整段，左侧 `»` 按钮确认后丢弃未暂存整段，新增文件整段丢弃会删除文件；撤回与确认按钮共用半透明悬停／按下背景，透出底层 diff 颜色；浅色模式使用更低的不透明度。无单行入口、不增加正文缩进。Diff 操作槽隔离鼠标按下和底层行号命中；Patchlook 关闭行号栏点击／拖动选整行，确认框与行号左侧空白不再触发整行蓝色选区，正文拖选仍保留。历史无写入口；二进制及非普通文件只支持整文件，冲突交由其他 Git 工具处理。
- 单文件文本 diff 支持并排／行内、语法与行内变化高亮、折行、同步横向滚动、选择复制、上下文展开／收起及改动块计数。顶栏显示文件类型图标与打开方式菜单（系统默认、推荐应用图标＋名称、其他程序），无整文件收起按钮。单次选择立即加载；120 ms 内连续选择才尾随防抖并取消旧任务，工作区／历史分别记录选择时间；详情切文件保留旧预览至新内容就绪，Repos 目录进入另一文件先显示目标加载状态，已完成的同文件预览复用。暂存状态变化且显示补丁相同时仅更新控件与背景，不重建 DiffState；内容实际变化时，以未变化的源版本保留已展开内容、视口像素偏移和横向滚动，不把点击行移到顶部。
- PNG、JPEG、WebP、GIF、BMP、TIFF、ICO、SVG 前后预览，含尺寸／大小、透明棋盘格、适应窗口、1:1、缩放与滚动。
- 持久化外观、diff 模式、语言、列表模式、行数显示、折行、提交说明开关、最近仓库、各仓库浏览分支、Repos 侧栏高度比例；其他分栏尺寸、窗口位置大小、Tab、选中项、滚动和搜索不持久化。访问模式仅当前窗口有效；标题栏按“指示灯、只读文字、开关”排列，开关使用按明暗主题调节对比度的中性色轨道、亮色滑块与共用组件的超小尺寸（24×14 px），不显示聚焦框；每个新窗口默认开启，指示灯显示绿色，关闭后进入读写模式并显示橙色。切回只读保留暂存区内容，写入进行中禁止切模式、打开外部文件和换仓库；只读开关在短暂写入期间不绑定切换回调且不参与 Tab 聚焦，状态与外观保持不变，领域设置入口同样阻止切换；外部打开按钮也维持原有颜色和选中背景，避免短暂变暗。九语言文案和中英文 README 已同步开关操作。真正缺少可打开文件等不可用状态仍显示正常禁用外观。
- Windows 和 Linux 标题栏左侧提供“≡”应用菜单，仅含“关于 Patchlook”和“退出 Patchlook”，复用现有动作并隔离原生拖动命中；macOS 使用系统菜单。关于页含 logo、版本、九语言简介、MIT 正文、GitHub／反馈链接；关闭按钮、Esc、遮罩均淡出。根 LICENSE 和文件图标许可证随三平台包分发。

### In Progress
- [paused] 提交说明局部高亮与实时拖选：用户决定不改 Kit Base，待上游支持／修复后再评估。当前 `SelectableText` 缺局部高亮 API，拖选变化未触发窗口重绘；列表标题高亮已完成。

### Next
暂无已确认的后续范围。

## Known Problems and Risks
- P2 [risk]: diff 布局、上下文与滚动依赖 Kit fork；升级时核对上游合入情况，保留 `tests/diff_layout.rs`、`tests/scroll_bounce.rs` 的消费方回归。

## Open Decisions
暂无待确认的决策。

## Verification
- 提交前 macOS 验证通过 fmt、Clippy、79 项串行 Rust 测试和 Release 构建；系统推荐程序测试在可访问 macOS 系统服务的环境运行。开关覆盖默认开启、点击与访问权限对应、写入期间不能切换、标题栏拖动命中隔离；暂存回归覆盖只读约束、分组／文件夹范围、同文件三态与进度、整段暂存／取消、索引独有及第三版本保留、初始仓库、过期补丁拒绝、丢弃确认／取消。并排／行内验证 gutter 操作不触发文本选择、正文拖选正常、块操作保留源位置、上下文及滚动。当前 Release 预览已更新，用户已确认本轮只读开关及按钮视觉调整；瞬时视觉效果由用户直接验收。Kit 组件补丁已单独提交并推送，依赖固定精确 revision，无本机路径覆盖。Windows／Linux 未做本轮实机验收。Kit 独立组件单测此前被未缓存的 quickjs-jit 工作区依赖阻挡，消费方回归验证组件交互。
- v0.1.4（`c45ae39`）云端运行 `40717259379` 三平台格式、Clippy、测试、Release 构建和打包全部通过；macOS／Windows 55 项、Linux 54 项 Rust 测试，各 4 项 Python 打包测试。正式包包含 macOS arm64 DMG／ZIP、Windows amd64 ZIP、Linux x86_64 tar.gz，已公开发布到 GitHub Releases 并标为 Latest；v0.1.3 保持旧草稿。
- macOS Developer ID／Hardened Runtime 签名、公证与票据验证通过；Apple 提交 `844cc354-8dc1-41cc-9e5f-3fb42165a38c` 为 Accepted。本地 `dist/v0.1.4/` 核对源提交、CI 摘要、版本、许可证及各平台包结构；应用／DMG 票据、Gatekeeper 和 DMG 完整性通过，Windows x64 PE 资源组 ID 1 和 7 种图标尺寸与源 ICO 一致，Linux ELF 与执行权限正常；公开 Release 的四项资产摘要与本地验证结果一致。
- Windows patch 回归覆盖 LF／CRLF、未暂存／已暂存／历史及极大 inter-hunk 配置；旧版 Git 2.49 的 32 位 long 模拟复现第 60 行重叠并验证修复。Windows／Linux 菜单回归覆盖仅两项操作、Tab／Enter、Esc 恢复焦点、关于页、退出动作及标题栏拖动命中隔离；macOS 实际标题栏不含该菜单。
- Diff 首次布局修复 `a3080ee7` 已由用户确认 macOS 表现正常；消费方回归在旧依赖失败、修复后通过，验证测量后通知及布局稳定后停止重绘。此前 macOS 预览覆盖分支标签、配色、Tab、搜索、复制、偏好、About 与外部打开；真实拖动分栏后重启未验收，未量化帧耗时，Windows／Linux 新菜单与首次 diff 显示的实机效果待确认。

## Source Pointers
- `README.md`、`README.zh-CN.md`、`docs/images/`、`docs/development.md` — 默认英文及简体中文产品介绍、正式版实机浅／暗色行内 diff 截图合成、依赖、验证、缓存、签名发布及 Kit 补丁。
- `Cargo.toml`、`Cargo.lock` — 版本、构建配置及依赖来源。
- `../patchlook_test/README.md`、`tools/restore_scenarios.py`（测试仓库内）— 可反复恢复的手工验证样例，含三类连续块、部分暂存、新增／删除／移动、图片／二进制、特殊路径与换行、26 条历史及分叉分支；恢复只作用于样例路径并要求显式 `--discard-changes`。
- `src/git.rs`、`src/git/staging.rs`、`src/git/review.rs`、`src/external_open.rs` — Git／文件版本与暂存契约、系统应用关联及回归。
- `src/app.rs`、`src/change_tree.rs`、`src/image_diff.rs`、`src/file_icons.rs`、`assets/ui-icons/` — 状态、界面、导航、预览和应用图标。
- `src/preferences.rs`、`src/strings.rs`、`locales/`、`themes/` — 设置、语言和样式。
- `src/about.rs`、`LICENSE`、`assets/app-icon/` — About、许可证和正式图标源 `patchlook-v2.png`。
- `build.rs`、`tests/`、`scripts/test_package.py`、`scripts/package.py`、`.github/workflows/desktop.yml` — Windows 图标资源、验证和三平台打包发布。
