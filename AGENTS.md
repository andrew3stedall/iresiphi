# Agent guidance

Repo-local skills are installed in `.agents/skills/` for Codex and `.claude/skills/` for Claude Code. Use a skill when its description matches the task or the user invokes it. The skills are copied into both directories so each agent can discover them natively.

The Matt Pocock engineering skills and their license are included in those directories. The pstack `unslop` skill is available as `unslop`; apply it when asked to clean up prose or documentation.

## Execution architecture

Run every workload requiring x86 through a GitHub Actions workflow on an x86
runner. ARM-compatible builds, tests and processing may run locally on ARM.
For local development, use native ARM tools and artifacts; use GitHub Actions
for x86 builds, container checks and release certification. Follow
[CI and processor architecture](docs/ci.md) for the workflow and evidence rules.
