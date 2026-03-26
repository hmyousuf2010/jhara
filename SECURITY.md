# security policy

we take the security of your files and data very seriously. jhara requires full disk access on macOS, which is a lot of trust. we're building this project in the open so you can see exactly how we use that trust.

## our security posture

- **local only:** jhara doesn't upload your filenames, project structures, or any data to our servers. the only thing that ever leaves your machine is the license key check (if you're a pro user).
- **no telemetry:** there's no tracking, no "anonymous usage stats," and no background phone-homes. i'm building this for developers who care about privacy.
- **core auditor:** the scanning logic is in a single rust crate (`jhara-core`), making it easier to audit than a sprawling multiprocess app.

## reporting a vulnerability

if you find a security issue, please don't open a public issue. it just gives everyone a chance to exploit it before we can fix it.

email me directly at **hmyousuf2010@gmail.com**. i'm a solo dev, so i'll acknowledge the report as fast as i can. 

### what to report

we're especially interested in:
- bugs that lead to unintended file deletion.
- ways to bypass the "caution" or "risky" tier protections.
- local privilege escalation through the background automation service (`SMAppService`).

## disclosure policy

1. **triage:** i'll look at the report and confirm if it's a real issue. 
2. **fix:** i'll prepare a patch in a private branch.
3. **release:** once the fix is ready, i'll push a new version and notify you. 
4. **credit:** i'll give you full credit in the changelog and the repo itself.

thanks for helping keep jhara safe for every developer.

---

*Author: H.M. Yousuf*
*Last updated: March 2026*
