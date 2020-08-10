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
use vte::ansi::Processor;

use crate::{
    console::{self, ChildPty, PtyUpdate},
    grid::Grid,
    util,
};

/// A collection of `Window`s.
pub struct Session<W: SessionWindow> {
    next_window: usize,
    selected_window: Option<usize>,
    windows: BTreeMap<usize, W>,
    size: Winsize,
}

impl<W: SessionWindow> Session<W> {
    /// Construct a new `Session`.
    pub fn new() -> Session<W> {
        Session {
            next_window: 0,
            selected_window: None,
            windows: BTreeMap::new(),
            size: util::get_term_size().unwrap(),
        }
    }

    /// Initialise a new window within this `Session`.
    pub fn new_window(
        &mut self,
    ) -> Result<(usize, impl Stream<Item = SessionPtyUpdate>)> {
        let window_idx = self.next_window;
        let (child, update) = W::new(&util::get_shell(), self.size).unwrap();
        self.windows.insert(window_idx, child);
        self.next_window += 1;
        Ok((
            window_idx,
            update.map(move |data| SessionPtyUpdate { window_idx, data }),
        ))
    }

    fn selected_window(&self) -> Option<&W> {
        self.windows.get(&self.selected_window.unwrap())
    }

    fn selected_window_mut(&mut self) -> Option<&mut W> {
        self.windows.get_mut(&self.selected_window.unwrap())
    }

    pub fn select_window(&mut self, idx: usize) -> Option<usize> {
        match self.windows.get(&idx) {
            Some(_) => {
                self.selected_window = Some(idx);
                let sz = self.size;
                let window = self.selected_window_mut().unwrap();
                window.resize(sz);
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
        self.selected_window().unwrap().receive_stdin(data)
    }

    /// Draw the selected `Window` to the given terminal.
    pub fn redraw<T: Write>(&mut self, tty_output: &mut T) {
        self.selected_window_mut().unwrap().redraw(tty_output);
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
                window.pty_update(byte);
            }
        }
    }

    /// Resize this session.
    ///
    /// Strategy: resize the active `Window`, and resize other `Window`s when they are
    /// selected.
    pub fn resize(&mut self) {
        let sz = util::get_term_size().unwrap();
        self.size = sz;
        self.selected_window_mut().unwrap().resize(sz);
    }
}

/// Session-specific PTY update tuple.
pub struct SessionPtyUpdate {
    window_idx: usize,
    data: PtyUpdate,
}

/// A Window object for a `Session`.
///
/// This trait exists to allow `Session` to handle different types of `Window`,
/// which is useful for testing.
pub trait SessionWindow
where
    Self: Sized,
{
    fn new(command: &str, size: Winsize) -> Result<(Self, Receiver<PtyUpdate>), ()>;
    fn receive_stdin(&self, data: &[u8]) -> Result<(), io::Error>;
    fn pty_update(&mut self, byte: u8);
    fn resize(&mut self, sz: Winsize);
    fn redraw<T: Write>(&mut self, output: &mut T);
}

/// Window: a `Console` abstraction.
///
/// This structure exists so that `Console` can be only concerned with the
/// underlying terminal implementation and frame, whereas `Window` acts as the
/// interface between the multiplexer and the `Console`.
pub struct Window {
    pty: ChildPty,
    grid: Grid<File>,
    processor: Processor,
    size: Winsize,
}

impl SessionWindow for Window {
    fn new(command: &str, size: Winsize) -> Result<(Window, Receiver<PtyUpdate>), ()> {
        let args: [&str; 0] = [];
        let (pty, grid, pty_update) = console::spawn_pty(command, &args, size)?;
        Ok((
            Window {
                pty,
                grid,
                processor: Processor::new(),
                size,
            },
            pty_update,
        ))
    }

    fn receive_stdin(&self, data: &[u8]) -> Result<(), io::Error> {
        let mut file = &self.pty.file;
        file.write(data)?;
        file.flush()?;
        Ok(())
    }

    fn pty_update(&mut self, byte: u8) {
        let mut reply = self.pty.file.try_clone().unwrap();
        self.processor.advance(&mut self.grid, byte, &mut reply);
    }

    fn resize(&mut self, sz: Winsize) {
        if sz != self.size {
            self.size = sz;
            self.grid.resize(sz.ws_col, sz.ws_row);
            self.pty.resize(sz).unwrap();
            self.grid.mark_all_dirty();
        }
    }

    fn redraw<T: Write>(&mut self, output: &mut T) {
        self.grid.draw(output);
    }
}
