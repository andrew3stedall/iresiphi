# Claude Code guidance

Repo-local skills are available under `.claude/skills/`. Use a skill when its description matches the task or the user invokes it. These are mirrored in `.agents/skills/` for Codex.

Use `unslop` when asked to revise prose or documentation to remove AI writing patterns.

Follow the execution architecture policy in [AGENTS.md](AGENTS.md): x86 workloads
run through GitHub Actions; ARM-compatible work may run locally on ARM.
