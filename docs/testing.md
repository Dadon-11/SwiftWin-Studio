# Testing your app on the iPhone

SwiftWin has no XCTest / XCUITest runner: Apple's test tooling needs a Mac. Instead, an AI agent (Claude Code
or Codex) or you, through the MCP tools, drives the real iPhone and checks what's on screen. It's the same
idea as an acceptance test: launch the app, tap through the screens, and verify the result.

Connect your agent first (see [the README](../README.md#use-it-with-claude-code-or-codex)).

## The tools you test with

| Tool | What it does |
|---|---|
| `launch_app` / `terminate_app` | Start (fresh) or close the app, without hunting for it on the home screen. `launch_app` can also use a larger text size to test Dynamic Type. |
| `ui_tree` | The accessibility tree: every element's type, label, value and position. This is what the agent "sees". |
| `tap`, `swipe`, `type`, `press_button` | Act on the screen. Tap by label (`tap "Add"`) rather than by coordinates. |
| `wait_for` | Wait until a label or text appears (or disappears). With `timeout=0` it checks once, so it works as an **assertion**: "is this on screen right now?". If it fails, it lists what is on screen. |
| `screenshot` | An exact picture of the screen. |
| `logs` | The app's own log lines (`app_only`). Log with `os.Logger` and `privacy: .public` so values are readable. |
| `crash_reports` | The app's crash reports, if it crashed during the test. |
| `appearance` | Switch the whole phone to dark or light mode, to test both. |

## Write a test as a prompt

Ask your agent, in plain words. For example:

> Use swiftwin: build and run my app. Tap "Add glass" three times, then check that "3 of 8" is on screen. Tap
> "Reset", confirm the alert, and check that "0 of 8" is on screen. Switch to dark mode and take a
> screenshot. If anything fails, read the app's logs and tell me why.

The agent will `install_run`, tap the buttons, call `wait_for` with `timeout=0` to assert each result, and report
what passed and failed. Because it reads the accessibility tree, it also notices things a fixed script would
miss: a button that is off screen, a missing label, an error alert.

## Tips

- Give controls clear accessibility labels. That is how the agent (and VoiceOver) finds them.
- Ask for the same flow again after each change. It's a quick regression check.
- Add `Logger(subsystem: <your bundle id>, ...)` lines at the important steps, and ask the agent to read them.
- Test text sizes with `launch_app` + `text_size` (for example `accessibility-xl`) and both appearances.

## What this is not

- It is not XCUITest: there are no `XCTestCase` files, no test targets, and nothing runs in CI. An agent runs
  the checks when you ask.
- Checks live in your prompt (or a saved prompt), not in code you can re-run unattended.
- It runs on the real iPhone plugged into your PC, not on a Simulator.
