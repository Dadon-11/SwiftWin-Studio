# Third-party notices

SwiftWin's own code is under the [MIT License](LICENSE). It builds on other people's work:

## Included in SwiftWin

- **glisse** 0.5.0 (commit `46be5ba`), vendored in `app/src/vendor/glisse/`: MIT License,
  Copyright (c) 2026 Kelvin Zhang. The full notice is in
  [app/src/vendor/glisse/LICENSE](app/src/vendor/glisse/LICENSE) and must stay with that folder.
- **Rust crates** linked into `swiftwin.exe` and the desktop app: almost all MIT and/or Apache-2.0;
  also BSD-2/3-Clause, ISC, Zlib, Unicode-3.0, CDLA-Permissive-2.0, Unlicense, and **MPL-2.0** for
  `cssparser`, `cssparser-macros`, `selectors`, `dtoa-short` (through Tauri) and `option-ext`. MPL-2.0
  is file-level copyleft: they are used unmodified from crates.io, where their source is available.
  `r-efi` is offered under MIT/Apache-2.0/LGPL; SwiftWin uses it under MIT/Apache-2.0. List them all
  with `cargo metadata` or `cargo about`.
- **npm packages** in the desktop UI: `monaco-editor` (MIT), `@tauri-apps/api` (Apache-2.0 or MIT).

## Downloaded or used as separate programs (not part of SwiftWin)

SwiftWin starts these as separate processes. They are downloaded from their publishers (or installed
by the user), never shipped inside the SwiftWin installer, and keep their own licenses:

| Program | License |
|---|---|
| pymobiledevice3 | GPL-3.0 |
| UxPlay (optional) | GPL-3.0 |
| plumesign (Impactor) | MIT |
| rcodesign (apple-codesign) | MPL-2.0 |
| WebDriverAgent | BSD-3-Clause |
| LLVM `ld64.lld` | Apache-2.0 WITH LLVM-exception |
| Swift toolchain | Apache-2.0 WITH Runtime Library Exception |

Running a GPL program as a separate process does not put SwiftWin under the GPL. Don't link or copy
GPL code into SwiftWin without changing its license.

Apple's iOS SDK is downloaded by each user from Apple with their own Apple ID and is never
redistributed. See [docs/dependencies.md](docs/dependencies.md) for what each tool is used for.
