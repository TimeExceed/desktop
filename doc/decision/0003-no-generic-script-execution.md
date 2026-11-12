---
title: "No Generic Script Execution in Restore Actions"
description: "恢复动作只提供专用类型，不提供 python 脚本或任意命令的执行能力；每个系统操作都由 Rust 实现，以统一完成判定、错误链与日志。"
type: adr
category: architecture
tags:
  - policy
  - actions
  - restore
  - python
status: accepted
created: 2026-11-11
updated: 2026-11-11
author: taoda
project: fedora-44-setup
technologies:
  - rust
  - toml
  - json-schema
audience:
  - operators
---

# ADR-0003: No Generic Script Execution in Restore Actions

## Status

Accepted

## Context

### Background and Problem Statement

[ADR-0002](0002-single-toml-file-as-restore-policy-and-backup-manifest.md) 把策略文件确立为纯数据：
每个步骤是一个动作，动作类型由 `doc/policy.schema.yaml` 约束。
但最初的九种动作并不足以表达所有恢复操作，于是有了两种“逃生舱”：

1. **`python` 动作**：策略里指定 `exec` 与 `verify` 两个脚本路径，恢复器先跑 `verify`（返回 0 视为已完成），否则跑 `exec`；
2. **`run-command` 动作**：策略里直接给出命令行，恢复器原样执行。

两者都出现过，`python` 至今仍被 `data-disk` 使用。
实际使用中暴露出同一类问题：**它们把“这一步是否完成”的判断推给了策略作者，而恢复器无法自证任何事**。

具体的三个场景：

- `nonfree-audio`：脚本里是 `dnf install @multimedia` 加一次 `dnf swap ffmpeg-free ffmpeg`；
- `fonts`：脚本里只有一行 `fc-cache -fv`；
- `data-disk`：三个脚本分别负责选盘写 fstab、只读校验、重启后确认。

其中前两个根本不需要脚本，只是当时的动作类型不够用；第三个的复杂度也完全可以用一个专用动作承载。

### Current Limitations

- `python` 的 `verify` 被要求“只读、快速、可反复执行”。
  这个约束在 step 粒度上过大：
  真正需要判断的往往只是一个原子查询（`rpm -q`、`mountpoint -q`、`fc-list`），却被要求封装成一个独立进程；
- `run-command` 连 `verify` 都没有，是比 `python` 更弱的形态：
  它把“幂等”完全交给策略作者，而策略作者写错时没有任何提示；
- 两者的失败信息都只有退出码。
  `python` 的 traceback 与 `run-command` 的 stderr 混在 tee 转发里，恢复器无法把“哪一步失败、为什么失败”结构化地带出来；
- 策略文件被迫与一组脚本文件共同维护，破坏了 ADR-0002 确立的“策略即数据”。
  脚本的日志格式、错误处理风格、路径解析方式都与恢复器不一致。

## Decision Drivers

### Primary Decision Drivers

1. **可验证性**：每个步骤必须能自证完成，判定逻辑属于动作实现，而不是策略文件。
2. **错误链一致**：失败时应当给出一句能定位问题的话，而不是“脚本失败（exit 1）”。
3. **策略即数据**：策略文件只描述“做什么”，不引入第二门语言或外部脚本。
4. **日志统一**：所有步骤的输出前缀由恢复器的 `log_target` 控制，不出现两套格式。

### Secondary Decision Drivers

1. **类型安全**：参数在编译期校验，字段拼写错误在 `cargo build` 时暴露，而不是等到恢复时。
2. **可测试性**：动作可以像普通 Rust 函数一样被单元测试覆盖，不需要启动解释器。

## Considered Options

### Option 1: 保留 `python` 动作（exec + verify 脚本）

**Description**: 策略里引用两个脚本，恢复器先跑 `verify` 判断是否已完成，未完成再跑 `exec`。

**Technical Characteristics**:
- 任意复杂逻辑都能表达；
- 需要为每个任务维护一对脚本文件；
- 备份器需要把这两个脚本一并收进 U 盘。

**Advantages**:
- 表达力最强，不需要改动恢复器本身。

**Disadvantages**:
- “是否完成”的判断标准由策略作者定义，`verify` 的只读约束在 step 粒度上不自然；
- 脚本的日志、错误、路径解析风格与恢复器不一致；
- 失败信息只有退出码与混在 tee 输出里的 stderr；
- 策略文件与脚本文件必须同步演进，破坏了“策略即数据”。

**Risk Assessment**:
- **Technical Risk**: High。脚本正确性不受 schema 约束，也无法在编译期检查。
- **Schedule Risk**: Low。
- **Ecosystem Risk**: Medium。需要额外保证 U 盘上脚本的完整性。

**Disqualifying Factor**: 把完成判定与错误处理这两件最需要统一的事，交给了最难统一的一侧。

### Option 2: 提供 `run-command` 通用命令动作

**Description**: 策略里直接写命令与参数，恢复器原样执行。

**Technical Characteristics**:
- 一行 TOML 就能做任何事；
- 没有任何完成判定；
- 恢复器无法区分“命令成功”与“操作已达到目标”。

**Advantages**:
- 实现成本最低。

**Disadvantages**:
- 完全没有幂等保障，重复执行是否安全完全取决于命令本身；
- 错误信息只有退出码；
- 相当于把 `python` 的问题换了个包装，而且比它更弱——连 `verify` 都没有；
- 为“执行任意命令”开了口子后，专用动作就没有存在的必要性，整个设计会退化成“一个 Shell 脚本框架”。

**Risk Assessment**:
- **Technical Risk**: High。
- **Schedule Risk**: Low。
- **Ecosystem Risk**: Low。

**Disqualifying Factor**: 与 ADR-0002 的“策略即数据”目标直接冲突，且无法自证完成。

### Option 3: 专用动作（采纳）

**Description**: 每种系统操作提供一个独立的动作类型，由 Rust 实现；参数经 schema 校验，完成判定在动作内部完成。

**Technical Characteristics**:
- 动作类型数量随需求增长；
- 新场景需要改 Rust 代码并重新编译；
- 每个动作都可以独立测试。

**Advantages**:
- 完成判定内建（`dnf_installed`、`patch_applied`、`boot_id` 等）；
- 错误链统一（`anyhow::Context` 逐层带出）；
- 日志前缀统一（`log_target`）；
- 策略文件是纯数据，不依赖任何外部脚本。

**Disadvantages**:
- 表达力受限于已实现的动作；
- 新增能力必须走代码变更，不能只改策略；
- 恢复器二进制随动作增多而变大（当前约 4.4MB）。

**Risk Assessment**:
- **Technical Risk**: Low。逻辑集中在已知的、可测试的代码路径上。
- **Schedule Risk**: Medium。新场景需要开发时间。
- **Ecosystem Risk**: Low。

## Decision

采用 **Option 3**：不提供 `python`、`run-command` 或任何形式的“执行任意代码/命令”的动作；每个系统操作对应一个专用动作类型。

实现层面的一些约定：

- **动作类型由 `doc/policy.schema.yaml` 穷举**，`oneOf` 中的每一项都是一个具体动作，没有兜底分支；
- **完成判定写在动作内部**：
  如 `dnf-installed` 用 `rpm -q` / `dnf group info`，`patch-file` 用反向 dry-run，`reboot` 用 `boot_id` 比较，`manual-check` 用状态文件；
- **错误统一走 `anyhow`**：动作返回 `Result`，恢复器在边界处打印完整错误链；
- **输出统一走 `StepContext::run`**：子进程的 stdout/stderr 直接继承终端，不重定向，
  `sudo` 的密码提示因此可见。

已经落地的替换：

| 原实现 | 替代动作 |
|---|---|
| `nonfree-audio/install.py` | `dnf-install` 扩展 `exclude` / `setopt` / `allowerasing` |
| `fonts/install.py`（`fc-cache -fv`） | 新增 `update-fc-cache` |

仍待迁移：

| 原实现 | 计划 |
|---|---|
| `data-disk/install.py` + `verify.py` + `post-check.py` | 新增 `mount` 动作 |

`python` 动作在 `mount` 落地前保留，但其存在本身已被本 ADR 判定为过渡状态。

## Consequences

### Positive

1. **每步可自证完成**：判定逻辑与动作实现绑定，策略作者无法写错，也不需要写。
2. **错误可定位**：失败时输出的是“打 patch 失败：/etc/lightdm/lightdm.conf”这类句子，而不是退出码。
3. **日志格式统一**：所有步骤共享 `{task}/{step-index}-{step-id}` 前缀，便于按步骤检索。
4. **策略文件保持纯数据**：U 盘上不再需要携带脚本，备份产物的完整性更容易保证。

### Negative

1. **新场景必须改代码**：无法通过只改策略文件引入新操作，开发周期比写脚本长。
2. **动作类型持续增长**：每一个“跑一下就好”的需求都会变成一个动作（`update-fc-cache` 就是例子）。
3. **二进制体积**：动作越多，恢复器越大；当前 4.4MB，仍远小于携带解释器的方案。

### Neutral

1. **`python` 动作暂时保留**：`data-disk` 尚未迁移，本 ADR 记录的是方向而非即时完成的状态。
2. **`run-command` 曾短暂实现并已移除**：它验证了“通用执行能力”这条路走不通。

## Decision Outcome

恢复器只接受 schema 中显式列出的动作类型，不提供执行任意脚本或命令的能力。
新增系统操作时，先扩展 schema 与 `Action` 枚举，再实现对应的 `backup` / `restore` 分支。

Mitigations:
- **表达力不足**：以“新增动作”而非“新增逃生舱”的方式扩展；`update-fc-cache` 与 `mount` 都是这一原则的产物；
- **开发成本**：动作之间共享 `StepContext`、`ctx.run`、`same_content` 等基础设施，单个动作的实现通常不超过五十行；
- **过渡期**：`python` 在 `data-disk` 迁移完成前保留，但不得用于新任务。

## Related Decisions

- [ADR-0001](0001-separate-backup-and-restore-into-two-programs.md) - 恢复器必须在裸系统上自包含运行，这是“不携带解释器与脚本”的前提。
- [ADR-0002](0002-single-toml-file-as-restore-policy-and-backup-manifest.md) - 本 ADR 是其“策略即数据”原则在动作粒度上的延伸。

## Links

- [policy.schema.yaml](../policy.schema.yaml) - 动作类型的定义
- [src/policy.rs](../../src/policy.rs) - `Action` 枚举与策略校验
- [src/backup.rs](../../src/backup.rs) - 动作的备份实现
- [src/restore.rs](../../src/restore.rs) - 动作的恢复实现、`StepContext` 与恢复器工具函数
- [src/run_as.rs](../../src/run_as.rs) - `RunAs`，以及 `same_content`、`restore_file` 等按身份执行的工具

## More Information

- **Date:** 2026-11-11
- **Source:** 关于 `python` 动作是否应该存在的讨论，以及 `run-command` 的短暂实现与撤销

## Audit

### 2026-11-11

**Status:** Done

**Findings:**

| Finding | Files | Lines | Assessment |
|---------|-------|-------|------------|
| `python` 动作已从 schema、枚举、`id()`、`backup` / `restore` 中移除 | `doc/policy.schema.yaml`, `src/policy.rs` | - | done |
| `nonfree-audio` 已迁移到 `dnf-install` 扩展 | `policy.toml` | 133-146 | done |
| `fonts` 已迁移到 `update-fc-cache` | `policy.toml` | 285-290 | done |
| `data-disk` 已迁移到 `mount` | `policy.toml`, `src/restore.rs` | - | done |
| `run-command` 已移除 | `doc/policy.schema.yaml`, `src/policy.rs` | - | done |
| 脚本夹具与相关测试已清理 | `tests/fixtures/`, `tests/backup.rs` | - | done |

**Summary:** 在原有动作之外新增 `update-fc-cache` 与 `mount` 两个专用动作，`nonfree-audio`、`fonts`、`data-disk` 三个任务全部改为声明式表达；`python` 与 `run-command` 均已从代码库移除，策略文件不再引用任何外部脚本。

**Action Required:** 无。后续若出现新的系统操作需求，按本 ADR 的 Decision 新增专用动作，不再引入通用执行能力。
