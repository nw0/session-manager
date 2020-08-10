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
    pub fn new(size: Winsize) -> Session<W> {
        Session {
            next_window: 0,
            selected_window: None,
            windows: BTreeMap::new(),
            size,
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

    pub fn selected_window_idx(&self) -> Option<usize> {
        self.selected_window
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
        self.windows.keys().next().copied()
    }

    /// Get index of youngest window.
    pub fn last_window_idx(&self) -> Option<usize> {
        self.windows.keys().rev().next().copied()
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
                match self.next_window_idx().or_else(|| self.last_window_idx()) {
                    Some(idx) => {
                        self.select_window(idx);
                    }
                    None => self.selected_window = None,
                }
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
    pub fn resize(&mut self, size: Winsize) {
        self.size = size;
        self.selected_window_mut().unwrap().resize(size);
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
        file.write_all(data)?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::WINSZ;

    use futures::channel::mpsc::{self, Sender};

    pub struct MockWindow {
        pty_channel: (Sender<u8>, Receiver<u8>),
        resize_channel: (Sender<Winsize>, Receiver<Winsize>),
    }

    impl SessionWindow for MockWindow {
        fn new(_: &str, _: Winsize) -> Result<(MockWindow, Receiver<PtyUpdate>), ()> {
            let (_, recv) = mpsc::channel(10);
            let pty_channel = mpsc::channel(10);
            let resize_channel = mpsc::channel(10);
            Ok((
                MockWindow {
                    pty_channel,
                    resize_channel,
                },
                recv,
            ))
        }

        fn receive_stdin(&self, _: &[u8]) -> Result<(), io::Error> {
            Ok(())
        }

        fn pty_update(&mut self, byte: u8) {
            self.pty_channel.0.try_send(byte).unwrap();
        }

        fn resize(&mut self, size: Winsize) {
            log::debug!("received resize");
            self.resize_channel.0.try_send(size).unwrap();
        }

        fn redraw<T: Write>(&mut self, _: &mut T) {}
    }

    #[test]
    fn session_window_relative() {
        let mut session: Session<MockWindow> = Session::new(WINSZ);
        assert_eq!(session.windows.len(), 0);

        let (first, _) = session.new_window().unwrap();
        assert_eq!(session.windows.len(), 1);
        session.select_window(first);
        let first = session.first_window_idx().unwrap();
        assert_eq!(
            session.select_window(2475),
            None,
            "should not select invalid idx"
        );
        assert_eq!(Some(first), session.selected_window_idx());
        assert_eq!(Some(first), session.first_window_idx());
        assert_eq!(Some(first), session.last_window_idx());
        assert_eq!(session.next_window_idx(), None);
        assert_eq!(session.prev_window_idx(), None);

        let (second, _) = session.new_window().unwrap();
        assert_eq!(session.windows.len(), 2);
        assert_ne!(first, second);
        assert_eq!(
            Some(first),
            session.selected_window_idx(),
            "selection changed when adding new window"
        );
        session.select_window(second);
        assert_eq!(Some(first), session.first_window_idx(), "ordering broken");
        assert_eq!(Some(second), session.last_window_idx(), "ordering broken");
        assert_eq!(
            Some(first),
            session.prev_window_idx(),
            "can't find first window"
        );
        assert_eq!(session.next_window_idx(), None);

        session.select_window(first);
        let (third, _) = session.new_window().unwrap();
        assert_eq!(session.windows.len(), 3);
        assert_eq!(Some(second), session.next_window_idx());
        assert_eq!(Some(third), session.last_window_idx());
        session.select_window(second);
        assert_eq!(Some(first), session.prev_window_idx());
        assert_eq!(Some(third), session.next_window_idx());
        assert_eq!(Some(first), session.first_window_idx());
        session.select_window(third);
        assert_eq!(Some(second), session.prev_window_idx());
        assert_eq!(None, session.next_window_idx());

        session.select_window(second);
        session.pty_update(SessionPtyUpdate {
            window_idx: second,
            data: PtyUpdate::Exited,
        });
        assert_eq!(
            Some(third),
            session.selected_window_idx(),
            "next younger window not selected"
        );
        assert_eq!(
            Some(first),
            session.prev_window_idx(),
            "can't find first window"
        );
        session.pty_update(SessionPtyUpdate {
            window_idx: first,
            data: PtyUpdate::Exited,
        });
        assert_eq!(Some(third), session.selected_window_idx());
        assert_eq!(
            Some(third),
            session.first_window_idx(),
            "only remaining window is not first"
        );
        assert_eq!(Some(third), session.last_window_idx());
        assert_eq!(None, session.prev_window_idx());
        session.pty_update(SessionPtyUpdate {
            window_idx: third,
            data: PtyUpdate::Exited,
        });
        assert_eq!(session.windows.len(), 0);
        assert_eq!(None, session.next_window_idx());
        assert_eq!(None, session.last_window_idx());
        assert_eq!(
            None,
            session.selected_window_idx(),
            "closed window not deselected"
        );
    }

    #[test]
    fn session_forward_pty_update() {
        let mut session: Session<MockWindow> = Session::new(WINSZ);
        let (first, _) = session.new_window().unwrap();
        let (second, _) = session.new_window().unwrap();
        session.select_window(second);
        assert_eq!(session.selected_window_idx(), Some(second));
        session.pty_update(SessionPtyUpdate {
            window_idx: first,
            data: PtyUpdate::Byte(13),
        });

        let recv = &mut session.windows.get_mut(&second).unwrap().pty_channel.1;
        assert!(recv.try_next().is_err(), "other window received byte");
        let recv = &mut session.windows.get_mut(&first).unwrap().pty_channel.1;
        assert_eq!(recv.try_next().unwrap(), Some(13u8), "failed to recv byte");
        assert!(recv.try_next().is_err(), "recv multiple bytes");
    }

    #[test]
    fn session_resize() {
        let mut session: Session<MockWindow> = Session::new(WINSZ);
        let (first, _) = session.new_window().unwrap();
        let (second, _) = session.new_window().unwrap();
        let (third, _) = session.new_window().unwrap();
        session.select_window(second);
        assert_eq!(session.selected_window_idx(), Some(second));
        let recv = &mut session.windows.get_mut(&second).unwrap().resize_channel.1;
        assert!(recv.try_next().is_ok(), "did not resize selected window");
        assert!(
            recv.try_next().is_err(),
            "resized multiple times on selection"
        );

        session.resize(WINSZ);
        let recv = &mut session.windows.get_mut(&first).unwrap().resize_channel.1;
        assert!(recv.try_next().is_err(), "resized background window");
        let recv = &mut session.windows.get_mut(&second).unwrap().resize_channel.1;
        assert!(recv.try_next().is_ok(), "did not resize selected window");
        assert!(
            recv.try_next().is_err(),
            "resized multiple times on selection"
        );
        let recv = &mut session.windows.get_mut(&third).unwrap().resize_channel.1;
        assert!(recv.try_next().is_err(), "resized background window");

        // some noise
        session.pty_update(SessionPtyUpdate {
            window_idx: first,
            data: PtyUpdate::Byte(13),
        });

        session.select_window(third);
        let recv = &mut session.windows.get_mut(&first).unwrap().resize_channel.1;
        assert!(recv.try_next().is_err(), "resized background window");
        let recv = &mut session.windows.get_mut(&second).unwrap().resize_channel.1;
        assert!(recv.try_next().is_err(), "resized on exit");
        let recv = &mut session.windows.get_mut(&third).unwrap().resize_channel.1;
        assert!(recv.try_next().is_ok(), "did not resize selected window");

        session.pty_update(SessionPtyUpdate {
            window_idx: third,
            data: PtyUpdate::Exited,
        });
        let recv = &mut session.windows.get_mut(&first).unwrap().resize_channel.1;
        assert!(recv.try_next().is_err(), "resized background window");
        let recv = &mut session.windows.get_mut(&second).unwrap().resize_channel.1;
        assert!(recv.try_next().is_ok(), "did not resize on selection");
    }
}
