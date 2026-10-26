# art-term terminal-report patch

This directory contains the minimal build package from Crossterm 0.29.0, licensed under MIT. The upstream LICENSE is retained unchanged. The original crates.io archive SHA-256 is `d8b9f2e4c67f833b660cdb0a3523065869fb35570177239812ed4c905aeff87b`.

The workspace uses one Cargo path patch so the TUI and Ratatui share the same event reader. This patch exposes DA1 and cell/text-area reports through `Event::TerminalResponse`. It keeps ordinary key, mouse and paste decoding, including literal report-shaped text inside bracketed paste. DA parameters are bounded before allocation, geometry uses exactly three integers, and oversized report metadata is discarded through its final byte without becoming keys.

The event source reopens the actual input TTY with NONBLOCK and CLOEXEC. It does not change shared stdin flags or output/raw-mode descriptors. Both Unix backends retain a lone Escape for at most 50 ms, so a reply split after its first Escape can complete. An isolated Escape after that ambiguity window is a key. A longer gap between a lone Escape and `[` cannot be distinguished from ordinary input and is not promised supported. Complete CSI prefixes can remain pending across the image-probe deadline. Input EOF/errors are transferred from the blocking poll worker to the async stream caller before it polls again. This retains errors after edge-triggered readiness has been consumed and avoids spinning or waiting forever.

`UPSTREAM-FILES.json` records the original hashes of every retained upstream file. The changed upstream files are:

- `Cargo.toml`.
- `src/event/filter.rs`.
- `src/event/source/unix/mio.rs`.
- `src/event/source/unix/tty.rs`.
- `src/event/stream.rs`.
- `src/event/sys/unix/parse.rs`.
- `src/event.rs`.
- `src/terminal/sys/file_descriptor.rs`.

Local additions are this note, `UPSTREAM-FILES.json`, and `.gitignore`. Registry metadata, generated lock/build output, upstream media/docs and prototype examples are excluded. Retained upstream examples satisfy the original Cargo manifest.

Validation:

```sh
cargo test --manifest-path vendor/crossterm/Cargo.toml --lib --features event-stream
cargo test --manifest-path vendor/crossterm/Cargo.toml --lib --all-features
cargo check --manifest-path vendor/crossterm/Cargo.toml --no-default-features --features event-stream
cargo build -p art-term-tui --example terminal_events --locked
python3 scripts/tui-regression/terminal_events.py --binary /absolute/target/debug/examples/terminal_events --output-dir /tmp/art-term-terminal-events-result
```

The standalone library checks can create a local Cargo.lock, which is ignored and must be removed after verification. The library's upstream `unused_parens` warning in `src/terminal/sys/unix.rs` is unchanged.

The PTY fixture proves event ordering, split and late replies, malformed recovery, bounded Escape, paste/CJK/Alt/mouse, resize, cancellation, EOF and stdin flag preservation. It does not prove a real terminal renders images. Actual SSH/Sixel painting is verified separately by art-term's candidate workflow.
