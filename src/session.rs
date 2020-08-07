//! Structures and functions to manage windows.

use std::{
    collections::BTreeMap,
    fs::File,
    io::{self, Write},
};

use anyhow::Result;
use futures::{
    channel::mpsc::Receiver,
    stream::{Stream, StreamExt},
};
use log::debug;
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
    windows: BTreeMap<usize, Window>,
    processors: Vec<Processor>,
}

impl Session {
    /// Construct a new `Session`.
    pub fn new() -> Session {
        Session {
            next_window: 0,
            selected_window: None,
            windows: BTreeMap::new(),
            processors: Vec::new(),
        }
    }

    /// Initialise a new window within this `Session`.
    pub fn new_window(
        &mut self,
    ) -> Result<(usize, impl Stream<Item = SessionPtyUpdate>)> {
        let window_idx = self.next_window;
        let (child, update) =
            Window::new(&util::get_shell(), util::get_term_size()?).unwrap();
        self.windows.insert(window_idx, child);
        self.processors.push(Processor::new());
        self.next_window += 1;
        Ok((
            window_idx,
            update.map(move |data| SessionPtyUpdate { window_idx, data }),
        ))
    }

    fn selected_window(&self) -> Option<&Window> {
        self.windows.get(&self.selected_window.unwrap())
    }

    fn selected_window_mut(&mut self) -> Option<&mut Window> {
        self.windows.get_mut(&self.selected_window.unwrap())
    }

    pub fn select_window(&mut self, idx: usize) -> Option<usize> {
        match self.windows.get(&idx) {
            Some(_) => {
                self.selected_window = Some(idx);
                self.selected_window_mut().unwrap().grid.mark_all_dirty();
                Some(idx)
            }
            None => None,
        }
    }

    /// Get index of next oldest window.
    pub fn next_window_idx(&self) -> Option<usize> {
        self.windows
            .range((self.selected_window? + 1)..)
            .next()
            .map(|(idx, _)| *idx)
    }

    /// Get index of next older window.
    pub fn prev_window_idx(&self) -> Option<usize> {
        self.windows
            .range(..self.selected_window?)
            .next_back()
            .map(|(idx, _)| *idx)
    }

    /// Get index of oldest window.
    pub fn first_window_idx(&self) -> Option<usize> {
        self.windows.keys().next().map(|idx| *idx)
    }

    /// Get index of youngest window.
    pub fn last_window_idx(&self) -> Option<usize> {
        self.windows.keys().rev().next().map(|idx| *idx)
    }

    /// Receive stdin for the active `Window`.
    pub fn receive_stdin(&self, data: &[u8]) -> Result<(), io::Error> {
        let mut file = self.selected_window().unwrap().get_file();
        file.write(data)?;
        file.flush()?;
        Ok(())
    }

    /// Draw the selected `Window` to the given terminal.
    pub fn redraw(&mut self, tty_output: &mut RawTerminal<File>) {
        self.selected_window_mut().unwrap().grid.draw(tty_output);
    }

    /// Update grid with PTY output.
    pub fn pty_update(&mut self, update: SessionPtyUpdate) {
        match update.data {
            PtyUpdate::Exited => {
                debug!("removed window {}", update.window_idx);
                self.windows.remove(&update.window_idx);
                self.next_window_idx()
                    .or(self.last_window_idx())
                    .map(|idx| self.select_window(idx));
            }
            PtyUpdate::Byte(byte) => {
                let window = self.windows.get_mut(&update.window_idx).unwrap();
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
        self.windows.get(&idx).unwrap().pty.resize(sz).unwrap();
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
