---
title: "Use a Single TOML File as Both Restore Policy and Backup Manifest"
description: "把“恢复清单”与“备份策略”合并为一个 TOML 文件，按恢复器视角描述，schema 置于仓库，备份器通过转换视角复用同一份文件。"
type: adr
category: architecture
tags:
  - policy
  - schema
  - toml
  - backup
  - restore
status: accepted
created: 2026-11-03
updated: 2026-11-03
author: taoda
project: fedora-44-setup
technologies:
  - toml
  - json-schema
audience:
  - operators
---

# ADR-0002: Use a Single TOML File as Both Restore Policy and Backup Manifest

## Status

Accepted

## Context

### Background and Problem Statement

[ADR-0001](0001-separate-backup-and-restore-into-two-programs.md) 确立了备份器与恢复器分离，并约定二者“通过 U 盘上的一组固定文件”交互。
它在两处提到了这份产物的两个名字：

- 第 159 行：备份器产出“数据 + 恢复器 + **清单**”；
- 第 161 行：仓库保存“备份器与恢复器源码、**备份策略**和**恢复策略**”。

这两个名字指向的是同一组事实：哪些配置需要恢复、从哪里来、如何验证。
如果按字面分成两份产物维护，会出现两个文件描述同一组事实的局面——它们会漂移，而漂移的后果是恢复时漏做或做错，且不容易被发现。

因此需要就三件事做出决定：

1. 这两者是否合并为一份文件；
2. 这份文件站在谁的视角描述（恢复器还是备份器）；
3. 这份文件放在哪里、以什么格式维护。

### Current Limitations

ADR-0001 第 172 行把该产物描述为“生成物，不能手工维护”。
这个表述当时针对的是“由备份器在运行时生成”的产物形态。
一旦决定把清单本身作为仓库中的源文件长期维护，这句话就不再准确，需要在本 ADR 中澄清适用范围。

## Decision Drivers

### Primary Decision Drivers

1. **单一事实来源**：同一组事实只应存在一份，避免两个文件互相漂移。
2. **恢复器视角优先**：恢复发生在裸系统上，是约束更多、更难实现的一侧；描述以其视角为准，可以减少“难的一侧”在运行时的转换负担。
3. **可审阅**：文件必须是纯文本，能 diff、能 review、能进入版本历史。
4. **备份器可复用**：备份器不应另起一份清单，而应从同一份文件推导所需信息。

### Secondary Decision Drivers

1. **格式适合手写**：清单需要人工维护，格式应避免缩进敏感与隐式类型带来的陷阱。
2. **解析零依赖**：恢复器面向最小环境，解析格式不应引入额外第三方库。

## Considered Options

### Option 1: 两份文件，恢复清单与备份策略分别维护

**Description**: 各写一份文件，分别面向恢复器与备份器。

**Technical Characteristics**:
- 两份文件结构可各自优化；
- 需要人工保证两者描述的是同一组事实。

**Advantages**:
- 每份文件的语义都很直接，读取方无需转换视角。

**Disadvantages**:
- **必然漂移**：新增一项配置时容易只改一份；
- 漂移的后果是静默的——恢复时漏做某一步，或备份时漏收某个文件；
- 两份文件都需要维护，成本翻倍。

**Risk Assessment**:
- **Technical Risk**: High。双写导致的不一致难以在运行期发现。
- **Schedule Risk**: Low。
- **Ecosystem Risk**: Low。

**Disqualifying Factor**: 违背“单一事实来源”，而这是本决策的核心诉求。

### Option 2: 一份文件，恢复器视角（采纳）

**Description**: 一份文件同时充当恢复清单与备份策略，按“从 U 盘复制到系统”的方向描述；备份器读取后自行逆转方向。

**Technical Characteristics**:
- 每个步骤描述“恢复动作”及其验证方式；
- 备份器只需筛选出涉及数据的步骤，反向推导出备份源；
- 文件位于仓库，随版本历史演进。

**Advantages**:
- 单一事实来源；
- 难的一侧（恢复器）不需要转换，直接消费；
- 备份器的“转换视角”是一次性的、可测试的映射。

**Disadvantages**:
- 备份器需要反向理解，认知负担落在备份器一侧；
- 并非所有恢复动作都有对应的备份数据（如安装软件包、重启、人工确认），备份器必须知道忽略哪些。

**Risk Assessment**:
- **Technical Risk**: Medium。视角转换的正确性依赖备份器的实现，但可以单独测试。
- **Schedule Risk**: Low。
- **Ecosystem Risk**: Low。

### Option 3: 一份文件，备份器视角

**Description**: 按“从系统备份到 U 盘”的方向描述，恢复器读取后逆转方向。

**Technical Characteristics**:
- 与 Option 2 对称，只是方向相反；
- 恢复器承担视角转换。

**Advantages**:
- 备份器实现更直接。

**Disadvantages**:
- 把认知负担推给恢复器，而恢复器恰恰运行在裸系统上、调试条件最差；
- 恢复器需要额外逻辑才能把“备份源”还原成“恢复目标”，出错的代价更高。

**Risk Assessment**:
- **Technical Risk**: Medium。
- **Schedule Risk**: Low。
- **Ecosystem Risk**: Low。

**Disqualifying Factor**: 把复杂度放在了最难验证、最难调试的一侧。

### Option 4: 一份文件，中立视角

**Description**: 用“源与目标”成对描述，不偏向任何一方。

**Technical Characteristics**:
- 每个条目同时给出两个方向；
- 读取方各自选取需要的方向。

**Advantages**:
- 表面上最公平。

**Disadvantages**:
- 描述冗长，每个条目都要写两遍信息；
- 对于“没有备份数据的恢复动作”（装包、重启、人工确认），中立视角无法自然表达，仍需特例；
- 手工维护成本最高。

**Risk Assessment**:
- **Technical Risk**: Medium。
- **Schedule Risk**: Low。
- **Ecosystem Risk**: Low。

**Disqualifying Factor**: 为了形式对称而增加了每次维护的成本，收益不明显。

## Decision

采用 **Option 2**：一份文件，恢复器视角。

The implementation will use:

- **一份 TOML 文件**同时充当恢复清单与备份策略，术语上统称“策略文件”；
- **恢复器视角**：文件中的每个步骤描述“如何从 U 盘恢复系统”，而不是“如何备份系统”；
- **schema 置于仓库**：`doc/policy.schema.yaml`，采用 JSON Schema draft 2020-12，供编辑器与校验工具使用；
- **策略文件维护在仓库**：它是源文件，进入版本历史与 review；U 盘上的同名文件由备份器写入，是它的副本。

选择 TOML 的理由：

1. **不依赖缩进**：YAML 对缩进与块标量指示符敏感，本次设计过程中已实际遇到一次因 `|>` 写法导致的整份文件无法解析；
2. **类型明确**：字符串、整数、布尔、数组、表都有显式语法，不像 YAML 存在隐式类型推断；
3. **解析零依赖**：Python 3.11 起标准库内置 `tomllib`，恢复器可在不引入第三方库的前提下读取；
4. **适合手写**：清单需要长期人工维护，TOML 的可读性优于同义 JSON。

关于 ADR-0001 第 172 行“清单文件是生成物，不能手工维护”的澄清：该表述适用于 **U 盘上的副本**。
仓库中的策略文件是源文件，由人手工维护；副本由备份器写入，不需要也不允许手改。
二者不是同一份东西。

## Consequences

### Positive

1. **单一事实来源**：新增或调整配置只改一处，两端自动同步。
2. **难的一侧不承担转换**：恢复器直接消费文件，逻辑最简。
3. **清单进入版本历史**：哪些配置被纳入恢复范围、何时变更，都可追溯。
4. **格式选型降低维护成本**：TOML 的显式语法与标准库解析，减少了“格式本身出错”这类问题。

### Negative

1. **备份器承担视角转换**：从“复制到甲”推导“从甲备份”，需要额外的映射与测试。
2. **需要区分可备份与不可备份的动作**：安装软件包、重启、人工确认等动作没有对应数据，备份器必须显式忽略它们。
3. **schema 与实现需同步演进**：因不引入版本号，策略文件、schema、恢复器三者必须在同一仓库内一起修改。

### Neutral

1. **术语统一**：“恢复清单”与“备份策略”在文档与代码中应统一称为“策略文件”，避免读者误以为存在两份产物。
2. **U 盘上存在同名文件**：它是副本，内容与仓库中的源文件一致，但生命周期不同。

## Decision Outcome

恢复清单与备份策略合并为仓库中的一份 TOML 文件，按恢复器视角描述，由 `doc/policy.schema.yaml` 约束其结构。
备份器不再拥有独立的清单，而是读取同一份文件并转换视角；恢复器直接消费，不做任何方向上的转换。

Mitigations:
- 视角转换的正确性：备份器的映射逻辑应能独立测试，覆盖“有数据”与“无数据”两类动作；
- 格式漂移：以 schema 为准，策略文件变更后先过校验；
- 术语混淆：在文档中固定使用“策略文件”，并在 ADR-0001 的引用处注明二者同义。

## Related Decisions

- [ADR-0001](0001-separate-backup-and-restore-into-two-programs.md) - 本 ADR 在其“通过 U 盘上的固定文件交互”的契约上具体化，并澄清该 ADR 第 172 行关于“生成物”的表述。

## Links

- [TOML v1.0.0](https://toml.io/en/v1.0.0) - 格式规范
- [JSON Schema 2020-12](https://json-schema.org/draft/2020-12) - schema 所用的规范版本
- [policy.schema.yaml](policy.schema.yaml) - 本决策产生的 schema

## More Information

- **Date:** 2026-11-03
- **Source:** 关于策略文件作用域的讨论，以及对 schema 的多轮审阅

## Audit

### 2026-11-03

**Status:** Pending

**Findings:**

| Finding | Files | Lines | Assessment |
|---------|-------|-------|------------|
| Schema written, policy file not yet created | `doc/policy.schema.yaml` | - | partial |

**Summary:** schema 已存在，策略文件本身与读取它的两个程序尚未实现。

**Action Required:** 编写实际的策略文件，并用它验证 schema 的完备性；在恢复器与备份器实现后更新审计结论。
