> [!IMPORTANT]
> **Historical reference — non-authoritative.** This document preserves the original product exploration and is no longer maintained as current project truth. Use `PRODUCT.md`, `ARCHITECTURE.md`, `ROADMAP.md`, and accepted ADRs for authoritative decisions.

# FaultSift

> **Sift through logs. Find the fault.**  
> **Find the incident, not the keyword.**

FaultSift 是一个面向开发者的 **本地优先日志故障取证工具**，专注于超大日志文件的快速分析、异常聚类、时间线定位和故障上下文还原。

它不是另一个 ELK，也不是单纯给日志套一个 AI 聊天框。

FaultSift 的目标是：

> **把几 GB、几千万行日志，压缩成少量真正值得关注的异常模式和事故线索。**

---

## 1. 项目定位

传统日志工具大多解决的是：

- 搜索关键词
- 过滤日志级别
- 浏览大文件
- 查看上下文

但真正排查线上问题时，开发者更关心的是：

- 哪些错误其实是同一个问题？
- 某个异常是什么时候开始爆发的？
- 哪些错误是第一次出现？
- 事故发生前几分钟发生了什么？
- 多个服务之间是否存在同一条调用链？
- 一堆 ERROR 里，哪些最值得优先看？

FaultSift 希望解决的是这些问题。

### 一句话定义

> **Open-source local log forensic tool for huge log files.**

中文可以理解为：

> **面向超大日志文件的本地故障取证工具。**

---

## 2. 为什么叫 FaultSift

### Fault

Fault 表示：

- 故障
- 异常
- 错误
- 系统问题

### Sift

Sift 表示：

- 筛选
- 淘出
- 从大量杂质中找出真正有价值的信息

组合起来就是：

> **从海量日志噪音中筛出真正的故障。**

示意：

```text
8 GB 原始日志
      ↓
   FaultSift
      ↓
20000 条 ERROR
      ↓
32 类异常模式
      ↓
3 个真正值得关注的故障
```

FaultSift 不使用 `LogXXX` 命名，也为未来扩展留下空间。

未来它可以不仅处理日志：

```text
FaultSift
 ├── Log
 ├── Stack Trace
 ├── Thread Dump
 ├── GC Log
 ├── Trace
 ├── Crash Dump
 └── Incident Timeline
```

---

## 3. 核心痛点

开发者日常排查日志时，经常依赖：

- EmEditor
- Notepad++
- grep
- ripgrep
- klogg
- lnav
- ELK
- OpenSearch
- Grafana Loki

这些工具各有优势，但存在一个共同问题：

> **它们大多擅长“找到日志”，但不擅长“告诉你问题是什么”。**

典型场景：

```text
15:01:03 ERROR PaymentService - pay failed order=10001
java.net.SocketTimeoutException: Read timed out

15:01:04 ERROR PaymentService - pay failed order=10002
java.net.SocketTimeoutException: Read timed out

15:01:05 ERROR PaymentService - pay failed order=10003
java.net.SocketTimeoutException: Read timed out
```

普通日志工具看到的是：

```text
3 个 ERROR
3 个 Exception
几十行堆栈
```

FaultSift 应该告诉用户：

```text
🚨 SocketTimeoutException

首次出现
15:01:03

最后出现
15:18:42

出现次数
2,381

错误模板
PaymentService - pay failed order=<*>

主要异常
java.net.SocketTimeoutException: Read timed out

爆发时间
15:03 ~ 15:07
```

---

## 4. 与现有工具的差异

FaultSift 不打算和成熟日志工具正面拼“查看器功能大全”。

它的重点是：

| 能力 | 传统日志查看器 | FaultSift |
|---|---:|---:|
| 大文件浏览 | 强 | 强 |
| 搜索/过滤 | 强 | 强 |
| 自动识别异常模式 | 弱 | 强 |
| 相似错误聚类 | 弱 | 强 |
| 错误爆发时间线 | 一般 | 强 |
| 新异常检测 | 弱 | 强 |
| 故障优先级评分 | 弱 | 强 |
| 事故前上下文分析 | 弱 | 强 |
| 本地 AI 分析 | 少 | 强 |
| 多日志调用链串联 | 少 | 后续重点 |

FaultSift 的方向不是：

> “帮我搜到 ERROR。”

而是：

> “帮我找到事故是怎么发生的。”

---

# 5. 核心设计原则

## 5.1 Local First

默认情况下：

- 日志只在本地读取
- 不上传文件
- 不建立远端索引
- 不依赖云服务

AI 能力优先支持：

```text
Ollama
```

同时允许用户自行配置：

```text
OpenAI-compatible API
```

---

## 5.2 AI 不是核心依赖

FaultSift 不应该依赖 AI 才能工作。

即使完全关闭 AI，也应该具备：

- 大文件快速读取
- 日志事件解析
- Exception 合并
- Pattern 聚类
- 时间线统计
- 异常爆发检测
- 新 Pattern 检测
- 上下文查看
- 故障评分

AI 只是最后一层增强。

---

## 5.3 AI 不直接读取整个日志文件

错误方案：

```text
8 GB 日志
 ↓
切 Chunk
 ↓
LLM
 ↓
总结
```

这种方案：

- 慢
- 贵
- Token 消耗巨大
- 容易丢失全局关系
- 难以保证隐私

FaultSift 应采用：

```text
                 8GB 日志
                    │
                    ↓
             Streaming Parser
                    │
       ┌────────────┼────────────┐
       ↓            ↓            ↓
    时间解析      Level解析     Event解析
                                │
                         Java Exception
                         JSON Log
                         普通文本
                                │
                                ↓
                         Pattern Miner
                                │
              ┌─────────────────┼──────────────┐
              ↓                 ↓              ↓
           Pattern A         Pattern B      Pattern C
           182,312次         2,181次         3次
              │                 │              │
              └─────────────────┼──────────────┘
                                ↓
                         Anomaly Engine
                                ↓
                    几百个结构化事件
                                │
                                ↓
                              LLM
```

核心原则：

> **AI reads incidents, not your entire log.**

---

# 6. 日志处理模型

FaultSift 第一阶段重点支持 Java 日志。

推荐优先支持：

- Spring Boot
- Logback
- Log4j2

自动识别：

```text
timestamp
level
thread
logger
message
traceId
requestId
```

同时正确识别 Java 多行异常：

```text
java.lang.RuntimeException
    at xxx
    at xxx
Caused by: ...
    at xxx
... 36 common frames omitted
```

整段 Stack Trace 必须被视为一个 Event，而不是几十条独立日志。

---

# 7. Pattern 聚类

这是 FaultSift 最重要的基础能力之一。

例如：

```text
User 12831 login failed from 10.1.1.2
User 78391 login failed from 10.1.1.8
User 92382 login failed from 192.168.1.5
```

归一化：

```text
User <NUM> login failed from <IP>
```

再例如：

```text
Order CL202608240001 not found
Order CL202608240002 not found
Order CL202608240003 not found
```

归一化：

```text
Order <*> not found
```

建议识别：

- UUID
- IPv4
- IPv6
- 数字
- 时间
- 日期
- URL
- 文件路径
- 十六进制
- traceId
- requestId
- sessionId
- 订单号
- 动态业务编号

之后生成：

```text
normalizedPattern
        ↓
hash
        ↓
patternId
```

例如：

```text
31,000,000 行日志
        ↓
287 个主要 Pattern
```

Pattern 聚类可以参考 Drain / Drain3 的思想，但 FaultSift 的实现可以根据桌面工具场景做进一步优化。

---

# 8. 时间线分析

FaultSift 应提供异常爆发时间线。

例如：

```text
ERROR / min

14:50 ▏
14:51 ▏
14:52 ▏
14:53 ▎
14:54 ▎
14:55 █
14:56 █████
14:57 ███████████████████
14:58 ███████████████████████
14:59 █████████████
15:00 ████
15:01 █
```

用户点击：

```text
14:57
```

立即展示：

```text
这一分钟：

SocketTimeoutException     1821
RedisTimeoutException       291
DubboTimeoutException        81
UnknownHostException          3
```

时间线不只是展示 ERROR 数量。

后续可以支持：

- WARN
- ERROR
- Exception
- Pattern
- 新 Pattern
- Pattern Burst
- Thread
- Logger
- traceId

---

# 9. 新异常检测

“第一次出现”往往比“出现次数最多”更重要。

例如：

```text
NullPointerException
100000 次
整个日志期间一直存在
```

可能只是长期噪音。

但：

```text
OutOfMemoryError
1 次
15:08 第一次出现
```

反而可能是最关键事件。

因此 FaultSift 应重点标记：

- First Seen
- Newly Appeared Pattern
- Rare Pattern
- Sudden Burst
- Frequency Change

---

# 10. Anomaly Score

第一版不需要机器学习。

简单规则已经可以产生非常有价值的结果。

例如：

```text
OutOfMemoryError

Severity          +30
New Pattern       +25
Rare              +10
Burst             +20
StackTrace        +10
─────────────────────
Anomaly Score      95
```

首页可以直接展示：

```text
最值得关注

95  OutOfMemoryError
89  RedisTimeoutException
73  DubboTimeoutException
42  NullPointerException
```

后续 Anomaly Score 可以综合：

```text
severity
firstSeen
frequency
frequencyDelta
burstScore
rarity
exceptionType
stackTrace
contextCorrelation
crossServiceCorrelation
```

---

# 11. 事故前 5 分钟

这是 FaultSift 非常值得做的特色功能。

用户选中：

```text
OutOfMemoryError
15:32:18
```

点击：

```text
分析此前发生了什么
```

FaultSift 自动分析：

```text
15:27 ~ 15:32
```

得到：

```text
15:27 Redis timeout ↑ 32%
15:28 HTTP 请求耗时 WARN ↑ 310%
15:29 GC overhead warning 首次出现
15:30 Full GC 高频出现
15:31 OutOfMemory 相关日志首次出现
15:32 OutOfMemoryError
```

核心产品价值：

> **不是告诉用户最后报了什么错，而是尽可能还原事故发生过程。**

---

# 12. AI / Ollama

AI 应建立在 FaultSift 已经完成的结构化分析之上。

例如用户问：

> 昨天下午 3 点发生了什么？

FaultSift 先转换时间范围：

```text
14:50 ~ 15:10
```

然后提取：

```text
异常 Pattern
新增 Pattern
ERROR 数变化
WARN 数变化
频率异常
代表性日志
前后上下文
```

给 LLM 的数据可以是：

```json
{
  "timeRange": "14:50 - 15:10",
  "totalEvents": 128721,
  "errorCount": 8321,
  "newPatterns": 3,
  "patterns": [
    {
      "template": "Redis connection <*> timed out",
      "count": 7122,
      "firstSeen": "15:02:12",
      "peak": "15:03:01"
    }
  ]
}
```

LLM 最后输出类似：

```text
15 点附近的故障很可能由 Redis 连接异常开始，
随后导致 PaymentService 请求堆积，
并进一步触发 Dubbo 调用超时。

Redis 在 15:03 左右恢复后，
下游错误数量同步下降。
```

---

# 13. 多日志关联

后续可以支持一次拖入多个日志：

```text
gateway.log
order.log
payment.log
redis.log
```

如果存在：

```text
traceId=abc123
```

FaultSift 自动构建：

```text
abc123

15:01:01 Gateway
    ↓
15:01:01 OrderService
    ↓
15:01:02 PaymentService
    ↓
15:01:32 RedisTimeout
    ↓
15:01:32 Payment failed
    ↓
15:01:33 Order rollback
```

这时 FaultSift 就不再只是日志查看器，而是：

> **本地故障取证工具。**

---

# 14. 技术栈

推荐：

```text
Rust
+
Tauri 2
+
React
```

原因：

FaultSift 的核心场景天然适合 Rust：

- GB 级文件
- mmap
- 多线程扫描
- 字符串处理
- 正则
- 文件索引
- 低内存
- 高性能搜索

整体架构：

```text
React
│
│ Tauri Command
↓
Rust Core
│
├── FileReader
├── LineIndexer
├── EventParser
├── PatternMiner
├── TimelineAggregator
├── AnomalyDetector
├── SearchEngine
└── AIContextBuilder
       │
       ├── Ollama
       └── OpenAI-compatible API
```

---

# 15. 大文件设计

FaultSift 的核心原则：

> **永远不要把整个日志文件读入内存。**

禁止：

```text
readToString()
```

禁止：

```text
Vec<String>
```

推荐：

```text
File
 ↓
Memory Map
 ↓
建立 newline offset
 ↓
Event index
```

只保存必要索引：

```rust
struct LogEventIndex {
    offset: u64,
    length: u32,
    timestamp: i64,
    level: LogLevel,
    pattern_id: u32,
}
```

日志原文始终留在磁盘。

需要显示时：

```text
offset
 ↓
mmap
 ↓
slice
```

---

# 16. 前端性能

前端不能直接接收几十万甚至几百万条日志。

推荐：

```text
visible range:

12001 ~ 12100
```

使用：

- Virtual List
- Windowed Rendering
- Lazy Load
- Incremental Query

Tauri 后端只返回当前需要显示的数据。

---

# 17. MVP — V0.1

第一版建议只做 5 个核心能力：

1. 拖入超大日志，不全量加载
2. 识别 Java 多行 Exception
3. 自动聚合相似错误
4. ERROR / WARN 时间线
5. 点击 Pattern 查看原始日志上下文

暂时不要做：

- 日志采集 Agent
- Kafka
- Elasticsearch
- OpenSearch
- Grafana
- Metrics
- Trace Backend
- 告警
- Dashboard 系统
- 用户体系
- 团队协作
- 权限系统

否则 FaultSift 很容易变成第二个 ELK。

---

# 18. MVP 界面草图

```text
┌─────────────────────────────────────────────────────────┐
│ server.log                         4.82 GB / 18,281,221行 │
├─────────────────────────────────────────────────────────┤
│                                                         │
│ ERROR 时间线                                             │
│             ▁▁▂▂▃▇████▆▃▂▁                              │
│                        ↑                                │
│                    15:03 爆发                           │
│                                                         │
├───────────────────────────────┬─────────────────────────┤
│ 异常模式                       │ 次数        首次出现      │
│                               │                         │
│ 🔴 SocketTimeoutException     │ 8,231      15:02:11    │
│ 🔴 RedisConnectionException   │ 2,918      15:02:03    │
│ 🟠 DubboTimeoutException      │   811      15:03:41    │
│ 🟡 NullPointerException       │    32      11:28:02    │
│                               │                         │
├───────────────────────────────┴─────────────────────────┤
│ 原始日志                                                │
│                                                        │
│ 15:02:11 ERROR PaymentService...                       │
│ java.net.SocketTimeoutException...                     │
│     at ...                                             │
└─────────────────────────────────────────────────────────┘
```

---

# 19. Roadmap

## V0.1

```text
大文件读取
Java Exception 解析
Pattern 聚类
ERROR / WARN 时间线
上下文查看
```

## V0.2

```text
全文搜索
Filter
Regex
Level
Logger
Thread
时间范围
```

## V0.3

```text
Anomaly Score
新 Pattern
Rare Pattern
Burst Detection
```

## V0.4

```text
Ollama
OpenAI-compatible API
AI 故障摘要
```

## V0.5

```text
自然语言查询

“下午 3 点发生了什么？”
“这个异常第一次是什么时候出现？”
“事故前 5 分钟有什么异常？”
```

## V0.6

```text
多个日志文件
统一时间线
跨文件 Pattern
```

## V0.7

```text
traceId
requestId
sessionId

自动构建调用链
```

## V0.8

```text
日志解析插件

Java
Nginx
Node.js
Python
.NET
JSON Log
```

## V1.0

```text
完整本地 Incident Forensics Workbench
```

---

# 20. 推荐模块结构

```text
faultsift/
├── apps/
│   └── desktop/
│
├── crates/
│   ├── faultsift-core/
│   ├── faultsift-parser/
│   ├── faultsift-index/
│   ├── faultsift-pattern/
│   ├── faultsift-timeline/
│   ├── faultsift-anomaly/
│   ├── faultsift-search/
│   └── faultsift-ai/
│
├── web/
│   └── ui/
│
├── examples/
│
├── docs/
│
└── README.md
```

---

# 21. CLI 设计

未来可以提供 CLI：

```bash
faultsift server.log
```

分析：

```bash
faultsift analyze server.log
```

查看 Pattern：

```bash
faultsift patterns server.log
```

分析指定时间：

```bash
faultsift analyze server.log \
  --from "2026-08-25 15:00:00" \
  --to   "2026-08-25 15:10:00"
```

指定 Ollama：

```bash
faultsift analyze server.log \
  --ai ollama \
  --model qwen3
```

---

# 22. 推荐仓库与包名

GitHub：

```text
faultsift/faultsift
```

Rust：

```text
faultsift-core
faultsift-parser
faultsift-pattern
faultsift-anomaly
```

应用：

```text
FaultSift Desktop
```

CLI：

```text
faultsift
```

---

# 23. 产品文案

推荐主标题：

> **FaultSift — Sift through logs. Find the fault.**

推荐副标题：

> **Find the incident, not the keyword.**

README 简介：

> FaultSift is an open-source, local-first log forensic tool designed for huge log files.  
> It clusters similar errors, detects suspicious patterns, visualizes incident timelines, and helps developers understand what actually happened — without uploading logs to the cloud.

中文：

> FaultSift 是一个开源、本地优先的日志故障取证工具。  
> 它面向 GB 级日志文件，通过异常聚类、时间线、异常评分和本地 AI，帮助开发者快速定位真正值得关注的问题，而不是在海量日志里反复搜索关键词。

---

# 24. 最终产品方向

FaultSift 最初可以只是：

> **一个比 grep 更懂故障的日志查看器。**

然后逐步成长为：

```text
日志查看
   ↓
异常聚类
   ↓
时间线
   ↓
故障评分
   ↓
事故上下文
   ↓
AI 分析
   ↓
多日志关联
   ↓
traceId 调用链
   ↓
Incident Forensics
```

最终定位：

> **FaultSift = Local Incident Forensics for Developers**

不是日志平台。

不是监控平台。

不是 APM。

而是：

> **打开事故现场，然后帮开发者把真正的故障从噪音中筛出来。**
