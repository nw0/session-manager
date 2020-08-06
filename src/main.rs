//! Session manager
//!
//! A would-be terminal multiplexer.

use std::{fs::File, thread};

use anyhow::Result;
use futures::{
    channel::mpsc::{self, Receiver},
    executor,
    stream::StreamExt,
};
use log::{info, LevelFilter};
use log4rs::{
    append::file::FileAppender,
    config::{Appender, Config, Root},
};
use signal_hook::{iterator::Signals, SIGWINCH};
use termion::{
    self,
    event::Event,
    input::{EventsAndRaw, TermReadEventsAndRaw},
    raw::{IntoRawMode, RawTerminal},
};

use session_manager::session::Session;

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

    let mut tty_output = termion::get_tty()?.into_raw_mode()?;
    let input_events = tty_output.try_clone()?.events_and_raw();
    let mut input_stream = input_to_stream(input_events);

    let session = Session::new();

    executor::block_on(event_loop(&mut input_stream, &mut tty_output, session));

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

async fn event_loop(
    input_events: &mut Receiver<(Event, Vec<u8>)>,
    tty_output: &mut RawTerminal<File>,
    mut session: Session,
) {
    let mut pty_update = session.new_window().unwrap();
    let pty_for_stdin = session
        .get_window(0)
        .unwrap()
        .get_file()
        .try_clone()
        .unwrap();
    let mut pty_input = pty_for_stdin.try_clone().unwrap();
    let mut sigwinch_stream = sigwinch_stream();
    loop {
        futures::select! {
            input = input_events.next() => {
                match input {
                    Some((_, data)) => {
                        session.stdin_to_window(0, &data).unwrap();
                    },
                    None => unreachable!(),
                }
            }
            byte = pty_update.next() => {
                if byte.is_none() {
                    info!("pty exit");
                    return;
                }
                session.pty_to_grid(0, byte.unwrap(), &mut pty_input, tty_output);
            }
            _ = sigwinch_stream.next() => {
                session.resize_pty(0);
            }
        }
    }
}
