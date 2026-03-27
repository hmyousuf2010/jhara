---
name: New ecosystem / signature
about: Add detection support for a new language, framework, or tool
title: "[ECOSYSTEM] Add support for <name>"
labels: ecosystem, needs-review
assignees: hmyousuf
---

## Ecosystem details

**Name:** (e.g., Gleam, Bun, Bazel, etc.)

**Descriptor file:** (the file that identifies this project type, e.g., `gleam.toml`, `bun.lockb`)

**Language / category:** (e.g., functional language, build system, monorepo tool)

## Artifact paths

For each artifact directory, specify the relative path from the project root, the proposed safety tier, and the recovery command.

| Path | Safety tier | Recovery command |
|------|-------------|-----------------|
| e.g., `build/` | Safe | `gleam build` |
| e.g., `deps/` | Safe | `gleam deps download` |

**Safety tier definitions:**
- **Safe**: regenerated automatically by the build tool, no developer intervention needed
- **Caution**: expensive to rebuild, has historical value, or contains user-managed state
- **Risky**: contains state that can't be recovered from version control
- **Blocked**: should never be deleted under any circumstance

## Global caches (if any)

Does this ecosystem have a global cache outside the project directory? (e.g., `~/.gleam/`, `~/.m2/repository/`)

If yes, where?

## Confidence signals

What makes this ecosystem unambiguous? Does the descriptor filename appear in other contexts? Is a content check needed (like checking `pyproject.toml` to distinguish Poetry from plain pip)?

## Test structure

Please describe what a minimal test directory would look like for this ecosystem. What files need to exist, and what should the test assert?

```
project/
├── gleam.toml     ← descriptor
└── build/         ← artifact (Safe, `gleam build`)
```

## References

- Official docs / repo:
- Any prior art in other cleaners (CleanMyMac, DevCleaner, etc.):

---

**Note:** PRs adding new ecosystems are welcome but require a passing test before merge. See CONTRIBUTING.md for the test pattern.
