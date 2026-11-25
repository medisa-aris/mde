# mde

A small, fast, native Markdown editor for macOS, written in Rust.

## Build and run

    ./scripts/bundle.sh
    open target/release/mde.app

From the terminal: `cargo run --release -- samples/demo.md`

## Shortcuts

| Keys | Action |
|------|--------|
| Cmd+1 / 2 / 3 | Editor / Split / Preview |
| Cmd+N / O | New / Open |
| Cmd+S / Shift+Cmd+S | Save / Save As |
| Cmd+W / Q | Close tab / Quit |

## Known limits

- The editor is custom-built and is not exposed to screen readers such as VoiceOver.
- Right-to-left text is not supported in the editor.
- A single extremely long line (hundreds of KB without a break) is still slow.
- Double-clicking a file in Finder does not open it yet; use Open or drag and drop.
- Quitting from the Dock menu or logging out may skip the unsaved-changes prompt.
- No find and replace, folder sidebar, export or math.

## Development

    cargo test
    cargo clippy --all-targets
    cargo test --release perf -- --ignored --nocapture   # timings
    MDE_PERF=1 cargo run --release -- file.md            # live timings
