# Contributing to SwiftWin

Thanks for helping! A few things keep the project easy to work on.

## Set up

Windows 10/11 x64 with Rust (stable, MSVC) and Node 20+. You don't need an iPhone: everything runs
against a simulated one with `--mock`. See the README, "Build from source".

```powershell
cargo test --workspace
cargo run -p swiftwin-cli -- --mock demo
cd app; npm ci; npm run tauri dev -- -- --mock
```

## Before you open a pull request

- `cargo test --workspace` passes, and `cargo clippy --workspace` adds no warnings.
- UI changes: the browser UI test passes (`app/tests/ui-mock.mjs`, how to run it is in its header).
- New behavior has a test. Device code goes behind the `Device` trait so `MockDevice` can test it.
- Code that has never run on a real iPhone or Apple account is marked `// UNTESTED-ON-DEVICE` and
  listed in [docs/untested.md](docs/untested.md).
- New external tools or services go in [docs/dependencies.md](docs/dependencies.md) with their license.
- Anything touching the local API, accounts, tokens, downloads or what Claude may do: read
  [docs/security.md](docs/security.md) and update it.
- Never commit Apple credentials, certificates, keys, tokens or the iOS SDK.

Keep changes small and focused, and write commit messages that say why.

## Reporting bugs

Open an issue with what you did, what happened, what you expected, your Windows and iOS versions, and
the output of `swiftwin doctor`. Security problems: see [SECURITY.md](SECURITY.md), not public issues.

By contributing you agree that your contribution is licensed under the [MIT License](LICENSE).
