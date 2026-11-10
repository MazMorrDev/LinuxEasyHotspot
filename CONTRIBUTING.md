# Contributing

Thanks for helping! Before opening a PR:

```sh
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

- Keep `leh-model` free of I/O, D-Bus and GUI dependencies. Logic that can be pure belongs there, with tests.
- Snapshot tests use [insta](https://insta.rs). After an intended change, run `cargo insta review`, or `INSTA_UPDATE=always cargo test`, and commit the `.snap` files.
- Never log secrets. Use `NmSettings::redacted()`, and `#[instrument(skip(...))]` on functions that take a password.
- User-visible strings go through gettext (`gettext!`/`i18n()` in `leh-gtk`).
- Add an entry under `[Unreleased]` in `CHANGELOG.md`.

By participating you agree to follow the [Contributor Covenant](https://www.contributor-covenant.org/version/2/1/code_of_conduct/).
