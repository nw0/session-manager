//! Abstractions used by the session manager.

#![recursion_limit = "1024"]
#[warn(missing_docs)]
pub mod console;
pub mod grid;
pub mod session;

pub mod util {
    use std::io;

    use nix::pty::Winsize;

    #[cfg(not(test))]
    pub fn get_term_size() -> io::Result<Winsize> {
        let (cols, rows) = termion::terminal_size()?;
        Ok(Winsize {
            ws_row: rows,
            ws_col: cols,
            ws_xpixel: 0,
            ws_ypixel: 0,
        })
    }

    #[cfg(test)]
    pub fn get_term_size() -> io::Result<Winsize> {
        Ok(crate::tests::WINSZ)
    }

    /// Return the path to the shell executable.
    pub fn get_shell() -> String {
        // TODO: something reasonable
        "/bin/sh".to_string()
    }
}

#[cfg(test)]
mod tests {
    use nix::pty::Winsize;

    pub const WINSZ: Winsize = Winsize {
        ws_row: 24,
        ws_col: 80,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
}
