//! Session manager
//!
//! A would-be terminal multiplexer.

use anyhow::Result;
use futures::stream::StreamExt;
use log::LevelFilter;
use log4rs::{
    append::file::FileAppender,
    config::{Appender, Config, Root},
};
use nix::pty::Winsize;
use signal_hook::{iterator::Signals, SIGWINCH};
use std::fs::File;
use std::io::{Read, Write};
use std::thread;
use termion::{
    get_tty,
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

    let mut tty_output = get_tty()?.into_raw_mode()?;
    let mut input_events = tty_output.try_clone()?.events_and_raw();

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

    futures::executor::block_on(handle_stdin(&mut input_events, &mut pty_for_stdin));

    thread::spawn(move || -> Result<()> {
        Ok(for _ in signal.forever() {
            child_pty.resize(get_term_size()?).unwrap();
        })
    });

    Ok(())
}

async fn handle_stdin(input_events: &mut EventsAndRaw<File>, pty_output: &mut File) {
    while let Some(Ok((_, data))) =
        futures::stream::iter(input_events.inspect(|e| log::debug!("{:?}", e)))
            .next()
            .await
    {
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
