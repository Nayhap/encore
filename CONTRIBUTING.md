# Contributing

Thanks for helping. Setup, commands and the CLI are in
[docs/development.md](docs/development.md); how the pieces fit is in
[docs/architecture.md](docs/architecture.md).

## Before you open a pull request

- `cargo test --workspace` and `bun run check` pass.
- UI changes include a before and after screenshot. Use `bun run dev` with the
  mock backend if you don't want your own library in it.
- Keep pull requests to one change. Open an issue first for anything large.

## Ground rules

- **Never commit secrets.** No cookies, tokens, signed stream URLs or personal
  data in code, fixtures, logs or screenshots. Record fixtures with
  `encore-cli record`, which scrubs them.
- **Write it yourself.** Don't copy code from GPL-licensed projects such as
  Meiro, Kopuz or Better Lyrics; Encore is MIT. Reading them to understand how
  YouTube behaves is fine.
- **Match the surrounding code.** Same naming, same comment density, same
  idioms.
- **Write interface text for the person using it.** Sentence case, plain verbs,
  and errors that say what happened and what to do next.

## Reporting a bug

Use the bug report template. Include your macOS version, Encore version, what
you did and what happened. The log at
`~/Library/Logs/app.encore.desktop/encore.log` is redacted, but read it before
attaching it.
