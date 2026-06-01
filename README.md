# seg

macOS 菜单栏小工具，用一条时间轴可视化你最近的 [Codex CLI](https://github.com/openai/codex) 和 [Claude Code](https://github.com/anthropics/claude-code) 会话——什么时候开始、什么时候在干活、什么时候停了。

完全本地：解析磁盘上已有的 jsonl，不联网、不上传、不写入第三方进程。

## 它扫描什么

- `~/.codex/sessions/**/*.jsonl` — Codex CLI 写出来的 rollout 文件
- `~/.claude/projects/**/*.jsonl` — Claude Code 写出来的对话文件

只读，不修改这些文件。

## 视图说明

每张卡片代表一次会话：

```
●  deal      CLAUDE  PLAN  claude-opus-4-7              59s ago
   ▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔▔
```

- 左侧圆点状态：running（黄、还在写）、finished（绿、10 分钟内停）、stopped（灰）
- 项目名取自会话的 `cwd` basename
- chip：source（codex / claude）、PLAN（用过 plan 模式）、acceptEdits / bypassPermissions、模型名、reasoning effort
- 时间条：会话整体跨度，里面的色块是一个个**段（segment）**

### 段（segment）的定义

一段从**一条真正的用户消息**开始，持续到下一条用户消息之前的最后一个**对话事件**。段内的模型回复、tool 调用、tool 返回都算作该段的活动，不会再细分。

段尾只认对话事件：CLI 在你提交下一条 prompt 时会顺带写入一批簿记行（Claude 的 `attachment` / `file-history-snapshot` / `permission-mode`，Codex 的 `event_msg` 如 `task_started`），它们带的是下一段的时间戳、却排在用户消息前约 1ms。若把它们算进段尾，挂机过夜的空闲就会被并入上一段，让一个 5 分钟的 turn 显示成 16 小时。因此这些行不参与段的起止判定（但仍计入会话整体跨度与状态）。

Claude jsonl 里有三种 `type:"user"` 的行：真实用户输入、CLI 注入的 `<local-command-...>` 提示、以及反馈给模型的 tool_result。后两者会被过滤，不作为段起点。

### 状态判定

`running` / `finished` / `stopped` 由文件里**最后一条事件的时间戳**决定，不看 mtime——这样从其他机器导入或同步过来的旧会话不会被误判为活跃。

- ≤ 60 秒：running
- ≤ 10 分钟：finished
- 否则：stopped

## 设置

点右上齿轮打开内嵌设置面板：

| 项 | 选项 | 默认 |
| --- | --- | --- |
| Sort | Duration（展示段时长降序）/ Recent（最后事件时间降序） | Duration |
| Show | 20 / 50 / 100 / All | 50 |
| Refresh | Off / 1m / 5m / 10m 自动重扫 | 5m |

配置写到 `localStorage['seg:settings:v1']`，重启保留。

## 过滤

设置面板下方那行 chip 是过滤标签：
- 项目名（按出现次数降序）
- mode（plan、acceptEdits、bypassPermissions 等）

点击切换；多个 chip 之间 OR within category，AND across categories；右侧 `clear ×` 一键清空。

## 构建 / 安装

需要 `node` + `npm` + `rust`（cargo）。

```bash
# 启动开发模式
npm run tauri dev

# 打包 .app
./build.sh
# 输出在 src-tauri/target/release/bundle/macos/seg.app

# 复制到 /Applications
./install.sh
```

启动后图标会出现在系统菜单栏。点击图标弹出/收起窗口，单击窗口外或失焦自动收起。`Quit seg` 在托盘菜单里。

## 项目结构

```
src/                  SvelteKit 前端（一个 +page.svelte）
src-tauri/src/
  ├── lib.rs          托盘 + 窗口 + macOS 集成
  ├── sessions.rs     段/状态/Session 类型，build_segments 核心逻辑
  ├── codex.rs        扫描 ~/.codex/sessions 的解析器
  └── claude.rs       扫描 ~/.claude/projects 的解析器
```

前端只调用一个 Tauri 命令：`list_sessions`，返回 `Session[]`。

## License

MIT
