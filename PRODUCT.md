# Codemode

<!-- impeccable:product-schema 1 -->

## Platform

web

## Stack

The owner selected build-react-stack: Vite, React, React Compiler, TypeScript 7, TanStack Router and Query, Tailwind and shadcn/ui, managed with Bun. The frontend is embedded in the Rust/axum service binary. The backend follows build-rust-stack.

## Users

The owner operates the control plane. Each user owns a private tenant. Agents connect to /mcp through tokens bound to a workspace selected during OAuth. A separate /mcp-admin resource manages that user’s workspaces and sources.

## Product Purpose

Connect each upstream tool once, expose small MCP endpoints, and let agents search tools and run Python instead of loading hundreds of tool definitions.

## Operating Context

The browser manages sources and their sync state, linked accounts, endpoint bundles, access tokens and execution logs. Provider OAuth consent remains a browser decision. Tool execution uses allow/block policies, default allow. Control-plane sign-in uses email/password registration restricted to ursus95@gmail.com and @olegpustovit.com addresses. The first user has an admin attribute; every user owns a private tenant with their own connected MCPs, OAuth accounts, credentials and agent access. The service runs on oracle-vm under smdctl, behind owner-managed Caddy, at codemode.microapps.space.

## Capabilities and Constraints

- Exactly two execution tools: search and execute. A separate /mcp-admin resource manages the signed-in tenant.
- Credentials are attached by the broker; agents and sandbox workers never receive them.
- Sources and workspaces have stable identities and shareable detail routes.
- One Rust binary serves the frontend and API; Monty is a separate worker binary.
- Phases 0–4 are implemented and deployed: UI, OAuth, OpenAPI/GraphQL sources, HTTP credentials, policies, worker isolation, rate limits, tool scopes, saved scripts and MCP tasks. Live-provider consent/token exchange, claude.ai acceptance and owner live-source approval remain unperformed. The production catalogue is empty.
- The owner instructed implementation to continue through manual acceptance gates, recording unperformed checks without halting.

## Evidence on Hand

The revision 5 implementation plan, AGENTS.md, docs/status.md, backend API contracts and automated checks. No customer claims, brand imagery or product screenshots have been supplied.

## Product Principles

- Make connection state and the next corrective action clear.
- Keep credential entry separate from credential metadata.
- Make endpoint scope visible before authorizing an agent.
- Preserve execution and redacted tool-call evidence for four days after each agent session’s last activity.

## Active replacement

The owner requested the exact final-v5 proposal at https://artifacts.microapps.space/a/hkwwwp90n0dt. Sessions, Integrations, Workspaces, Scripts and Tokens replace the historical navigation. The current local implementation removes approvals/resume and snapshots, stores bounded execution results, and applies four-day session retention. DESIGN.md records that replacement. Deployment and acceptance evidence remain in docs/proposal/implementation.md; historical deployment statements above do not qualify the replacement release.
