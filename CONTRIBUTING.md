# contributing to jhara

thanks for even thinking about contributing. it's a solo project right now, so any help with the 80+ detection rules or fixing my messy swift code is massive.

## setup

the [README](README.md) has everything you need to get the monorepo running. you'll need rust, xcode if you're on mac, and bun for the web and tauri stuff. pnpm won't work because the project is locked to bun itself.

## how to contribute

i'm a bit picky about the core scanning logic because it's got to be fast. if you want to add a new project type to `jhara-core`, please add a test case in `crates/jhara-core/src/detector/tests.rs` with it. 

### pull requests

1. check the issues map first to see if someone's already on it. 
2. if it's a big change, just open a discussion or ping me. i don't want you to waste time on something i might not merge.
3. keep the commit messages clean. we use commitlint, so follow the `feat:`, `fix:`, `docs:` convention.

### code style

we use [biome](https://biomejs.dev/) for everything typescript and `rustfmt` for the core. just run `bun run lint` before you push. it'll save everyone's time.

## cultural note

jhara has its roots in south asian dev culture. comments and documentation in bengali are absolutely welcome alongside english. if you're stuck, just reach out. we're building this together, no?
