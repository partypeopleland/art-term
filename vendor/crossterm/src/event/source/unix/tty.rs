#[cfg(feature = "libc")]
use std::os::unix::prelude::AsRawFd;
use std::{
    collections::VecDeque,
    io,
    os::unix::net::UnixStream,
    time::{Duration, Instant},
};

#[cfg(not(feature = "libc"))]
use rustix::fd::{AsFd, AsRawFd};

use signal_hook::low_level::pipe;

use crate::event::timeout::PollTimeout;
use crate::event::Event;
use filedescriptor::{poll, pollfd, POLLERR, POLLHUP, POLLIN};

#[cfg(feature = "event-stream")]
use crate::event::sys::Waker;
use crate::event::{
    source::EventSource,
    sys::unix::parse::{parse_event, terminal_report_metadata_overflow},
    InternalEvent,
};
use crate::terminal::sys::file_descriptor::{event_tty_fd, FileDesc};

/// Holds a prototypical Waker and a receiver we can wait on when doing select().
#[cfg(feature = "event-stream")]
struct WakePipe {
    receiver: UnixStream,
    waker: Waker,
}

#[cfg(feature = "event-stream")]
impl WakePipe {
    fn new() -> io::Result<Self> {
        let (receiver, sender) = nonblocking_unix_pair()?;
        Ok(WakePipe {
            receiver,
            waker: Waker::new(sender),
        })
    }
}

// I (@zrzka) wasn't able to read more than 1_022 bytes when testing
// reading on macOS/Linux -> we don't need bigger buffer and 1k of bytes
// is enough.
const TTY_BUFFER_SIZE: usize = 1_024;

pub(crate) struct UnixInternalEventSource {
    parser: Parser,
    tty_buffer: [u8; TTY_BUFFER_SIZE],
    tty: FileDesc<'static>,
    winch_signal_receiver: UnixStream,
    #[cfg(feature = "event-stream")]
    wake_pipe: WakePipe,
}

fn nonblocking_unix_pair() -> io::Result<(UnixStream, UnixStream)> {
    let (receiver, sender) = UnixStream::pair()?;
    receiver.set_nonblocking(true)?;
    sender.set_nonblocking(true)?;
    Ok((receiver, sender))
}

impl UnixInternalEventSource {
    pub fn new() -> io::Result<Self> {
        UnixInternalEventSource::from_file_descriptor(event_tty_fd()?)
    }

    pub(crate) fn from_file_descriptor(input_fd: FileDesc<'static>) -> io::Result<Self> {
        Ok(UnixInternalEventSource {
            parser: Parser::default(),
            tty_buffer: [0u8; TTY_BUFFER_SIZE],
            tty: input_fd,
            winch_signal_receiver: {
                let (receiver, sender) = nonblocking_unix_pair()?;
                // Unregistering is unnecessary because EventSource is a singleton
                #[cfg(feature = "libc")]
                pipe::register(libc::SIGWINCH, sender)?;
                #[cfg(not(feature = "libc"))]
                pipe::register(rustix::process::Signal::WINCH.as_raw(), sender)?;
                receiver
            },
            #[cfg(feature = "event-stream")]
            wake_pipe: WakePipe::new()?,
        })
    }
}

/// read_complete reads from a non-blocking file descriptor
/// until the buffer is full or it would block.
///
/// Similar to `std::io::Read::read_to_end`, except this function
/// only fills the given buffer and does not read beyond that.
fn read_complete(fd: &FileDesc, buf: &mut [u8]) -> io::Result<usize> {
    loop {
        match fd.read(buf) {
            Ok(x) => return Ok(x),
            Err(e) => match e.kind() {
                io::ErrorKind::WouldBlock => return Ok(0),
                io::ErrorKind::Interrupted => continue,
                _ => return Err(e),
            },
        }
    }
}

impl EventSource for UnixInternalEventSource {
    fn try_read(&mut self, timeout: Option<Duration>) -> io::Result<Option<InternalEvent>> {
        let timeout = PollTimeout::new(timeout);

        fn make_pollfd<F: AsRawFd>(fd: &F) -> pollfd {
            pollfd {
                fd: fd.as_raw_fd(),
                events: POLLIN,
                revents: 0,
            }
        }

        #[cfg(not(feature = "event-stream"))]
        let mut fds = [
            make_pollfd(&self.tty),
            make_pollfd(&self.winch_signal_receiver),
        ];

        #[cfg(feature = "event-stream")]
        let mut fds = [
            make_pollfd(&self.tty),
            make_pollfd(&self.winch_signal_receiver),
            make_pollfd(&self.wake_pipe.receiver),
        ];

        loop {
            // check if there are buffered events from the last read
            if let Some(event) = self.parser.next() {
                return Ok(Some(event));
            }
            match poll(&mut fds, self.parser.read_timeout(timeout.leftover())) {
                Err(filedescriptor::Error::Poll(e)) | Err(filedescriptor::Error::Io(e)) => {
                    match e.kind() {
                        // retry on EINTR
                        io::ErrorKind::Interrupted => continue,
                        _ => return Err(e),
                    }
                }
                Err(e) => {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::Other,
                        format!("got unexpected error while polling: {:?}", e),
                    ))
                }
                Ok(_) => (),
            };
            if fds[0].revents & POLLIN != 0 {
                loop {
                    match self.tty.read(&mut self.tty_buffer) {
                        Ok(0) => {
                            return Err(io::Error::new(
                                io::ErrorKind::UnexpectedEof,
                                "Terminal input closed",
                            ))
                        }
                        Ok(read_count) => self.parser.advance(
                            &self.tty_buffer[..read_count],
                            read_count == TTY_BUFFER_SIZE,
                        ),
                        Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                        Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                        Err(error) => return Err(error),
                    }
                    if let Some(event) = self.parser.next() {
                        return Ok(Some(event));
                    }
                }
            }
            if fds[0].revents & (POLLHUP | POLLERR) != 0 {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "Terminal input closed",
                ));
            }
            if fds[1].revents & POLLIN != 0 {
                #[cfg(feature = "libc")]
                let fd = FileDesc::new(self.winch_signal_receiver.as_raw_fd(), false);
                #[cfg(not(feature = "libc"))]
                let fd = FileDesc::Borrowed(self.winch_signal_receiver.as_fd());
                // drain the pipe
                while read_complete(&fd, &mut [0; 1024])? != 0 {}
                // TODO Should we remove tput?
                //
                // This can take a really long time, because terminal::size can
                // launch new process (tput) and then it parses its output. It's
                // not a really long time from the absolute time point of view, but
                // it's a really long time from the mio, async-std/tokio executor, ...
                // point of view.
                let new_size = crate::terminal::size()?;
                return Ok(Some(InternalEvent::Event(Event::Resize(
                    new_size.0, new_size.1,
                ))));
            }

            #[cfg(feature = "event-stream")]
            if fds[2].revents & POLLIN != 0 {
                #[cfg(feature = "libc")]
                let fd = FileDesc::new(self.wake_pipe.receiver.as_raw_fd(), false);
                #[cfg(not(feature = "libc"))]
                let fd = FileDesc::Borrowed(self.wake_pipe.receiver.as_fd());
                // drain the pipe
                while read_complete(&fd, &mut [0; 1024])? != 0 {}

                return Err(std::io::Error::new(
                    std::io::ErrorKind::Interrupted,
                    "Poll operation was woken up by `Waker::wake`",
                ));
            }
            if timeout.elapsed() {
                return Ok(None);
            }
        }
    }

    #[cfg(feature = "event-stream")]
    fn waker(&self) -> Waker {
        self.wake_pipe.waker.clone()
    }
}

//
// Following `Parser` structure exists for two reasons:
//
//  * mimic anes Parser interface
//  * move the advancing, parsing, ... stuff out of the `try_read` method
//
#[derive(Debug)]
struct Parser {
    buffer: Vec<u8>,
    discarding_report: bool,
    escape_started: Option<Instant>,
    internal_events: VecDeque<InternalEvent>,
}

impl Default for Parser {
    fn default() -> Self {
        Parser {
            // This buffer is used for -> 1 <- ANSI escape sequence. Are we
            // aware of any ANSI escape sequence that is bigger? Can we make
            // it smaller?
            //
            // Probably not worth spending more time on this as "there's a plan"
            // to use the anes crate parser.
            buffer: Vec::with_capacity(256),
            discarding_report: false,
            escape_started: None,
            // TTY_BUFFER_SIZE is 1_024 bytes. How many ANSI escape sequences can
            // fit? What is an average sequence length? Let's guess here
            // and say that the average ANSI escape sequence length is 8 bytes. Thus
            // the buffer size should be 1024/8=128 to avoid additional allocations
            // when processing large amounts of data.
            //
            // There's no need to make it bigger, because when you look at the `try_read`
            // method implementation, all events are consumed before the next TTY_BUFFER
            // is processed -> events pushed.
            internal_events: VecDeque::with_capacity(128),
        }
    }
}

impl Parser {
    fn read_timeout(&self, timeout: Option<Duration>) -> Option<Duration> {
        match self.escape_started {
            Some(started) => {
                let remaining = Duration::from_millis(50).saturating_sub(started.elapsed());
                Some(timeout.map_or(remaining, |timeout| timeout.min(remaining)))
            }
            None => timeout,
        }
    }

    fn advance(&mut self, buffer: &[u8], more: bool) {
        for (idx, byte) in buffer.iter().enumerate() {
            let more = idx + 1 < buffer.len() || more;

            if self.discarding_report {
                if (64..=126).contains(byte) {
                    self.discarding_report = false;
                }
                continue;
            }
            self.buffer.push(*byte);
            if self.buffer == [b'\x1b'] {
                self.escape_started = Some(Instant::now());
                continue;
            }
            self.escape_started = None;
            if terminal_report_metadata_overflow(&self.buffer) {
                self.buffer.clear();
                self.discarding_report = !(64..=126).contains(byte);
                continue;
            }

            match parse_event(&self.buffer, more) {
                Ok(Some(ie)) => {
                    self.internal_events.push_back(ie);
                    self.buffer.clear();
                }
                Ok(None) => {
                    // Event can't be parsed, because we don't have enough bytes for
                    // the current sequence. Keep the buffer and process next bytes.
                }
                Err(_) => {
                    // Event can't be parsed (not enough parameters, parameter is not a number, ...).
                    // Clear the buffer and continue with another sequence.
                    self.buffer.clear();
                }
            }
        }
    }
}

impl Iterator for Parser {
    type Item = InternalEvent;

    fn next(&mut self) -> Option<Self::Item> {
        // A lone ESC is also the prefix of a fragmented terminal reply.
        if self
            .escape_started
            .is_some_and(|started| started.elapsed() >= Duration::from_millis(50))
        {
            self.escape_started = None;
            self.buffer.clear();
            self.internal_events
                .push_back(InternalEvent::Event(Event::Key(
                    crate::event::KeyCode::Esc.into(),
                )));
        }
        self.internal_events.pop_front()
    }
}

#[cfg(test)]
mod terminal_report_bound_tests {
    use super::Parser;
    use crate::event::{Event, InternalEvent, KeyCode};

    #[test]
    fn oversized_report_is_bounded_and_its_tail_cannot_become_keys() {
        for prefix in [b"\x1b[?".as_slice(), b"\x1b[6;"] {
            let mut parser = Parser::default();
            parser.advance(prefix, true);
            for _ in 0..1024 {
                parser.advance(b"1;", true);
                assert!(parser.buffer.len() <= 256);
            }
            parser.advance(b"cZ", false);
            let events: Vec<_> = parser.collect();
            assert_eq!(events.len(), 1);
            assert!(
                matches!(&events[0], InternalEvent::Event(Event::Key(key)) if key.code == KeyCode::Char('Z'))
            );
        }
    }

    #[test]
    fn escape_is_bounded_and_a_fragmented_reply_keeps_its_prefix() {
        let mut parser = Parser::default();
        parser.advance(b"\x1b", false);
        assert!(parser.next().is_none());
        parser.advance(b"[?61;4c", false);
        assert!(
            matches!(parser.next(), Some(InternalEvent::PrimaryDeviceAttributes(values)) if values == vec![61, 4])
        );
        parser.advance(b"\x1b", false);
        parser.escape_started =
            Some(std::time::Instant::now() - std::time::Duration::from_millis(51));
        assert!(
            matches!(parser.next(), Some(InternalEvent::Event(Event::Key(key))) if key.code == KeyCode::Esc)
        );
    }

    #[test]
    fn malformed_private_report_recovers_before_the_following_key() {
        let mut parser = Parser::default();
        parser.advance(b"\x1b[?1xZ", false);
        let events: Vec<_> = parser.collect();
        assert_eq!(events.len(), 1);
        assert!(
            matches!(&events[0], InternalEvent::Event(Event::Key(key)) if key.code == KeyCode::Char('Z'))
        );
    }
}
