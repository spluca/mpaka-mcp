# MCP Consolidation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Close the MCP review's API-summary and configuration gaps and strengthen REST fixture coverage and setup documentation.

**Architecture:** Extend the existing typed `ApiClient` and generated tool router; preserve the current stdio process model and controls. Add REST behavior alongside current operations.

**Tech Stack:** Rust 2024, rmcp, reqwest, serde, Tokio, Axum test fixtures.

**Spec:** User-provided ChatGPT review dated 2026-11-26; API contract in `rustbox/crates/api/src/lib.rs`.

## Global Constraints

- Default API URL is `https://api.mpaka.dev`.
- Prefer `MPAKA_API_URL`; retain `MPAKA_API_BASE_URL` as a compatibility alias.
- HTTPS is required except for loopback; redirects remain disabled.
- Summary queries accept the backend's optional `since`, `until`, `group_by=day` contract.

## Review Focus

- Both environment aliases set: common `MPAKA_API_URL` takes precedence (config test).
- Missing endpoint setting: production URL is used (config test).
- Invalid usage date order/grouping: rejected before transport (unit test).
- Wrong fixture route or method: explicit 404/405 (contract fixture test).
- Summary with daily series and empty data: schema and totals deserialize (HTTP/tool test).

### Task 1: Production configuration and aggregate usage tool

**Files:** `src/lib.rs`, `src/tools.rs`, README and tool docs.

- Add endpoint alias precedence and production default.
- Add bounded typed `/v1/usage/summary` request and `get_usage_summary` MCP tool.

### Task 2: Contract fixtures and documentation

**Files:** smoke test scripts and README/client configuration docs.

- Tighten method/path fixtures and verify auth/idempotency, status handling, identifiers, and ambiguous retry semantics where represented by current tests.
- Document Claude Desktop, Claude Code, Codex, Cursor, and VS Code stdio configuration.

### Task 3: Verification

- Run `cargo fmt --check`, `cargo test`, and MCP smoke tests where their dependencies are available.
