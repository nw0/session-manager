use std::{io::Write, marker::Unpin, time::Duration};

use futures::{
    future::FutureExt,
    stream::{FusedStream, SelectAll, StreamExt},
};
use log::info;
use termion::{
    self, clear,
    cursor::Goto,
    event::{Event, Key},
};

use crate::session::{Session, SessionError, SessionWindow};

const PREFIX: Event = Event::Key(Key::Ctrl('b'));

pub struct EventLoop<P, SI, SR, W>
where
    P: SessionWindow,
    SI: FusedStream<Item = (Event, Vec<u8>)> + Unpin,
    SR: FusedStream<Item = bool> + Unpin,
    W: Write,
{
    input: SI,
    resize: SR,
    output: W,
    session: Session<P>,
}

impl<P, SI, SR, W> EventLoop<P, SI, SR, W>
where
    P: SessionWindow,
    SI: FusedStream<Item = (Event, Vec<u8>)> + Unpin,
    SR: FusedStream<Item = bool> + Unpin,
    W: Write,
{
    pub fn new(
        input: SI,
        resize: SR,
        output: W,
        session: Session<P>,
    ) -> EventLoop<P, SI, SR, W> {
        EventLoop {
            input,
            resize,
            output,
            session,
        }
    }

    pub async fn run(&mut self) {
        let mut ptys_update = SelectAll::new();
        let (idx, window) = self.session.new_window().unwrap();
        ptys_update.push(window);
        self.session.select_window(idx);
        let mut manage_mode = false;
        let mut redraw_timer = SelectAll::new();
        redraw_timer
            .push(futures_timer::Delay::new(Duration::from_millis(5)).into_stream());
        let mut dirty = true;

        loop {
            futures::select! {
                input = self.input.next() => {
                    if manage_mode {
                        match input {
                            Some((PREFIX, data)) => {
                                self.session.receive_stdin(&data).unwrap();
                            },
                            Some((Event::Key(Key::Char('c')), _)) => {
                                let (idx, window) = self.session.new_window().unwrap();
                                ptys_update.push(window);
                                self.session.select_window(idx);
                            },
                            Some((Event::Key(Key::Char('n')), _)) => {
                                self.session.next_window_idx()
                                    .or(self.session.first_window_idx())
                                    .map(|idx| self.session.select_window(idx));
                            },
                            Some((Event::Key(Key::Char('p')), _)) => {
                                self.session.prev_window_idx()
                                    .or(self.session.last_window_idx())
                                    .map(|idx| self.session.select_window(idx));
                            },
                            None => unreachable!(),
                            _ => info!("unhandled event: {:?}", input),
                        }
                        manage_mode = false;
                        dirty = true;
                    }
                    else {
                        match input {
                            Some((PREFIX, _)) => {
                                manage_mode = true;
                            }
                            Some((event, data)) => {
                                    self.session.receive_stdin(&data).unwrap();
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
                    self.session.pty_update(pty_update.unwrap()).unwrap();
                    dirty = true;
                }
                _ = self.resize.next() => {
                    self.session.resize(crate::util::get_term_size().unwrap()).unwrap();
                    dirty = true;
                }
                _ = redraw_timer.next() => {
                    if dirty {
                        match self.session.redraw(&mut self.output) {
                            Ok(_) => (),
                            Err(SessionError::NoSelectedWindow) => {
                                write!(self.output,
                                       "{}{}sm: last window closed. Exiting.\r\n",
                                       Goto(1, 1),
                                       clear::All
                                ).unwrap();
                            }
                            _ => panic!("unhandled redraw error")
                        }
                        self.output.flush().unwrap();
                        dirty = false;
                    }
                    redraw_timer.push(futures_timer::Delay::new(Duration::from_millis(5)).into_stream());
                }
            }
        }
    }
}
