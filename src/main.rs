//! Session manager
//!
//! A would-be terminal multiplexer.

use std::{fs::File, io::Write, thread};

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
use vte::ansi::Processor;

use session_manager::{
    grid::Grid,
    util::{get_shell, get_term_size},
    window::Window,
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

    let signal = Signals::new(&[SIGWINCH])?;

    let mut tty_output = termion::get_tty()?.into_raw_mode()?;
    let input_events = tty_output.try_clone()?.events_and_raw();
    let mut input_stream = input_to_stream(input_events);

    let (child, mut pty_update) = Window::new(&get_shell(), get_term_size()?).unwrap();
    let mut pty_for_stdin = child.get_file().try_clone()?;
    let Window { pty, mut grid } = child;

    executor::block_on(event_loop(
        &mut input_stream,
        &mut pty_for_stdin,
        &mut pty_update,
        &mut grid,
        &mut tty_output,
    ));

    thread::spawn(move || -> Result<()> {
        Ok(for _ in signal.forever() {
            pty.resize(get_term_size()?).unwrap();
        })
    });

    Ok(())
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
    pty_output: &mut File,
    pty_update: &mut Receiver<u8>,
    grid: &mut Grid<File>,
    tty_output: &mut RawTerminal<File>,
) {
    let mut parser = Processor::new();
    let mut pty_input = pty_output.try_clone().unwrap();
    loop {
        futures::select! {
            input = input_events.next() => {
                match input {
                    Some((_, data)) => {
                        pty_output.write(&data).unwrap();
                        pty_output.flush().unwrap();
                    },
                    None => unreachable!(),
                }
            }
            byte = pty_update.next() => {
                if byte.is_none() {
                    info!("pty exit");
                    return;
                }
                parser.advance(grid, byte.unwrap(), &mut pty_input);
                grid.draw(tty_output);
            }
        }
    }
}
