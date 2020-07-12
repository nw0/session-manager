# session-manager

A would-be terminal multiplexer.

My initial MWE is 114 lines of Rust, although it does depend on [termion](https://gitlab.redox-os.org/redox-os/termion), ~2000 lines.

The objective is a simple terminal multiplexer (more [`dvtm`](https://github.com/martanne/dvtm) than [`tmux`](https://github.com/tmux/tmux)), but without the `ncurses` dependency.
I would like to be able to detach, but that's not a priority.

I'd be very happy if anyone has a tool that fills this niche.


## Status
`session-manager` can run a shell (or any application, really) in a pty.
It doesn't multiplex yet.

Tmux, vim, and friends seem to work correctly.

## Contributing
Contributions welcome.

Some tasks that need doing:
- [ ] A better way of collecting control sequences (I currently capture `0x18` and assume it is C-x)
- [ ] Error handling throughout the program
- [ ] Actual multiplexing
- [ ] Tests -- the MWE has none
- [x] Handle resize (SIGWINCH)
