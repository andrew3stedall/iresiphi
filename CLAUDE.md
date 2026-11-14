# Claude Code guidance

Repo-local skills are available under `.claude/skills/`. Use a skill when its description matches the task or the user invokes it. These are mirrored in `.agents/skills/` for Codex.

Use `unslop` when asked to revise prose or documentation to remove AI writing patterns.

Follow the execution architecture policy in [AGENTS.md](AGENTS.md): all builds,
including ARM-compatible builds, run through GitHub Actions on the matching
GitHub-hosted runner. Do not build locally on ARM.
