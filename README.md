# mde

A small, fast, native Markdown editor for macOS, written in Rust.

## Build and run

    ./scripts/bundle.sh
    open target/release/mde.app

From the terminal: `cargo run --release -- samples/demo.md`

## Using it

The commands are in the macOS menu bar. The window header holds the
Editor / Split / Preview switch and the theme menu, and a tab bar appears once
a second file is open.

## Shortcuts

| Keys | Action |
|------|--------|
| Cmd+1 / 2 / 3 | Editor / Split / Preview |
| Cmd+N / O | New / Open |
| Cmd+S / Shift+Cmd+S | Save / Save As |
| Cmd+W / Q | Close tab / Quit |

## Known limits

- The editor is custom-built and does not publish its text to macOS accessibility, so
  VoiceOver cannot read it, and tools that insert text that way will not work.
- Right-to-left text is not supported in the editor.
- A single extremely long line (hundreds of KB without a break) is still slow.
- Double-clicking a file in Finder does not open it yet; use Open or drag and drop.
- Quitting from the Dock menu or logging out may skip the unsaved-changes prompt.
- The controls are drawn by the app to resemble macOS; they are not Apple's own.
- No find and replace, folder sidebar, export or math.

## Development

    cargo test
    cargo clippy --all-targets
    cargo test --release perf -- --ignored --nocapture   # timings
    MDE_PERF=1 cargo run --release -- file.md            # live timings
