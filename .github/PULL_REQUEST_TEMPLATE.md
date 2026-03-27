## What this PR does

<!-- Describe the change. What problem does it solve? What does it add? Be specific. -->

## Type of change

- [ ] Bug fix
- [ ] New ecosystem / signature
- [ ] New feature
- [ ] Performance improvement
- [ ] Refactor (no behavior change)
- [ ] Documentation
- [ ] CI/CD / tooling
- [ ] Other (describe):

## Related issues

<!-- Link any related issues. Use "Closes #123" to auto-close on merge. -->

## Testing

<!-- How did you test this? What tests were added or modified? -->

- [ ] `cargo test -p jhara-core` passes
- [ ] `pnpm biome check` passes (for JS/TS changes)
- [ ] New tests added for new behavior
- [ ] Existing tests updated if behavior changed

## For ecosystem / signature PRs

- [ ] New `ProjectSignature` entry in `signatures.rs` (or `frameworks.rs`)
- [ ] Test in `detector/mod.rs` that exercises the detection path
- [ ] Safety tier rationale in a comment on each `ArtifactPath`
- [ ] Verified the descriptor filename doesn't conflict with another ecosystem

## For scanner / performance PRs

- [ ] Benchmark numbers included (before and after)
- [ ] Cross-platform tested or noted which platforms were tested

## For macOS app PRs

- [ ] Builds cleanly in Xcode against latest `libjhara_core.a`
- [ ] Dark mode works
- [ ] VoiceOver accessibility checked (for UI changes)

## Breaking changes?

<!-- Does this change any public API, FFI surface, or behavior that other parts depend on? -->

- [ ] No breaking changes
- [ ] Yes, breaking changes (describe what changes and who needs to update):

## Screenshots / recordings

<!-- For UI changes, include before/after screenshots or a short screen recording. -->

---

<!-- Reviewers: check that safety classifications are correct and tests cover the new behavior. -->
