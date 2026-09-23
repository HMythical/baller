---
name: Bug report
about: File a bug or enhancement in baller. Ground every claim in the real working tree — verified src/path:line references, a code-level "why this happens," a sandboxed reproduction, and a suggested fix direction. Style reference: baller issue #89.
title: "<short command/area>: <what's broken in one line>"
labels: ["bug"]
assignees: []
---

## Summary

<what breaks, and the user-visible impact: which command, what output/exit-code/DB state is wrong, and for whom it matters>

## Affected code

- `src/path/file.rs:LINE` — <what this code does>
- `src/path/file.rs:LINE-LINE` — <what this code does>

Every reference must be verified against the current working tree before filing. Do not carry stale line numbers from memory.

## Why this happens

<code-level walkthrough of the root cause, and why it originated:
reference git history where available ("introduced by commit <hash> in PR #X")>

## Reproduction

How to trigger it — sandboxed only:

- Prefer an existing unit test in the tree (name the test and its file:line) — run it if it's offline-safe.
- Or a precise static walkthrough with exact file:line.
- Or a would-be unit test that describes the assertion without inventing outputs.

Never invent terminal output, exit codes, or error strings. Never run live-network, sudo, or system-install steps in a reproduction.

## Expected behavior

<what should happen instead, in terms of user-visible output / exit codes / DB state>

## Suggested fix direction

<concrete options grounded in the code — file, approach, trade-offs>

## Environment

<branch, platform, related issue numbers>
