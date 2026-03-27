---
name: Bug report
about: Something isn't working right
title: "[BUG] "
labels: bug, needs-triage
assignees: hmyousuf
---

## What happened

<!-- Describe what went wrong. Be specific. "It doesn't work" isn't useful. -->

## What you expected to happen

<!-- What should have happened instead? -->

## Steps to reproduce

<!-- Exact steps to reproduce the issue. -->

1.
2.
3.

## Environment

- **OS and version:** (e.g., macOS 14.5, Ubuntu 24.04)
- **Jhara version / commit:** (run `git rev-parse --short HEAD` if building from source)
- **Rust version:** (run `rustc --version`)
- **Xcode version:** (macOS app issues only)

## Component

<!-- Which part of the codebase is this? Check all that apply. -->

- [ ] jhara-core (Rust scanner, detector, classifier)
- [ ] macOS app (Swift/SwiftUI)
- [ ] Tauri app (Linux/Windows)
- [ ] Web dashboard
- [ ] CI/CD

## Relevant logs or output

```
Paste any relevant error output here.
```

## Additional context

<!-- Anything else that might help. Screenshots, related issues, etc. -->

---

**Important for deletion-related bugs:** If Jhara deleted something it shouldn't have, please include the exact path(s) and what the classifier showed for that path (safety tier, ecosystem, reason). This helps narrow down whether it's a classification error or a deletion logic error.
