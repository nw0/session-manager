//! Session manager
//!
//! A would-be terminal multiplexer.

use std::{fs::File, thread};

use anyhow::Result;
use futures::{
    channel::mpsc::{self, Receiver},
    executor,
};
use log::LevelFilter;
use log4rs::{
    append::file::FileAppender,
    config::{Appender, Config, Root},
};
use signal_hook::{iterator::Signals, SIGWINCH};
use termion::{
    self,
    event::Event,
    input::{EventsAndRaw, TermReadEventsAndRaw},
    raw::IntoRawMode,
};

use session_manager::{
    // event::EventLoop,
    session::{Window},
    util,
};

fn main() -> Result<()> {
    let logfile = FileAppender::builder()
        // Pattern: https://docs.rs/log4rs/*/log4rs/encode/pattern/index.html
        .build("log")
        .unwrap();
    let config = Config::builder()
        .appender(Appender::builder().build("logfile", Box::new(logfile)))
        .build(
            Root::builder()
                .appender("logfile")
                .build(LevelFilter::Trace),
        )
        .unwrap();
    let _handle = log4rs::init_config(config)?;

    let tty_output = termion::get_tty()?.into_raw_mode()?;
    let input_events = tty_output.try_clone()?.events_and_raw();
    let input_stream = input_to_stream(input_events);
    let session = Session::<Window>::new(util::get_term_size().unwrap());

    let mut event_loop =
        EventLoop::new(input_stream, sigwinch_stream(), tty_output, session);
    executor::block_on(event_loop.run());

    Ok(())
}

fn sigwinch_stream() -> Receiver<bool> {
    let (mut send, recv) = mpsc::channel(0x1000);
    let signal = Signals::new(&[SIGWINCH]).unwrap();
    thread::spawn(move || {
        for _ in signal.forever() {
            send.try_send(true).unwrap();
        }
        send.disconnect();
    });
    recv
}

fn input_to_stream(mut input_events: EventsAndRaw<File>) -> Receiver<(Event, Vec<u8>)> {
    let (mut send, recv) = mpsc::channel(0x1000);
    thread::spawn(move || {
        while let Some(Ok((e, d))) = input_events.next() {
            send.try_send((e, d)).unwrap();
        }
        send.disconnect();
    });
    recv
}
