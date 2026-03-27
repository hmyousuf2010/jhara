# Security policy

## Supported versions

Jhara is pre-release software. There's no stable version yet. Security fixes will be applied to the latest commit on `main` only.

Once v1.0.0 ships, this policy will be updated with a proper support matrix.


## Reporting a vulnerability

Please don't open a public GitHub issue for security vulnerabilities.

Email [hmyousuf@gmail.com](mailto:hmyousuf@gmail.com) with the subject line `[SECURITY] Jhara - brief description`. Include as much detail as you can: what you found, how to reproduce it, what the impact is, and any suggested fixes.

You'll get a response within 72 hours acknowledging receipt. After that, the timeline depends on severity:

- **Critical** (data loss, silent deletion of non-artifact files, privilege escalation): fix within 7 days
- **High** (incorrect safety classification leading to deletion of important files): fix within 14 days
- **Medium** (information disclosure, minor bypass): fix within 30 days
- **Low** (everything else): next regular release

If you've found something critical, mention it in the email and we'll coordinate a disclosure timeline that gives time to patch before you publish anything.


## Scope

Jhara is a disk cleaner that requests Full Disk Access on macOS. That makes the attack surface and the potential impact both worth taking seriously.

Things that are in scope:

- Silent deletion of files outside the declared safety tiers
- Traversal escaping intended scan boundaries (e.g., crossing device boundaries unexpectedly, following symlinks into unexpected locations)
- iCloud hydration triggered by scanner traversal (this would cause silent bandwidth/storage costs)
- License validation bypass
- The web dashboard or server API leaking user data or license tokens
- XPC communication between the main app and background agent (if implemented)
- Privilege escalation or sandbox escape in the macOS app
- Incorrect blocklist matching that causes blocklisted files (`.env`, `terraform.tfstate`, `*.pem`) to be presented as deletable

Things that are out of scope:

- Issues that require physical access to the machine
- Social engineering attacks
- Issues in dependencies that don't affect Jhara specifically (report those upstream)
- Performance issues
- Bugs that don't have a security impact
- Features you want added


## What Jhara does with your data

The free tier sends nothing anywhere. The scanner runs locally, classification runs locally, deletion runs locally. No telemetry, no phone-home, no analytics.

The Pro tier makes one network call: a license activation/validation request to `jhara.app` (via Lemon Squeezy's API). This sends your license key and a machine identifier. Nothing about your filesystem, scan results, or projects is transmitted.

The web dashboard at `jhara.app` has standard server logs (IP, timestamp, route). These aren't shared with anyone.

If you're auditing the network behavior yourself, the relevant code is in `packages/auth/src/lib/payments.ts` and the Swift `LicenseKeychainManager` (when it exists).


## Dependency security

Rust dependencies are managed via `Cargo.lock`. JavaScript dependencies are managed via `pnpm-lock.yaml`. We run `cargo audit` and `pnpm audit` in CI.

If you find a vulnerable dependency, you can either report it here or directly to the dependency maintainer. Either is fine.


## Credit

If you report a vulnerability and it leads to a fix, you'll be credited in the release notes (unless you'd prefer to stay anonymous, just say so in the email).

---

*This policy will be updated as the project matures.*
