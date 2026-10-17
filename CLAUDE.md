# CLAUDE.md — UDM (Ubai Download Manager)

All repository rules live in `AGENTS.md` (canonical, English). Indonesian translation: `AGENTS.id.md`.

@AGENTS.md

Quick reminders for every Claude Code session:

1. Start with `plan/STATUS.md`, `plan/execution-plan.md`, `plan/architecture.md` and `plan/adr/`, then follow the autonomous protocol in `plan/execution-plan.md` §6.
2. `cargo xtask ci` must be green before every commit. Commit messages, code comments and everything in `plan/` are in English.
3. No `unwrap`/`expect`/`panic!`/`unsafe` in production code; hostile-input tests are named `hostile_*`.
4. Never deliver a corrupt file: validators before resume, verified `Content-Range`, coverage map + size (+ checksum) before renaming `.udmpart`.
5. Clean-room toward Internet Download Manager: observe behaviour only, never inspect or copy it. No DRM, encrypted-stream or paywall circumvention. No GPL/LGPL code.
6. All UI text through `crates/udm-i18n/locales/{id,en}/udm.ftl`; default locale `id`, fallback `en`. Docs in `docs/en/` and `docs/id/`.
7. After any UI change: `cargo run -p udm-app -- --sample --control 7878 --data-dir .udm-dev\agent-a`, then screenshot in `id` and `en` via `cargo xtask ctl 7878 '<json>'` and look at the images.
