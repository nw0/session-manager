//! Session manager
//!
//! A would-be terminal multiplexer.
#![recursion_limit = "1024"]

use std::{fs::File, thread};

use anyhow::Result;
use futures::{
    channel::mpsc::{self, Receiver},
    executor,
    stream::{SelectAll, StreamExt},
};
use log::{info, LevelFilter};
use log4rs::{
    append::file::FileAppender,
    config::{Appender, Config, Root},
};
use signal_hook::{iterator::Signals, SIGWINCH};
use termion::{
    self,
    event::{Event, Key},
    input::{EventsAndRaw, TermReadEventsAndRaw},
    raw::{IntoRawMode, RawTerminal},
};

use session_manager::session::Session;

const PREFIX: Event = Event::Key(Key::Ctrl('b'));

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
    let mut ptys_update = SelectAll::new();
    let (idx, window) = session.new_window().unwrap();
    ptys_update.push(window);
    session.select_window(idx);
    let mut sigwinch_stream = sigwinch_stream();
    let mut manage_mode = false;

    loop {
        futures::select! {
            input = input_events.next() => {
                if manage_mode {
                    match input {
                        Some((PREFIX, data)) => {
                            session.receive_stdin(&data).unwrap();
                        },
                        Some((Event::Key(Key::Char('c')), _)) => {
                            let (idx, window) = session.new_window().unwrap();
                            ptys_update.push(window);
                            session.select_window(idx);
                        },
                        Some((Event::Key(Key::Char('n')), _)) => {
                            session.next_window_idx()
                                .or(session.first_window_idx())
                                .map(|idx| session.select_window(idx));
                        },
                        Some((Event::Key(Key::Char('p')), _)) => {
                            session.prev_window_idx()
                                .or(session.last_window_idx())
                                .map(|idx| session.select_window(idx));
                        },
                        None => unreachable!(),
                        _ => info!("unhandled event: {:?}", input),
                    }
                    manage_mode = false;
                    session.redraw(tty_output);
                }
                else {
                    match input {
                        Some((PREFIX, _)) => {
                            manage_mode = true;
                        }
                        Some((event, data)) => {
                                session.receive_stdin(&data).unwrap();
                        },
                        None => unreachable!(),
                    }
                }
            }
            pty_update = ptys_update.next() => {
                if pty_update.is_none() {
                    info!("last pty exited");
                    return;
                }
                session.pty_update(pty_update.unwrap());
                session.redraw(tty_output);
            }
            _ = sigwinch_stream.next() => {
                session.resize_pty(0);
                session.redraw(tty_output);
            }
        }
    }
}
