use crate::image_view::ImageMode;
use crossterm::{
    cursor::{Hide, Show},
    event::{DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture},
    execute,
    terminal::{
        disable_raw_mode, enable_raw_mode, Clear, ClearType, EnterAlternateScreen,
        LeaveAlternateScreen,
    },
};
use ratatui::{backend::CrosstermBackend, Terminal};
use std::{
    fs::OpenOptions,
    io::{self, Stdout},
    os::fd::AsRawFd,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Once,
    },
};

static ACTIVE: AtomicBool = AtomicBool::new(false);
static NATIVE_IMAGES: AtomicBool = AtomicBool::new(false);
static PANIC_HOOK: Once = Once::new();

pub(crate) struct Tty {
    pub(crate) terminal: Terminal<CrosstermBackend<Stdout>>,
}

pub(crate) struct StartupSignals {
    armed: Arc<AtomicBool>,
    task: tokio::task::JoinHandle<()>,
}

impl StartupSignals {
    pub(crate) fn install() -> io::Result<Self> {
        let tty = OpenOptions::new().read(true).write(true).open("/dev/tty")?;
        let mut original = std::mem::MaybeUninit::<libc::termios>::uninit();
        // SAFETY: tty owns a live descriptor and original has space for termios.
        if unsafe { libc::tcgetattr(tty.as_raw_fd(), original.as_mut_ptr()) } != 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: successful tcgetattr initialized the complete termios value.
        let original = unsafe { original.assume_init() };
        let mut terminate =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
        let mut interrupt =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())?;
        let armed = Arc::new(AtomicBool::new(true));
        let active = armed.clone();
        let task = tokio::spawn(async move {
            let signal = tokio::select! {
                _ = terminate.recv() => libc::SIGTERM,
                _ = interrupt.recv() => libc::SIGINT,
            };
            if active.swap(false, Ordering::SeqCst) {
                // Startup can fail before the UI installs its graceful signal handlers.
                // SAFETY: the captured file owns the descriptor and snapshot.
                unsafe {
                    libc::tcsetattr(tty.as_raw_fd(), libc::TCSANOW, &original);
                }
                std::process::exit(128 + signal);
            }
        });
        Ok(Self { armed, task })
    }

    pub(crate) fn disarm(self) {
        self.armed.store(false, Ordering::SeqCst);
        self.task.abort();
    }
}

impl Drop for StartupSignals {
    fn drop(&mut self) {
        self.armed.store(false, Ordering::SeqCst);
        self.task.abort();
    }
}

fn restore() {
    if ACTIVE.swap(false, Ordering::SeqCst) {
        if NATIVE_IMAGES.swap(false, Ordering::SeqCst) {
            let _ = execute!(io::stdout(), Clear(ClearType::All));
        }
        let _ = execute!(
            io::stdout(),
            DisableBracketedPaste,
            DisableMouseCapture,
            Show,
            LeaveAlternateScreen
        );
        let _ = disable_raw_mode();
    }
}

impl Tty {
    pub(crate) fn enter_with_mode(mode: ImageMode) -> io::Result<Self> {
        NATIVE_IMAGES.store(matches!(mode, ImageMode::Sixel { .. }), Ordering::SeqCst);
        PANIC_HOOK.call_once(|| {
            let previous = std::panic::take_hook();
            std::panic::set_hook(Box::new(move |panic| {
                restore();
                previous(panic);
            }));
        });
        enable_raw_mode()?;
        ACTIVE.store(true, Ordering::SeqCst);
        if let Err(error) = execute!(
            io::stdout(),
            EnterAlternateScreen,
            Hide,
            EnableBracketedPaste,
            EnableMouseCapture
        ) {
            restore();
            return Err(error);
        }
        match Terminal::new(CrosstermBackend::new(io::stdout())) {
            Ok(terminal) => Ok(Self { terminal }),
            Err(error) => {
                restore();
                Err(error)
            }
        }
    }
}

impl Drop for Tty {
    fn drop(&mut self) {
        restore();
    }
}
