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
- User-visible strings go through `leh-gtk/src/i18n.rs`: `tr("…")` for plain text, `trf("… {name} …", &[("name", v)])` for placeholders, `tr_n("{n} device", "{n} devices", n)` for counts, and `n_("…")` inside constants (translate with `tr` where shown). Never build sentences with `format!`.
- After adding or changing strings, run `python3 scripts/update-po.py` and translate the new entries in `po/es.po` (Poedit works). `cargo test` checks that placeholders survive translation.
- To add a language: copy `po/linuxeasyhotspot.pot` to `po/<code>.po`, translate it, add it to `CATALOGS` in `i18n.rs` and to `leh_model::Language`.
- Package changes: test with `./scripts/build-deb.sh` and `apt-get install -s ./target/debian/*.deb`.
- Add an entry under `[Unreleased]` in `CHANGELOG.md`.

By participating you agree to follow the [Contributor Covenant](https://www.contributor-covenant.org/version/2/1/code_of_conduct/).
