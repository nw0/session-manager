//! Structures and functions to manage windows.

use std::{
    collections::BTreeMap,
    fs::File,
    io::{self, Write},
    thread,
};

use anyhow::Result;
use futures::{
    channel::mpsc::{self, Receiver},
    stream::{Stream, StreamExt},
};
use log::debug;
use nix::pty::Winsize;
use thiserror::Error;
use vte::ansi::Processor;

use crate::{
    console::{self, ChildPty, PtyUpdate},
    grid::Grid,
    util,
};

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
    // fn resize(&mut self, sz: Winsize);
    // fn mark_dirty(&mut self);
    // fn redraw<T: Write>(&mut self, output: &mut T);
}

/// Window: a `Console` abstraction.
///
/// This structure exists so that `Console` can be only concerned with the
/// underlying terminal implementation and frame, whereas `Window` acts as the
/// interface between the multiplexer and the `Console`.
pub struct Window {
    pty: ChildPty,
    processor: Processor,
    size: Winsize,
}

impl SessionWindow for Window {
    fn new(command: &str, size: Winsize) -> Result<(Window, Receiver<PtyUpdate>), ()> {
        let args: [&str; 0] = [];
        let (pty, mut grid) = console::spawn_pty(command, &args, size)?;
        let mut processor = Processor::new();
        let mut pty_output = pty.file.try_clone().unwrap();
        let (mut send, pty_update) = mpsc::channel(0x100);
        thread::spawn(move || {
            use std::io::Read;
            let mut buf = [0u8; 4096];
            while let Ok(sz) = pty_output.read(&mut buf) {
                for byte in &buf[..sz] {
                    processor.advance(&mut grid, *byte, &mut pty_output);
                }
                send.try_send(PtyUpdate::Exited).unwrap();
                send.disconnect();
            }
        });
        Ok((
            Window {
                pty,
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

    // fn resize(&mut self, sz: Winsize) {
    //     if sz != self.size {
    //         self.size = sz;
    //         self.grid.resize(sz.ws_col, sz.ws_row);
    //         self.pty.resize(sz).unwrap();
    //         self.mark_dirty();
    //     }
    // }

    // fn mark_dirty(&mut self) {
    //     self.grid.mark_all_dirty();
    // }

    // fn redraw<T: Write>(&mut self, output: &mut T) {
    //     self.grid.draw(output);
    // }
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use crate::tests::WINSZ;

    use futures::channel::mpsc::{self, Sender};

    pub struct MockWindow {
        stdin_channel: (Sender<u8>, Receiver<u8>),
        pty_channel: (Sender<u8>, Receiver<u8>),
        resize_channel: (Sender<Winsize>, Receiver<Winsize>),
        dirty_channel: (Sender<bool>, Receiver<bool>),
    }

    impl SessionWindow for MockWindow {
        fn new(_: &str, _: Winsize) -> Result<(MockWindow, Receiver<PtyUpdate>), ()> {
            let (_, recv) = mpsc::channel(10);
            let stdin_channel = mpsc::channel(100);
            let pty_channel = mpsc::channel(10);
            let resize_channel = mpsc::channel(10);
            let dirty_channel = mpsc::channel(10);
            Ok((
                MockWindow {
                    stdin_channel,
                    pty_channel,
                    resize_channel,
                    dirty_channel,
                },
                recv,
            ))
        }

        fn receive_stdin(&self, data: &[u8]) -> Result<(), io::Error> {
            for byte in data {
                self.stdin_channel.0.clone().try_send(*byte).unwrap();
            }
            Ok(())
        }

        fn pty_update(&mut self, byte: u8) {
            self.pty_channel.0.try_send(byte).unwrap();
        }

        fn resize(&mut self, size: Winsize) {
            self.resize_channel.0.try_send(size).unwrap();
        }

        fn mark_dirty(&mut self) {
            self.dirty_channel.0.try_send(true).unwrap();
        }

        fn redraw<T: Write>(&mut self, file: &mut T) {
            file.write(b"hello").unwrap();
            file.flush().unwrap();
        }
    }

    #[test]
    fn session_report_unselected() {
        let mut session: Session<MockWindow> = Session::new(WINSZ);
        match session.redraw(&mut io::sink()).unwrap_err() {
            SessionError::NoSelectedWindow => (),
            _ => assert!(false, "wrong error when redrawing nonexistent window"),
        };
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
        session
            .pty_update(SessionPtyUpdate {
                window_idx: second,
                data: PtyUpdate::Exited,
            })
            .unwrap();
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
        session
            .pty_update(SessionPtyUpdate {
                window_idx: first,
                data: PtyUpdate::Exited,
            })
            .unwrap();
        assert_eq!(Some(third), session.selected_window_idx());
        assert_eq!(
            Some(third),
            session.first_window_idx(),
            "only remaining window is not first"
        );
        assert_eq!(Some(third), session.last_window_idx());
        assert_eq!(None, session.prev_window_idx());
        session
            .pty_update(SessionPtyUpdate {
                window_idx: third,
                data: PtyUpdate::Exited,
            })
            .unwrap();
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
    fn session_forward_stdin() {
        let mut session: Session<MockWindow> = Session::new(WINSZ);
        let (first, _) = session.new_window().unwrap();
        let (second, _) = session.new_window().unwrap();

        session.select_window(second);
        assert_eq!(session.selected_window_idx(), Some(second));
        session.receive_stdin(b"Hello").unwrap();

        let recv = &mut session.windows.get_mut(&first).unwrap().stdin_channel.1;
        assert!(recv.try_next().is_err(), "other window received byte");
        let recv = &mut session.windows.get_mut(&second).unwrap().stdin_channel.1;
        for byte in b"Hello" {
            assert_eq!(recv.try_next().unwrap(), Some(*byte), "failed to recv byte");
        }
        assert!(recv.try_next().is_err(), "recv too many bytes");

        session.select_window(first);
        session.receive_stdin(b"World").unwrap();

        let recv = &mut session.windows.get_mut(&first).unwrap().stdin_channel.1;
        for byte in b"World" {
            assert_eq!(recv.try_next().unwrap(), Some(*byte), "failed to recv byte");
        }
        assert!(recv.try_next().is_err(), "recv too many bytes");
        let recv = &mut session.windows.get_mut(&second).unwrap().stdin_channel.1;
        assert!(recv.try_next().is_err(), "other window received byte");
    }

    #[test]
    fn session_forward_pty_update() {
        let mut session: Session<MockWindow> = Session::new(WINSZ);
        let (first, _) = session.new_window().unwrap();
        let (second, _) = session.new_window().unwrap();
        session.select_window(second);
        assert_eq!(session.selected_window_idx(), Some(second));
        session
            .pty_update(SessionPtyUpdate {
                window_idx: first,
                data: PtyUpdate::Byte(13),
            })
            .unwrap();

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

        session.resize(WINSZ).unwrap();
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
        session
            .pty_update(SessionPtyUpdate {
                window_idx: first,
                data: PtyUpdate::Byte(13),
            })
            .unwrap();

        session.select_window(third);
        let recv = &mut session.windows.get_mut(&first).unwrap().resize_channel.1;
        assert!(recv.try_next().is_err(), "resized background window");
        let recv = &mut session.windows.get_mut(&second).unwrap().resize_channel.1;
        assert!(recv.try_next().is_err(), "resized on exit");
        let recv = &mut session.windows.get_mut(&third).unwrap().resize_channel.1;
        assert!(recv.try_next().is_ok(), "did not resize selected window");

        session
            .pty_update(SessionPtyUpdate {
                window_idx: third,
                data: PtyUpdate::Exited,
            })
            .unwrap();
        let recv = &mut session.windows.get_mut(&first).unwrap().resize_channel.1;
        assert!(recv.try_next().is_err(), "resized background window");
        let recv = &mut session.windows.get_mut(&second).unwrap().resize_channel.1;
        assert!(recv.try_next().is_ok(), "did not resize on selection");
    }

    #[test]
    fn session_mark_dirty_on_select() {
        let mut session: Session<MockWindow> = Session::new(WINSZ);
        let (first, _) = session.new_window().unwrap();
        let (second, _) = session.new_window().unwrap();

        let recv = &mut session.windows.get_mut(&first).unwrap().dirty_channel.1;
        assert!(recv.try_next().is_err(), "marked before selection");
        let recv = &mut session.windows.get_mut(&second).unwrap().dirty_channel.1;
        assert!(recv.try_next().is_err(), "marked before selection");

        session.select_window(second);
        assert_eq!(session.selected_window_idx(), Some(second));
        let recv = &mut session.windows.get_mut(&first).unwrap().dirty_channel.1;
        assert!(recv.try_next().is_err(), "unselected window marked");
        let recv = &mut session.windows.get_mut(&second).unwrap().dirty_channel.1;
        assert!(recv.try_next().is_ok(), "selected window not marked");

        session.select_window(first);
        let recv = &mut session.windows.get_mut(&first).unwrap().dirty_channel.1;
        assert!(recv.try_next().is_ok(), "selected window not marked");
        let recv = &mut session.windows.get_mut(&second).unwrap().dirty_channel.1;
        assert!(recv.try_next().is_err(), "unselected window marked");
    }
}
