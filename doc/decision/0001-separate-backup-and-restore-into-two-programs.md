---
title: "Separate Backup and Restore into Two Independent Programs"
description: |
  备份器在完整系统上运行、可用任意编排工具；
  恢复器在裸系统上运行、必须自包含，除 dnf 外不依赖其他网络；
  二者通过 U 盘上的固定契约交互。
type: adr
category: architecture
tags:
  - architecture
  - backup
  - restore
  - offline
  - usb
status: accepted
created: 2026-11-01
updated: 2026-11-01
author: taoda
project: fedora-44-setup
technologies:
  - usb
audience:
  - operators
---

# ADR-0001: Separate Backup and Restore into Two Independent Programs

## Status

Accepted

## Context

### Background and Problem Statement

本项目的目标只有一个：把一台用官方介质装好的 Fedora 44 Xfce 版，恢复成个人日常使用的状态。

这个目标天然包含两个动作，而它们发生的**时间点、所在机器、可用条件完全不同**：

| | 备份 | 恢复 |
|---|---|---|
| 何时 | 重装之前 | 重装之后 |
| 机器状态 | 完整可用，职责齐全 | 刚装完，几乎什么都没有 |
| 网络 | 不相关 | dnf 大概率可用，其余默认不能用 |
| 可用工具 | 全部已装 | 只有基础系统 |
| 数据 | 完整存在 | 已经不存在 |

若用同一套工具（如 Ansible）实现，其使用假设的差异在实践中反复制造矛盾：

1. 恢复器写在 `restore.yml` 里，可运行它的机器上恰恰没有 Ansible；
2. 为了让恢复“跑起来”，流程被迫先 `git clone` 仓库、再 `dnf install ansible-core`——恢复的第一步就把控制权交回了网络；
3. 恢复路径上的每一个依赖（仓库、包管理器、网络）都会成为单点故障。

根因不是工具选错了，而是**把两个运行约束完全不同的程序当成了一个**。

### Current Limitations

当前的做法本质是手工备份数据到仓库，而仓库维护恢复器和用户数据。

1. 字体等大体积文件为了“随仓库分发”而被塞进 Git LFS。而 Git 和 LFS 的取回同样依赖远端可达。
   一旦网络受限（例如 GitHub 不可达），整套恢复链路失效。
2. 有些用户数据因数据安全的原因，不适宜暴露在公开仓库里，比如 ssh 私钥。

## Decision Drivers

### Primary Decision Drivers

1. **恢复器必须在最小环境中可运行**：裸系统 + 在线 dnf 仓库 + U 盘，除此之外不假设任何东西。
2. **恢复路径上的外部依赖必须可枚举且最少**：每一项依赖都要能被明确列出并单独评估。
3. **备份与恢复的职责边界清晰**：各自只对自己那台机器负责。
4. **不把“仓库”当作数据**：仓库不是需要备份的个人资产。

### Secondary Decision Drivers

1. **两端可使用不同技术**：允许备份端用高级编排工具，恢复端用最原始的工具。
2. **契约稳定**：U 盘上的目录与文件格式一旦确定，两端可独立演进。
3. **可审阅**：备份范围应是文本清单，可 diff、可 review。

## Considered Options

### Option 1: 单一程序，备份与恢复同源（同一套 Ansible 代码）

**Description**: `roles/backup` 与 `roles/restore` 共用同一技术栈，由脚本双向调用。

**Technical Characteristics**:
- 复用 `become`、幂等、错误处理与变量体系；
- 恢复前需要 `ansible-core`、`python3` 与仓库本体。

**Advantages**:
- 单一技术栈，风格一致；
- 备份端读取 YAML 清单非常自然。

**Disadvantages**:
- 恢复环境不具备其依赖，“自包含”无法成立；
- 恢复的第一步被迫是 `git clone` + 装包，把网络重新引入关键路径；
- 备份范围与恢复逻辑共享代码，改动互相牵连。

**Risk Assessment**:
- **Technical Risk**: High。恢复器的运行前提与设计目标直接冲突。
- **Schedule Risk**: Low。
- **Ecosystem Risk**: Low。

**Disqualifying Factor**: 要求“在没有 Ansible 的机器上运行 Ansible”，是逻辑上的自相矛盾。

### Option 2: 单体打包

**Description**: 用 AppImage 把恢复器、字体、配置全部打进单个可执行文件。

**Technical Characteristics**:
- 单文件分发，运行时挂载 SquashFS；
- 依赖 `libfuse.so.2`；
- 包内资源默认只对自身进程可见。

**Advantages**:
- 分发形态干净；
- 不依赖宿主 Python。

**Disadvantages**:
- 只在谈恢复一端，而没有谈备份一端。技术方案不完整。
- Fedora 默认只有 fuse3，需要先安装 `fuse-libs` 才能运行 AppImage。
  也就是说，在目标环境下，实际做不到单文件分发。

**Risk Assessment**:
- **Technical Risk**: High。
- **Schedule Risk**: High。
- **Ecosystem Risk**: Medium。

**Disqualifying Factor**: 解决了“载体形态”，但没有解决“恢复环境缺依赖”这一真正约束。

### Option 3: U 盘保存完整仓库，恢复时从 U 盘执行 `ansible-playbook`

**Description**: U 盘保存完整仓库（含 `.git`、字体与 LFS 对象），恢复时从 U 盘执行 `ansible-playbook`。

**Technical Characteristics**:
- 备份对象单一，天然自包含；
- 恢复仍需要预先安装 `ansible-core`。

**Advantages**:
- 结构简单；
- 备份与日常 apply 使用同一套代码。

**Disadvantages**:
- 恢复的第一步仍是装依赖，与“最小环境”目标不符；
- 备份 `.git` 与 LFS 对象意义不大——仓库可重建；
- 无法表达“只备份哪些用户数据”。

**Risk Assessment**:
- **Technical Risk**: Medium。
- **Schedule Risk**: Low。
- **Ecosystem Risk**: Low。

**Disqualifying Factor**: 仍然把“恢复器能否运行”押在恢复环境之外的东西上。

### Option 4: 备份器与恢复器分离，通过 U 盘契约交互（采纳）

**Description**: 两个独立程序，各自针对自己的运行环境设计；唯一共享物是 U 盘上的一组固定文件。

**Technical Characteristics**:
- **备份器**运行在完整系统上，可用任何工具，产出“数据 + 恢复器 + 清单”；
- **恢复器**根据 U 盘内的数据和清单来恢复系统；
- **仓库**只保存备份器与恢复器源码、备份策略和恢复策略；仓库不进 U 盘，也不保存用户数据。

**Advantages**:
- 恢复器可在裸系统上直接运行，不依赖 `git clone` 或已装 Ansible；
- 备份端可自由选择不同的技术栈而不影响恢复端；
- 恢复路径上的依赖可被完整枚举；
- 备份范围完全由清单决定，改清单可以不改代码。

**Disadvantages**:
- 两套实现，存在语义漂移（即两端的约定不一致）的风险；
- U 盘需要同时承载用户数据与恢复器；
- 清单文件是生成物，不能手工维护。

**Risk Assessment**:
- **Technical Risk**: Low。
- **Schedule Risk**: Low。
- **Ecosystem Risk**: Low。

## Decision

采用 **Option 4**：备份器与恢复器是两个程序。

**仓库的定位**：只提供备份器和恢复器的实现。如果备份选择通用程序，仓库存放“备份策略”。
**被备份的用户数据不存放在仓库里**。

## Consequences

### Positive

1. **恢复器可在最小系统上运行**：插上 U 盘即可开始，`git clone` 与 LFS 均不在关键路径上。
2. **依赖可枚举**：恢复需要什么，一眼能列出并逐项评估。
3. **接口稳定即可独立演进**：只要 U 盘契约不变，任一端替换实现都不影响另一端。
4. **策略与数据分离**：备份器源码或备份策略是纯文本，可 diff、可 review。

### Negative

1. **两套实现**：备份与恢复需要分别维护。
2. **契约变更需同步两端**：清单格式为两端共享，改动属于破坏性修改。
3. **依赖 U 盘意味着单点风险**：方案强依赖 U 盘的两个特征——可离线搬运数据、不接入网络。
   后者避免了用户数据在线上泄漏，但两者共同意味着：U 盘一旦遗失，数据与恢复手段同时消失。
   此外 U 盘本身不提供加密，遗失即泄露。

### Neutral

1. **独立演进的边界止于接口**：两端共享同一套清单格式，格式变更对双方都是破坏性的。

## Related Decisions

- 暂无其他 ADR。本决策是所有后续 ADR 的前提；任何与之冲突的决策都应先修订本文件。

## Links

- [ADR](https://adr.github.io/) - 架构决策记录方法论

## More Information

- **Date:** 2026-11-01
- **Source:** 关于“是否改用单体打包”与“U 盘能否独立恢复”的架构讨论

## Audit

### 2026-11-01

**Status:** Pending

**Findings:**

| Finding | Files | Lines | Assessment |
|---------|-------|-------|------------|
| Awaiting implementation | - | - | pending |

**Summary:** 架构已确定，备份器与恢复器尚未在本仓库中实现。

**Action Required:** 按本 ADR 落地备份器与恢复器，并在真实 U 盘上完成一次备份 → 重装 → 恢复的完整验证后更新审计结论。
