#[cfg(unix)]
#[tokio::main]
async fn main() -> std::io::Result<()> {
    use crossterm::{
        event::{
            DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
            Event, EventStream, KeyCode, KeyModifiers, TerminalResponse,
        },
        execute,
        terminal::{disable_raw_mode, enable_raw_mode},
    };
    use futures_util::StreamExt;
    use serde_json::json;
    use std::io::{self, Write};

    struct Restore;
    impl Drop for Restore {
        fn drop(&mut self) {
            let _ = execute!(io::stdout(), DisableBracketedPaste, DisableMouseCapture);
            let _ = disable_raw_mode();
        }
    }
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    let mut interrupt = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())?;
    enable_raw_mode()?;
    let _restore = Restore;
    execute!(io::stdout(), EnableBracketedPaste, EnableMouseCapture)?;
    let mut events = EventStream::new();
    #[cfg(target_os = "linux")]
    if let Some(path) = std::env::var_os("ART_TERM_EVENTS_FD_AUDIT") {
        let tty = std::fs::read_link("/proc/self/fd/0")?;
        // SAFETY: fcntl(F_GETFL) only reads flags from a live descriptor.
        let stdin_flags = unsafe { libc::fcntl(0, libc::F_GETFL) };
        let mut owned = Vec::new();
        for entry in std::fs::read_dir("/proc/self/fd")? {
            let entry = entry?;
            let Some(fd) = entry
                .file_name()
                .to_str()
                .and_then(|name| name.parse::<i32>().ok())
            else {
                continue;
            };
            if fd > 2 && std::fs::read_link(entry.path()).ok().as_ref() == Some(&tty) {
                // SAFETY: fcntl(F_GETFL) does not mutate or take ownership of the descriptor.
                let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
                if flags >= 0 {
                    owned.push(flags);
                }
            }
        }
        std::fs::write(
            path,
            json!({"stdin_flags":stdin_flags, "owned_tty_flags":owned}).to_string(),
        )?;
    }
    println!("READY");
    io::stdout().flush()?;
    loop {
        let event = tokio::select! {
            event = events.next() => match event { Some(value) => value?, None => break },
            _ = terminate.recv() => break,
            _ = interrupt.recv() => break,
        };
        let stop = matches!(&event, Event::Key(key) if key.code == KeyCode::Char('q') && key.modifiers == KeyModifiers::CONTROL);
        let value = match event {
            Event::TerminalResponse(TerminalResponse::PrimaryDeviceAttributes(values)) => {
                json!({"kind":"report", "type":"da", "values":values})
            }
            Event::TerminalResponse(TerminalResponse::CellSizePixels { width, height }) => {
                json!({"kind":"report", "type":"cell", "width":width, "height":height})
            }
            Event::TerminalResponse(TerminalResponse::TextAreaPixels { width, height }) => {
                json!({"kind":"report", "type":"pixels", "width":width, "height":height})
            }
            Event::TerminalResponse(TerminalResponse::TextAreaCells { columns, rows }) => {
                json!({"kind":"report", "type":"cells", "columns":columns, "rows":rows})
            }
            Event::Key(key) => {
                json!({"kind":"key", "code":format!("{:?}",key.code), "mods":key.modifiers.bits()})
            }
            Event::Paste(text) => json!({"kind":"paste", "text":text}),
            Event::Mouse(mouse) => {
                json!({"kind":"mouse", "event":format!("{:?}",mouse.kind), "column":mouse.column, "row":mouse.row})
            }
            Event::Resize(columns, rows) => {
                json!({"kind":"resize", "columns":columns, "rows":rows})
            }
            event => json!({"kind":"other", "event":format!("{event:?}")}),
        };
        println!("{value}");
        io::stdout().flush()?;
        if stop {
            break;
        }
    }
    Ok(())
}

#[cfg(not(unix))]
fn main() {
    eprintln!("This terminal regression fixture requires Unix.");
    std::process::exit(1);
}
