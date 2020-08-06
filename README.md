# session-manager

A would-be terminal multiplexer.

The objective is a simple terminal multiplexer (more [`dvtm`](https://github.com/martanne/dvtm) than [`tmux`](https://github.com/tmux/tmux)), but without the `ncurses` dependency.
I would like to be able to detach, but that's not a priority.

I'd be very happy if anyone has a tool that fills this niche.


## Status
`session-manager` can run a shell (or any application, really) in a pty.
See the [issue tracker](https://github.com/nw0/session-manager/issues) for status.

Applications seem to work correctly, but it doesn't multiplex yet.

## Contributing
Contributions welcome, especially for tests, performance, or colour support.
