# FaultSift Product

## Responsibility of This Document

This document defines stable product intent: who FaultSift is for, which problem it solves, its core value, its product principles, its first MVP, and its non-goals.

It does not define implementation structures, package layout, algorithms, task status, or release scheduling. Those belong in `ARCHITECTURE.md`, ADRs, task specs, and `ROADMAP.md` respectively.

## Product Definition

FaultSift is an open-source, local-first log incident forensics tool for developers working with huge local log files.

FaultSift is licensed under Apache-2.0.

> **Find the incident, not the keyword.**

Its job is to sift through log noise and reduce millions of lines and thousands of repeated errors to a small set of patterns, suspicious time windows, and evidence that helps explain what happened.

FaultSift begins as a log viewer that understands failures better than keyword search and can grow into a local incident forensics workbench.

## Target Users and Situations

The primary users are developers investigating incidents from large application log files, especially when they need to answer questions such as:

- Which errors are repetitions of the same underlying pattern?
- When did an exception first appear or begin to burst?
- Which rare or new event matters more than long-running noise?
- What happened in the minutes before a critical failure?
- Which original log context supports a suspected pattern?
- Later, can events across several service logs be correlated by trace or request identifiers?

The initial log-domain focus is Java applications, prioritizing Spring Boot logs produced with Logback or Log4j2 and Java multiline stack traces.

## Core Product Value

Traditional viewers are effective at finding text. FaultSift focuses on turning raw log evidence into an investigation path:

```text
local log file
   ↓
suspicious timeline interval
   ↓
grouped failure pattern
   ↓
representative event and original context
   ↓
incident understanding
```

The intended outcome is not merely “find ERROR,” but “identify how the incident developed and which evidence deserves attention first.”

## Product Principles

### Local First

Raw logs are read locally by default. FaultSift does not require uploading files, building a remote index, or depending on a cloud service.

### Huge Files Are the Normal Case

The product must remain useful for multi-GB files and files larger than available RAM. Avoiding whole-file memory loading is a product requirement, not an optional optimization.

### Deterministic Analysis Before AI

FaultSift must provide useful file access, event parsing, exception assembly, pattern aggregation, timeline analysis, new-pattern detection, context navigation, and later anomaly scoring without AI.

### AI Reads Incidents, Not Entire Logs

Optional AI works on selected, structured incident evidence prepared by FaultSift. Local Ollama is the preferred integration direction; a user-configured OpenAI-compatible API may also be supported. AI is an enhancement, never the foundation of correctness.

### Preserve Investigation Evidence

Summaries and scores must lead back to representative events and original local log context so the user can verify conclusions.

### Focus on Incident Forensics

FaultSift should deepen the path from anomalous time range to pattern to context rather than becoming a broad monitoring or dashboard platform.

## First Product MVP

The first usable product MVP contains exactly five core capabilities:

1. Open a huge local log file without full-file loading.
2. Recognize a Java multiline exception as one logical event.
3. Automatically aggregate similar errors into patterns.
4. Visualize WARN and ERROR timelines.
5. Open original log context from a selected pattern.

## Product Non-Goals for the MVP

The first MVP is not intended to provide:

- a log collection agent;
- Kafka ingestion;
- Elasticsearch, OpenSearch, Grafana, or a replacement for them;
- metrics or a trace backend;
- alerting;
- a general dashboard system;
- user accounts, permissions, team collaboration, or other SaaS platform features;
- cloud-first storage or indexing;
- AI analysis of an entire raw log file.

## Longer-Term Direction

After the deterministic MVP is stable, the documented product direction includes:

- search and filters by regex, level, logger, thread, and time range;
- new, rare, and bursting pattern detection and anomaly scoring;
- optional AI incident summaries and natural-language investigation;
- multiple files on a unified timeline;
- traceId, requestId, and sessionId correlation;
- parsers for additional log formats;
- a possible CLI and, much later, a FaultSift MCP interface over structured results.

These are directions, not permission to add them to an earlier task.

## Product Open Questions

- What measurable product-level performance targets, if any, should supplement reproducible benchmark baselines?
