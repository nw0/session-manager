//! Session manager
//!
//! A would-be terminal multiplexer.

use anyhow::Result;
use log::LevelFilter;
use log4rs::{
    append::{
        console::{ConsoleAppender, Target},
        file::FileAppender,
    },
    config::{Appender, Config, Root},
    encode::pattern::PatternEncoder,
    filter::threshold::ThresholdFilter,
};
use nix::pty::Winsize;
use signal_hook::{iterator::Signals, SIGWINCH};
use std::io::{Read, Write};
use std::thread;
use termion::get_tty;
use termion::raw::IntoRawMode;

mod console;
mod window;
use window::Window;

fn main() -> Result<()> {
    let stderr = ConsoleAppender::builder().target(Target::Stderr).build();
    let logfile = FileAppender::builder()
        // Pattern: https://docs.rs/log4rs/*/log4rs/encode/pattern/index.html
        .encoder(Box::new(PatternEncoder::new("{l} - {m}\n")))
        .build("log")
        .unwrap();
    let config = Config::builder()
        .appender(Appender::builder().build("logfile", Box::new(logfile)))
        .appender(
            Appender::builder()
                .filter(Box::new(ThresholdFilter::new(log::LevelFilter::Debug)))
                .build("stderr", Box::new(stderr)),
        )
        .build(
            Root::builder()
                .appender("logfile")
                .build(LevelFilter::Trace),
        )
        .unwrap();
    let _handle = log4rs::init_config(config)?;

    let signal = Signals::new(&[SIGWINCH])?;

    let tty_output = get_tty()?.into_raw_mode()?;
    let mut tty_input = tty_output.try_clone()?.bytes();

    let child = Window::new(&get_shell(), get_term_size()?, tty_output).unwrap();
    let mut pty_output = child.get_file().try_clone()?;

    let child_pty = child.console.child_pty;

    thread::spawn(move || -> Result<()> {
        // Handle stdin
        while let Some(Ok(byte)) = tty_input.next() {
            pty_output.write(&[byte])?;
            pty_output.flush()?;
        }
        Ok(())
    });

    thread::spawn(move || -> Result<()> {
        Ok(for _ in signal.forever() {
            child_pty.resize(get_term_size()?).unwrap();
        })
    });

    child.status.recv().unwrap();
    Ok(())
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
