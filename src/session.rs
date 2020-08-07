//! Structures and functions to manage windows.

use std::{
    fs::File,
    io::{self, Write},
};

use anyhow::Result;
use futures::{
    channel::mpsc::Receiver,
    stream::{Stream, StreamExt},
};
use nix::pty::Winsize;
use termion::raw::RawTerminal;
use vte::ansi::Processor;

use crate::{
    console::{self, ChildPty, PtyUpdate},
    grid::Grid,
    util,
};

/// A collection of `Window`s.
pub struct Session {
    next_window: usize,
    selected_window: Option<usize>,
    windows: Vec<Window>,
    processors: Vec<Processor>,
}

impl Session {
    /// Construct a new `Session`.
    pub fn new() -> Session {
        Session {
            next_window: 0,
            selected_window: None,
            windows: Vec::new(),
            processors: Vec::new(),
        }
    }

    /// Initialise a new window within this `Session`.
    pub fn new_window(&mut self) -> Result<impl Stream<Item = SessionPtyUpdate>> {
        let (child, update) =
            Window::new(&util::get_shell(), util::get_term_size()?).unwrap();
        self.windows.push(child);
        self.processors.push(Processor::new());
        let window_idx = self.next_window;
        self.next_window += 1;
        Ok(update.map(move |data| SessionPtyUpdate { window_idx, data }))
    }

    fn selected_window(&self) -> Option<&Window> {
        self.windows.get(self.selected_window.unwrap())
    }

    pub fn select_window(&mut self, idx: usize) -> Option<usize> {
        match self.windows.get(idx) {
            Some(_) => {
                self.selected_window = Some(idx);
                Some(idx)
            }
            None => None,
        }
    }

    /// Receive stdin for the active `Window`.
    pub fn receive_stdin(&self, data: &[u8]) -> Result<(), io::Error> {
        let mut file = self.selected_window().unwrap().get_file();
        file.write(data)?;
        file.flush()?;
        Ok(())
    }

    /// Draw the selected `Window` to the given terminal.
    pub fn redraw(&self, tty_output: &mut RawTerminal<File>) {
        self.selected_window().unwrap().grid.draw(tty_output);
    }

    /// Update grid with PTY output.
    pub fn pty_update(&mut self, update: SessionPtyUpdate) {
        match update.data {
            PtyUpdate::Exited => (),
            PtyUpdate::Byte(byte) => {
                let window = self.windows.get_mut(update.window_idx).unwrap();
                let mut reply = window.get_file().try_clone().unwrap();
                self.processors.get_mut(update.window_idx).unwrap().advance(
                    &mut window.grid,
                    byte,
                    &mut reply,
                );
            }
        }
    }

    pub fn resize_pty(&self, idx: usize) {
        let sz = util::get_term_size().unwrap();
        self.windows.get(idx).unwrap().pty.resize(sz).unwrap();
    }
}

/// Session-specific PTY update tuple.
pub struct SessionPtyUpdate {
    window_idx: usize,
    data: PtyUpdate,
}

/// Window: a `Console` abstraction.
///
/// This structure exists so that `Console` can be only concerned with the
/// underlying terminal implementation and frame, whereas `Window` acts as the
/// interface between the multiplexer and the `Console`.
pub struct Window {
    pub pty: ChildPty,
    pub grid: Grid<File>,
}

impl Window {
    pub fn new(
        command: &str,
        size: Winsize,
    ) -> Result<(Window, Receiver<PtyUpdate>), ()> {
        let (pty, grid, pty_update) = console::spawn_pty(command, size)?;
        Ok((Window { pty, grid }, pty_update))
    }

    pub fn get_file(&self) -> &File {
        &self.pty.file
    }
}
