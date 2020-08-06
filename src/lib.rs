//! Abstractions used by the session manager.

pub mod console;
pub mod grid;
pub mod window;

pub mod util {
    use std::io;

    use nix::pty::Winsize;
    use termion;

    pub fn get_term_size() -> io::Result<Winsize> {
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
}
