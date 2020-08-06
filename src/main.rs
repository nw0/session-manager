//! Session manager
//!
//! A would-be terminal multiplexer.

use std::{
    fs::File,
    io::{Read, Write},
    thread,
};

use anyhow::Result;
use futures::{
    channel::mpsc::{self, Receiver},
    executor,
    stream::StreamExt,
};
use log::LevelFilter;
use log4rs::{
    append::file::FileAppender,
    config::{Appender, Config, Root},
};
use nix::pty::Winsize;
use signal_hook::{iterator::Signals, SIGWINCH};
use termion::{
    self,
    event::Event,
    input::{EventsAndRaw, TermReadEventsAndRaw},
    raw::IntoRawMode,
};
use vte::ansi::Processor;

mod console;
mod grid;
mod window;

use console::Console;
use window::Window;

fn main() -> Result<()> {
    // TODO: turn into lib/bin crate
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

    let child = Window::new(&get_shell(), get_term_size()?).unwrap();
    let mut pty_output = child.get_file().try_clone()?.bytes();
    let mut pty_input = child.get_file().try_clone()?;
    let mut pty_for_stdin = child.get_file().try_clone()?;
    let mut parser = Processor::new();
    let Console {
        child_pty,
        mut grid,
    } = child.console;

    thread::spawn(move || {
        // loop to take care of pty
        while let Some(Ok(byte)) = pty_output.next() {
            // give Grid `pty_input` in case it needs to reply to the pty
            parser.advance(&mut grid, byte, &mut pty_input);
            grid.draw(&mut tty_output);
        }
    });

    let child_pty = child_pty;

    executor::block_on(event_loop(&mut input_stream, &mut pty_for_stdin));

    thread::spawn(move || -> Result<()> {
        Ok(for _ in signal.forever() {
            child_pty.resize(get_term_size()?).unwrap();
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
) {
    while let Some((_, data)) = input_events.next().await {
        pty_output.write(&data).unwrap();
        pty_output.flush().unwrap();
    }
}

pub fn get_term_size() -> Result<Winsize> {
    let (cols, rows) = termion::terminal_size()?;
    Ok(Winsize {
        ws_row: rows,
        ws_col: cols,
        ws_xpixel: 0,
        ws_ypixel: 0,
    })
}

/// Return the path to the shell executable.
pub fn get_shell() -> String {
    // TODO: something reasonable
    "/bin/sh".to_string()
}
