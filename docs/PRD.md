# Spellbook — 产品需求文档（PRD v1.0）

> 一个专门给开发者记录和快速查找常用命令的本地桌面软件。
>
> 维护：2026-05-20 · 状态：草案

---

## 目录

1. [项目概述](#1-项目概述)
2. [设计原则与技术目标](#2-设计原则与技术目标)
3. [核心功能（MVP / v1）](#3-核心功能mvp--v1)
4. [UI 设计](#4-ui-设计)
5. [数据模型](#5-数据模型)
6. [快捷键](#6-快捷键)
7. [智能字段推断（标题 / 分类 / 标签）](#7-智能字段推断标题--分类--标签)
8. [AI / LLM 功能（v2+，可选）](#8-ai--llm-功能v2可选)
9. [非目标（不做）](#9-非目标不做)
10. [开发路线图](#10-开发路线图)
11. [风险与开放问题](#11-风险与开放问题)

---

## 1. 项目概述

### 1.1 项目定位

一个本地优先、单文件即可运行的桌面工具，帮开发者把日常工作中"用一次记一次、记了又忘"的命令归档、检索、复用。

### 1.2 命名

**正式名称：Spellbook**

把 CLI 命令视作"咒语"是开发/运维社区已有的隐喻，产品本质就是用户的私人咒语本。
名字短、不限工种（开发/运维/数据都说得通）、中文友好（咒语本 / 法术书）。

二进制 / 包名 / 配置目录均使用 `spellbook` / `Spellbook`。

### 1.3 目标用户

- 后端 / 全栈开发者
- 运维 / DevOps / SRE
- Linux 重度用户
- 一切"每天敲一堆命令"的人

### 1.4 解决的问题

- 命令记不住，每次都要 Google
- 笔记散落在 Markdown / 飞书 / Notion / 浏览器收藏夹里
- VSCode 太重、Obsidian 太泛、终端历史不够结构化
- 文档和代码编辑器混在一起，找命令要切窗口
- 复制命令效率低（要选中 + 复制 + 切窗口 + 粘贴）

---

## 2. 设计原则与技术目标

### 2.1 设计原则

- **本地优先**：无账号、无登录、无强制联网
- **轻量**：单文件可执行，绿色版即可运行
- **秒开**：冷启动 < 500ms
- **聚焦命令**：不是通用笔记，不是 Markdown 编辑器
- **键盘优先**：核心动作都能用快捷键完成
- **可降级**：AI 功能可选，关掉后核心体验完全不受影响

### 2.2 技术栈

| 层 | 选型 |
|---|---|
| 语言 | Rust |
| GUI | egui / eframe |
| 存储 | SQLite（`rusqlite`） |
| 模糊匹配 | `fuzzy-matcher` (`SkimMatcherV2`) |
| Markdown 渲染 | `egui_commonmark` 或 `pulldown-cmark` |
| 序列化 | `serde` / `serde_json` |
| 时间 | `chrono` |
| LLM（可选） | `reqwest` + 自实现 OpenAI 兼容客户端 |

### 2.3 性能目标

| 指标 | 目标 |
|---|---|
| 冷启动 | < 500ms |
| 首屏可交互 | < 800ms |
| 搜索响应（1w 条） | < 30ms（输入即过滤、无回车） |
| 内存占用（空闲） | < 100MB |
| 安装包体积 | < 20MB |

### 2.4 平台支持

- v1：macOS、Windows、Linux（egui 三端通吃）
- 数据库 / 配置存放位置：
  - macOS：`~/Library/Application Support/Spellbook/`
  - Windows：`%APPDATA%\Spellbook\`
  - Linux：`~/.local/share/Spellbook/`

---

## 3. 核心功能（MVP / v1）

### 3.1 命令管理

#### 3.1.1 极速新增（核心 UX）

> **设计目标**：从浏览器/终端复制一段命令 → 切到 Spellbook → **1 步**即保存。
> 不强制填标题、说明、标签，所有字段由规则引擎自动推断，能改但不必改。

**入口**：`Ctrl/Cmd + N` 唤起快速捕获浮窗（打开时自动读剪贴板）。

**两层补全**：

1. **规则层（同步）** — 粘贴后 ~5ms 内填好 title / category / tags（来自 [§7.2](#72-v1-规则引擎纯本地零延迟)）
2. **AI 层（异步，可选）** — 如启用了"粘贴即增强"，后台调用 LLM 把字段升级为更好的版本（人类可读标题、参数说明、补充标签）；返回前用户照常编辑、照常保存（详见 [§7.3](#73-v2-ai-后台增强粘贴即增强可选)）

**UI — 粘贴后立即**（规则层结果，AI 异步进行中）：

```
┌──────────────────────────────────────────────────────────────┐
│ ✨ 新建命令                          🧠 AI 增强中... [取消]   │
│ ┌──────────────────────────────────────────────────────────┐ │
│ │ docker container prune -f                                │ │
│ └──────────────────────────────────────────────────────────┘ │
│                                                              │
│   标题：docker container prune -f             [✎ 改]         │
│   分类：Docker                                 [▾ 换]         │
│   标签：[docker] [container] [cleanup]        [+ 加]         │
│   说明：(等待 AI 补全 ...)                                   │
│                                                              │
│              [Enter 保存]                    [Esc 取消]      │
└──────────────────────────────────────────────────────────────┘
```

**UI — AI 返回后**（用户未编辑过的字段被静默升级，带 🧠 标记）：

```
┌──────────────────────────────────────────────────────────────┐
│ ✨ 新建命令                                  🧠 已增强        │
│ ┌──────────────────────────────────────────────────────────┐ │
│ │ docker container prune -f                                │ │
│ └──────────────────────────────────────────────────────────┘ │
│                                                              │
│   标题：删除所有已停止的 docker 容器  🧠      [✎ 改]         │
│   分类：Docker                                 [▾ 换]         │
│   标签：[docker] [container] [cleanup] [prune] 🧠 [+ 加]    │
│   说明：删除所有处于 stopped 状态的容器...  🧠 (展开)        │
│                                                              │
│              [Enter 保存]                    [Esc 取消]      │
└──────────────────────────────────────────────────────────────┘
```

**关键体验**：

- 浮窗打开 → 自动粘贴 → 规则层瞬时填好 → AI 层异步增强（如启用）
- **用户已编辑过的字段绝不被 LLM 覆盖**（详见 [§7.3.3](#733-覆盖策略--用户至上)）
- 用户按 `Enter` 保存时如果 LLM 还没返回 → 立即保存当前值，LLM 结果用 toast 询问是否合并（详见 [§7.3.4](#734-保存时-llm-未返回怎么办)）
- 默认值可改、可忽略；`description` 允许留空、不阻塞保存
- **重复检测**：库里已有完全相同的 `command`，提示"已存在，是否打开"

**字段补全来源**（详细规则见 [§7](#7-智能字段推断标题--分类--标签)）：

| 字段 | 规则层（同步） | AI 层（异步，可选） |
|---|---|---|
| `title` | 命令前 `#` 注释行 → 首行截断 | 生成简洁中文标题（如"删除所有停止的容器"） |
| `category` | 工具→分类映射表，未命中 `Other` | 当映射未命中时由 LLM 推荐 |
| `tags` | 工具名 + 动词 + 子命令 + 端口/协议 | 补充非显然的标签（如 `safe-by-default`） |
| `description` | 留空 | 生成 Markdown 说明 + 逐参数解释 |

**多行命令**：

输入：

```
# 备份当前目录为带日期的 tar.gz
tar -czf backup-$(date +%F).tar.gz .
```

识别：
- title = `备份当前目录为带日期的 tar.gz`（取注释）
- command = `tar -czf backup-$(date +%F).tar.gz .`
- category = `Linux`（`tar` → Linux）
- tags = `[tar] [backup]`

#### 3.1.2 完整编辑（次要入口）

当用户需要精修说明、调整字段时，从详情页 `[编辑]` 按钮进入完整编辑页（左 Markdown 编辑、右预览）。

#### 3.1.3 字段定义

| 字段 | 类型 | 说明 |
|---|---|---|
| title | String | 命令标题，必填（可自动推断） |
| command | String | 命令本身，支持多行 |
| description | String | Markdown 格式的说明，可空 |
| category_id | i64 | 所属分类 |
| tags | Vec\<String\> | 标签，多个 |
| favorite | bool | 是否收藏 |
| visit_count | i32 | 访问次数（用于排序） |
| created_at / updated_at | DateTime | 时间戳 |

示例：

```
标题：docker 删除所有停止容器
命令：docker container prune -f
说明：删除所有处于 stopped 状态的容器，不影响运行中的。
分类：Docker
标签：cleanup, docker
收藏：true
```

**操作集**：新增（极速） / 编辑（完整） / 删除 / 复制 / 收藏 / 移动分类。

### 3.2 分类管理

左侧树状结构，自带数量统计：

```
Git (81)
Docker (34)
Linux (123)
SSH (12)
MySQL (28)
Redis (15)
Python (44)
Rust (9)
NodeJS (33)
FFmpeg (17)
```

支持：

- 新增 / 重命名 / 删除分类
- 拖拽排序
- 单层（v1 不做嵌套）

### 3.3 极速模糊搜索（核心）

**搜索范围**：title、command、description、tags、category_name。

**搜索类型**：

- 普通子串匹配（`docker`、`git remote`、`nginx`）
- 模糊匹配（`dkps` → `docker ps`，`gtrm` → `git remote`，`sshkp` → `ssh keygen password`）

**权重设计**：

| 字段 | 权重 |
|---|---|
| title | 10 |
| tag | 8 |
| command | 6 |
| category | 4 |
| description | 2 |

**排序优先级**：完全匹配 > 前缀匹配 > 模糊匹配。次级按 `visit_count desc, updated_at desc`。

**性能**：1w 条命令仍要丝滑（输入即过滤，无回车确认）。

**实现要点**：内存中维护倒排索引或直接全量打分（1w 量级足够），不依赖 SQLite FTS。

### 3.4 一键复制（核心）

- 命令详情页一个显眼的 `[复制命令]` 按钮
- 双击命令列表项 = 复制
- 快捷键 `Cmd/Ctrl + Shift + C` = 复制当前选中命令
- 复制成功后右下角弹 toast：`已复制`
- 复制后 `visit_count + 1`，用于"最近使用"排序

### 3.5 Markdown 说明

- 编辑区：纯文本 Markdown
- 预览区：渲染后的视图
- 布局：左右 split view
- 支持：标题、代码块、列表、表格、链接、行内代码
- **不做**：富文本所见即所得编辑器

### 3.6 收藏

- 每条命令一个 `★` 收藏按钮
- 列表顶部固定区块：`Favorite Commands`
- 收藏数无上限

### 3.7 历史记录 / 最近使用

- 顶部固定区块：`Recent`，显示最近访问的 10 条命令
- 排序依据：`visit_count` + 时间衰减

### 3.8 智能字段推断（v1 规则版）

> 极速新增的"大脑"。基于命令文本，**纯本地、零延迟**地推断 title / category / tags。
> 详细设计见 [§7](#7-智能字段推断标题--分类--标签)。LLM 增强（生成 title、description）放到 v2。

---

## 4. UI 设计

### 4.1 整体布局（三栏）

```
┌─────────────┬──────────────────┬─────────────────────────┐
│ 分类         │ 命令列表          │ 详情                     │
│             │                  │                         │
│ ★ Favorites │ git remote -v    │ # 标题                   │
│ ⏱ Recent    │ docker ps        │                         │
│             │ docker logs      │ ```bash                 │
│ Git         │ ssh-copy-id      │ docker container prune  │
│ Docker      │ ...              │ ```                     │
│ Linux       │                  │ [复制]                  │
│ SSH         │                  │                         │
│ ...         │                  │ ## 说明（markdown 渲染）│
│             │                  │                         │
│ + 新建分类   │                  │ 标签：cleanup, docker   │
└─────────────┴──────────────────┴─────────────────────────┘
```

### 4.2 顶部搜索栏

```
┌────────────────────────────────────────────────────────┐
│ 🔍 搜索命令、标题、标签...                Ctrl+K        │
└────────────────────────────────────────────────────────┘
```

- `Ctrl/Cmd + K` 全局聚焦搜索框
- 输入即过滤；`Esc` 清空并失焦
- 上下方向键在结果中移动，`Enter` 复制并关闭

### 4.3 体验参考

- Raycast（搜索唤起感）
- VSCode Command Palette（键盘流）
- Everything（极速过滤）
- Dash（开发者文档）

---

## 5. 数据模型

### 5.1 Rust 结构体

```rust
pub struct CommandNote {
    pub id: i64,
    pub title: String,
    pub command: String,
    pub description: String,
    pub category_id: i64,
    pub tags: Vec<String>,
    pub favorite: bool,
    pub visit_count: i32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

pub struct Category {
    pub id: i64,
    pub name: String,
    pub sort_order: i32,
}
```

### 5.2 SQLite Schema

```sql
CREATE TABLE category (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL UNIQUE,
    sort_order INTEGER DEFAULT 0
);

CREATE TABLE command_note (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    title TEXT NOT NULL,
    command TEXT NOT NULL,
    description TEXT DEFAULT '',
    category_id INTEGER REFERENCES category(id) ON DELETE SET NULL,
    tags TEXT DEFAULT '',          -- 逗号分隔，简单粗暴
    favorite INTEGER DEFAULT 0,
    visit_count INTEGER DEFAULT 0,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE INDEX idx_cmd_category ON command_note(category_id);
CREATE INDEX idx_cmd_favorite ON command_note(favorite);
CREATE INDEX idx_cmd_updated  ON command_note(updated_at DESC);
```

> tags 用逗号分隔字符串存。1w 量级下完全够用，避免引入第三张表的复杂度。

### 5.3 数据导出

- 导出格式：JSON（v1）
- 导入格式：同 JSON
- 用于：备份、跨设备迁移、未来同步基础

---

## 6. 快捷键

| 快捷键 | 行为 |
|---|---|
| `Ctrl/Cmd + K` | 聚焦全局搜索 |
| `Ctrl/Cmd + N` | 极速新建（打开浮窗，自动粘贴剪贴板） |
| `Ctrl/Cmd + S` | 保存当前编辑 |
| `Ctrl/Cmd + F` | 当前视图内搜索 |
| `Ctrl/Cmd + Shift + C` | 复制当前命令 |
| `Ctrl/Cmd + D` | 切换收藏 |
| `Delete` | 删除选中命令（带确认） |
| `↑ / ↓` | 列表导航 |
| `Enter` | 复制并 toast |
| `Esc` | 清空搜索 / 关闭弹窗 |

---

## 7. 智能字段推断（标题 / 分类 / 标签）

> 极速新增的"大脑"。目标：用户只输入命令，软件把其余字段都猜得八九不离十。
> v1 纯规则（本地、零延迟、离线可用），v2 可选 LLM 增强。

### 7.1 触发时机

- **极速新增浮窗打开** → 命令变化后 200ms 防抖触发
- 完整编辑页 → `[重新推断]` 按钮触发
- 推断结果**直接填入字段作为默认值**（非阻塞、可改）
- 标签例外：以"候选 chips"形式呈现，点击采纳，避免噪音

UI 示意（标签区）：

```
标签：[cleanup ×] [docker ×]
候选：[+ container] [+ prune] [+ stopped]
```

### 7.2 v1 规则引擎（纯本地，零延迟）

#### 7.2.1 标题推断

按优先级匹配，第一条命中即采用：

| 优先级 | 规则 | 例 |
|---|---|---|
| 1 | 命令前若有 `#` 注释行，取第一条注释 | `# 备份目录` → title = `备份目录` |
| 2 | 多行命令，取第一行非空内容（截断 60 字符） | `tar -czf ...` 多行 → title = 第一行 |
| 3 | 单行命令，取前 60 字符 | `docker ps` → title = `docker ps` |

清洗：去掉行首 `$` `#` ` ` 等终端提示符。

#### 7.2.2 分类推断

维护工具名 → 分类映射表（内置，可在 `~/.config/Spellbook/category_map.json` 覆盖）：

| 命令首 token | 默认分类 |
|---|---|
| `git`, `gh` | Git |
| `docker`, `docker-compose`, `podman` | Docker |
| `kubectl`, `helm`, `k9s` | Kubernetes |
| `ls`, `cd`, `cp`, `mv`, `rm`, `find`, `grep`, `sed`, `awk`, `tar`, `chmod`, `chown` | Linux |
| `ssh`, `scp`, `rsync`, `ssh-keygen`, `ssh-copy-id` | SSH |
| `mysql`, `psql`, `mongo`, `redis-cli`, `sqlite3` | Database |
| `npm`, `yarn`, `pnpm`, `node` | NodeJS |
| `python`, `pip`, `poetry`, `uv` | Python |
| `cargo`, `rustc`, `rustup` | Rust |
| `go`, `gofmt` | Go |
| `ffmpeg`, `imagemagick`, `convert` | Media |
| `curl`, `wget`, `httpie` | HTTP |
| `systemctl`, `journalctl`, `service` | System |
| `brew`, `apt`, `yum`, `pacman`, `dnf` | Package |
| 未命中 | Other |

> 若用户已自定义过同名分类，直接复用其 ID；不存在则推断为"建议创建"（不自动创建）。

#### 7.2.3 标签推断

**Rule 1 — 工具名识别**

内置 CLI 白名单（约 100 个），命令首 token 命中即作为标签：

```
docker, git, kubectl, helm, npm, yarn, pnpm, cargo, rustc,
python, pip, node, go, mvn, gradle, ssh, scp, rsync, curl,
wget, ffmpeg, tar, zip, grep, sed, awk, find, lsof, netstat,
systemctl, journalctl, brew, apt, yum, pacman, nginx, redis-cli,
mysql, psql, mongo, ...
```

**Rule 2 — 动作动词识别**

| 命令片段 | 推荐标签 |
|---|---|
| `rm`, `prune`, `delete`, `remove`, `clean` | `cleanup` |
| `ls`, `list`, `ps`, `show`, `get` | `list` |
| `restart`, `reload` | `restart` |
| `log`, `logs`, `tail`, `journalctl` | `logs` |
| `backup`, `dump`, `export` | `backup` |
| `install`, `add` | `install` |
| `kill`, `stop`, `down` | `stop` |
| `start`, `up`, `run` | `start` |
| `find`, `grep`, `search` | `search` |

**Rule 3 — 子命令识别**

工具 + 子命令的常见组合：

```
docker compose    → compose
docker container  → container
git remote        → remote
git stash         → stash
kubectl get       → query
kubectl apply     → apply
ssh-keygen        → keygen
```

**Rule 4 — 端口 / 协议提取**

命令中出现常见端口或协议关键字时加标签：`:8080` → `port`、`https://` → `http`。

**Rule 5 — 去重与归一化**

- 全部小写
- 同义词归一（`rm` → `cleanup`，`ls` → `list`）
- 去掉与已有标签重复的候选
- 最多展示 5 个候选

#### 7.2.4 规则配置文件

用户可自定义规则，覆盖内置：

```jsonc
// ~/.config/Spellbook/inference_rules.json
{
  "category_map": {
    "terraform": "Infra",
    "ansible":   "Infra"
  },
  "tool_tags": ["terraform", "ansible", "vault"],
  "verb_tags": {
    "plan":  "preview",
    "apply": "deploy"
  }
}
```

### 7.3 v2 AI 后台增强（"粘贴即增强"，可选）

> 设计目标：用户只管粘贴和按 Enter，AI 在后台把字段悄悄升级到"像人写的"质量。
> 前提：开关默认关，首次启用一次性同意；用户编辑过的字段绝不覆盖。

#### 7.3.1 触发与生命周期

```
[T0]   用户粘贴命令 / 输入变化
        ↓ 防抖 200ms
[T1]   规则引擎同步运行（~5ms），字段立刻填好
        ↓
[T2]   若已启用 AI 增强，且通过门禁（见 §7.3.2）
        ↓
       异步发起 LLM 调用（顶部显示 🧠 加载指示）
        ↓
[T3]   LLM 返回（一般 1-3s，超时 8s）
        ↓
       按"覆盖策略"合并到字段（见 §7.3.3）
        ↓
       字段旁加 🧠 标记表明数据来源
```

#### 7.3.2 触发门禁（避免噪音和滥费）

满足**全部**条件才触发 LLM：

- 已启用"粘贴即增强"开关
- 命令长度 ≥ 10 字符（短命令不值得调用）
- 命令首 token 不在"已知安全黑名单"（如 `rm -rf /` 等危险命令直接跳过 LLM，防止幻觉描述误导）
- 同一 command 在 24h 内未被增强过（内存 + SQLite 缓存）
- 当前节流窗口未超限（默认 30 次/分钟，可配）
- 当前未处于"离线模式"

不满足时静默跳过，**不报错、不提示**（避免打扰）。

#### 7.3.3 覆盖策略 — 用户至上

为每个字段维护 `dirty` 标志，标记用户是否手动编辑过：

| 字段当前状态 | LLM 返回后行为 |
|---|---|
| `dirty = false`（用户没动） | **直接替换**，加 🧠 标记，短暂高亮闪烁 |
| `dirty = true`（用户改过） | **不覆盖**。把 LLM 结果作为"候选"挂在字段旁边，用户可一键采纳 |
| 字段为空 + 用户没动 | 直接填入 |

> 黄金法则：**用户的输入永远是最高优先级。** AI 只在用户没动的字段上"擅作主张"。

#### 7.3.4 保存时 LLM 未返回怎么办

用户按 `Enter`：

- **立即保存**当前字段值（不等待 LLM，符合"极速"承诺）
- 浮窗关闭，但 LLM 调用继续在后台
- LLM 返回后：
  - 仅有 tags / description 等"低风险"补充 → **静默 patch** 已保存记录，详情页打开时刷新
  - title / category 等"显著字段"被改写 → 右下角 toast：`🧠 AI 已优化"docker container prune -f"的标题和说明 [应用] [忽略]`，用户点击应用才生效
- 用户清退应用 → LLM 结果丢弃，下次打开该命令时再触发"按需增强"按钮

#### 7.3.5 调用策略

- 一律**异步**，非阻塞
- 同一字段集只发**一次合并请求**，不为每个字段单独调用
- 失败 / 超时 / 网络错 → 静默丢弃，绝不弹错（用户已经有了规则结果）
- 缓存命令哈希 → 结果：同命令再次粘贴，秒级返回

#### 7.3.6 Prompt 模板（合并请求）

```
你是命令笔记助手。基于下面这条 shell 命令，生成 JSON：
{
  "title":       "简短中文标题，不超过 20 字",
  "description": "Markdown 格式的说明，包含一段简述和逐参数解释",
  "tags":        ["英文小写", "标签", "..."]
}

命令：
{command}

已有规则推断结果（仅供参考）：
- title:    {rule_title}
- category: {rule_category}
- tags:     {rule_tags}

只输出严格的 JSON，无解释、无 markdown 代码块包裹。
```

校验：返回必须能 `serde_json::from_str` 解析；失败则丢弃。

#### 7.3.7 用户首次启用的同意流程

设置里勾选"粘贴即增强"→ 弹出一次性对话框：

```
┌─────────────────────────────────────────────────────────┐
│ 🧠 启用"粘贴即增强"                                      │
│                                                         │
│ 启用后，每次粘贴新命令时会自动把以下内容发送给           │
│ {provider}（{base_url}）：                              │
│                                                         │
│   • 命令文本（command）                                  │
│   • 规则推断出的初始 title / category / tags             │
│                                                         │
│ 不会发送：                                               │
│   • 你的其他命令                                         │
│   • 任何个人信息                                         │
│   • API key（从环境变量或系统 Keychain 读，不写配置文件）│
│                                                         │
│ 可随时在设置中关闭。                                     │
│                                                         │
│       [我了解，启用]              [取消]                 │
└─────────────────────────────────────────────────────────┘
```

确认后写入 `~/.config/Spellbook/llm.toml` 的 `consent.paste_enhance = "2026-05-18T10:23Z"`。

后续静默运行，仅在浮窗顶部保留 🧠 加载指示作为持续提示。

### 7.4 用户控制

设置项：

**规则层（v1）**

- [x] 启用智能字段推断（默认开）
- [x] 自动填充 title / category（默认开）
- [x] 标签以候选 chips 呈现，必须点击采纳（默认开）
- 自定义规则文件路径：`~/.config/Spellbook/inference_rules.json`

**AI 层（v2，需先在 §8 配置 LLM provider）**

- [ ] **粘贴即增强**（默认关；首次启用需明示同意）
- [ ] 允许 AI 改写 title（默认开，仅在 dirty=false 时生效）
- [ ] 允许 AI 生成 description（默认开）
- [ ] 允许 AI 补充 tags（默认开）
- 节流上限：`30 次/分钟`（可改）
- 缓存有效期：`24 小时`（同命令不重复调用）
- 危险命令黑名单：`~/.config/Spellbook/llm_blocklist.txt`

---

## 8. AI / LLM 功能（v2+，可选）

> **设计前提**：所有 AI 功能都是**可选**的，关掉后 v1 全部功能不变。
> 不做"AI 优先"产品，避免做着做着变成另一个 ChatGPT 客户端。

### 8.1 设计原则

- **本地优先可选**：支持 Ollama / LM Studio 本地模型
- **远程也支持**：OpenAI / Anthropic / DeepSeek / 任何 OpenAI 兼容 endpoint
- **必须可降级**：网络断了、API 没配，软件照常用
- **数据最小化**：每次只发当前操作所需字段；绝不批量、绝不发其他命令
- **默认手动；自动需明示同意**：默认所有 LLM 调用由用户点击触发。"粘贴即增强"是唯一的自动入口，开启时必须通过一次性同意对话框（见 [§7.3.7](#737-用户首次启用的同意流程)）
- **用户输入至上**：用户手动编辑过的字段，LLM 绝不覆盖（见 [§7.3.3](#733-覆盖策略--用户至上)）
- **失败静默**：所有 LLM 错误不弹窗、不打扰，丢弃即可
- **不留聊天历史**：不是 Chat 工具，所有调用都是无状态的

### 8.2 功能清单

| 功能 | 触发方式 | 输入 | 输出 | 优先级 |
|---|---|---|---|---|
| 智能字段推断增强 | 极速新增浮窗（异步、可关） | command | 更好的 title / category / tags | P1 |
| 智能生成 description | 编辑页 `✨` 按钮 | title + command | Markdown 说明（含逐参数解释） | P1 |
| 自然语言生成命令 | `Ctrl/Cmd + I` | 自然语言描述 | 命令 + 说明 | P1 |
| 命令解释 | 详情页按钮 | command | 逐参数解释（markdown） | P1 |
| 自然语言搜索 | 搜索框 `?` 前缀 | "如何查看占用 8080 端口" | 匹配的命令列表 | P2 |
| 命令翻译 | 详情页按钮 | command + 目标工具 | 等价命令（如 docker → podman） | P3 |
| 错误诊断 | 独立面板 | 报错文本 | 可能原因 + 建议命令 | P3 |

### 8.3 重点功能详述

#### 8.3.1 自然语言生成命令（P1）

入口：`Ctrl/Cmd + I` 唤起一个浮窗：

```
┌────────────────────────────────────────────┐
│ 我想要... 删除 30 天前修改过的 log 文件    │
├────────────────────────────────────────────┤
│ find /var/log -name "*.log" -mtime +30 \   │
│   -type f -delete                          │
│                                            │
│ [插入到当前编辑] [复制] [保存为新命令]     │
└────────────────────────────────────────────┘
```

#### 8.3.2 命令解释（P1）

详情页一个 `[解释]` 按钮，点击后在说明区下方追加一段 LLM 生成的逐参数解释：

```
docker container prune -f
├─ container : 仅作用于容器（不影响镜像 / 网络）
├─ prune     : 清理子命令
└─ -f        : 跳过确认提示
```

#### 8.3.3 自然语言搜索（P2）

搜索框输入以 `?` 开头时，整句作为自然语言查询：

```
? 怎么看哪个进程占用了 8080 端口
```

LLM 把它翻译成关键词（如 `port lsof netstat 8080`），再走本地模糊搜索流程；如果库里没有，提示"是否生成新命令"。

### 8.4 模型接入

配置文件 `~/.config/Spellbook/llm.toml`：

```toml
[provider]
type = "openai"          # openai | anthropic | ollama | custom
base_url = "https://api.openai.com/v1"
model = "gpt-4o-mini"
api_key_env = "OPENAI_API_KEY"   # 环境变量名；密钥本身不存配置文件

[limits]
max_tokens = 512
timeout_secs = 10

[features]
auto_tagging = true
nl_generate = true
explain = true
nl_search = false
```

> API key **绝不写入 `llm.toml`**。两条读取路径：
>
> 1. 环境变量（`api_key_env` 指定名字，如 `OPENAI_API_KEY`）—— 优先
> 2. 系统 Keychain（用户在设置里点"保存到 Keychain"主动写入）—— 兜底
>
> 后者通过 `keyring` crate 接入 macOS Keychain / Windows Credential Manager /
> Linux Secret Service，密钥从不在明文磁盘文件里出现，避免泄漏到备份/同步盘。

### 8.5 隐私

- 设置页一个明显的"AI 数据流向"说明，列出每个功能发送哪些字段、发往哪个 endpoint
- **每个 AI 功能首次使用时弹一次性确认**，尤其是"粘贴即增强"等自动触发的功能（见 [§7.3.7](#737-用户首次启用的同意流程)）
- 任意时刻可在浮窗 / 详情页点 `[查看本次将发送的内容]` 看到明文 payload
- **离线模式**（设置开关）= 强制禁用所有远程调用，本地 Ollama 也包含在内（彻底"飞行模式"）
- 同意状态记录在 `~/.config/Spellbook/llm.toml`，删除该文件即重置全部同意
- 加载指示常驻：只要有 LLM 调用在飞，顶部一定有 🧠 标记，让用户对"现在有数据在外发"始终有感

---

## 9. 非目标（不做）

### v1 完全不做

- 云同步 / 多端
- 多人协作 / 分享
- 插件系统
- 复杂主题系统（v1 仅 light/dark）
- 数据库切换（仅 SQLite）
- 笔记之间的关系图谱
- WYSIWYG 富文本编辑器
- 嵌套分类
- 命令执行（**绝不**在软件内运行 shell 命令，安全风险太大）

### v2 也暂不做

- 浏览器扩展
- 终端集成 / shell hook
- 团队空间
- 命令的版本历史 / diff

### 总原则

> "做着做着变成 Obsidian" = 立刻停手。
>
> 这个工具只解决一件事：**记命令、找命令、复制命令**。

---

## 10. 开发路线图

### v1（MVP，约 4 周）

| 周 | 目标 |
|---|---|
| W1 | SQLite + CRUD + UI 骨架 + 分类树 + 详情页 |
| W2 | 模糊搜索 + 快捷键 + 一键复制 + Markdown 预览 |
| W3 | 收藏 + 最近使用 + **规则版自动打标签** + JSON 导入导出 |
| W4 | 性能优化 + UI polish + 三端打包发布 |

### v2（约 3-4 周）

- LLM 集成框架（provider 抽象 + 配置 + 隐私确认）
- 智能打标签增强
- 自然语言生成命令
- 命令解释
- 设置页（含 AI 开关）

### v3+（待定）

- 自然语言搜索
- 智能补全描述
- 命令翻译
- 错误诊断

### 已落地（不在初版路线图但已在 0.1.x 中实装）

- 命令变量 / 参数模板（`ssh ${USER}@${HOST}` → 复制前弹填空对话框）
- 回收站 / 7 天软删除（`command_note.deleted_at` 列 + Trash 视图）
- API key 写入系统 Keychain（v0.1.6）
- 应用内一键自更新（仅 macOS `.app`，v0.1.8）

---

## 11. 风险与开放问题

| 风险 / 问题 | 备注 |
|---|---|
| egui 在三端的字体渲染一致性 | 提前在 W1 验证 macOS / Windows / Linux 中文字体 |
| 1w+ 命令下搜索性能 | W2 用 mock 数据压测；如内存策略不够，再上 FTS5 |
| LLM 输出不稳定（标签格式不一致） | 加 schema 校验 + 重试一次，失败则丢弃 |
| API key 管理 | 环境变量优先 + 系统 Keychain 兜底；密钥从不写 `llm.toml` |
| "本地优先" vs "AI 联网" 的产品张力 | 默认全部 AI 功能关闭，用户主动开启 |
| 命令执行的安全风险 | 永远不在软件内执行命令，仅复制到剪贴板 |
| 自动打标签的"噪音" | 用候选 chips、不自动写入；用户拒绝行为不上传 |

---

**附：变更记录**

| 日期 | 版本 | 变更 |
|---|---|---|
| 2026-05-18 | v1.0 | 初版整理；新增第 7 节自动打标签、第 8 节 AI/LLM 可选功能 |
| 2026-05-18 | v1.1 | 新增"极速新增（粘贴即存）"为命令新建主流程；第 7 节扩展为标题/分类/标签统一推断 |
| 2026-05-18 | v1.2 | 第 7.3 节重构为"AI 后台增强（粘贴即增强）"，覆盖触发门禁、覆盖策略、保存时机、首次同意流程；§8 原则更新为"默认手动，自动需明示同意" |
| 2026-05-20 | v1.3 | §8.4 / §11 / §7.3.7：API key 管理新增系统 Keychain 兜底路径（v0.1.6）；路线图新增"已落地"小节，记录参数模板、回收站、Keychain、应用内自更新（v0.1.8）等已实装但未在初版规划中的能力 |
