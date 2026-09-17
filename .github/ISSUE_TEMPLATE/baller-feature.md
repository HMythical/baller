---
name: Feature request
about: Propose an enhancement to baller.
title: "<command/area>: <what feature you want in one line>"
labels: ["enhancement"]
assignees: []
---

## Summary

<what the feature is, in one or two sentences, and the user-visible capability it unlocks>

## Why this is needed

<who needs it and why: what pain it removes, what workflow it unblocks, what users are currently doing instead (workarounds, shell aliases, manual steps)>

## Affected code

- `src/path/file.rs:LINE` — <what this code does>
- `src/path/file.rs:LINE-LINE` — <what this code does>

Every reference must be verified against the current working tree before filing. Do not carry stale line numbers from memory.

## Why we don't have this today

<code-level walkthrough of what currently exists instead:
- how the closest related path behaves today (verified src/path:line)
- why the function/feature was never implemented or was left incomplete, grounded in git history where available ("removed in <hash>/PR #X", "left as a TODO/future-goal in docs/registry.md:LINE", "listed as not-implemented in docs/commands.md:LINE")>

## Expected behavior

<user-visible behavior once implemented: command + flags, output shape (JSON schema inclusive), exit codes, DB state. Do not invent output you have not produced; prefer a would-be unit test or static walkthrough>

## Suggested implementation direction

<concrete, code-grounded options: new file vs. extension of an existing source, approach, trade-offs, and where the feature hooks into the existing source chain (github/registry/chocolatey/system/cargo/baller) or config (src/config/config.rs)>

## Environment

<branch, platform, related issues (e.g. "closes #10", "related to #89"), target repo>
