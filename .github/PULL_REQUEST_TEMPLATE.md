## What this changes

<!-- One or two sentences, and the issue it fixes if there is one (Fixes #123). -->

## Checklist

- [ ] `cargo test --workspace` passes
- [ ] UI changes: the UI test passes (`app/tests/ui-mock.mjs`, see README)
- [ ] Paths that never ran on a real iPhone are marked `// UNTESTED-ON-DEVICE` and listed in `docs/untested.md`
- [ ] No Apple credentials, keys, tokens or personal data in the diff
