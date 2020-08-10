# session-manager

[![](https://img.shields.io/github/workflow/status/nw0/session-manager/Rust)](https://github.com/nw0/session-manager/actions)
[![codecov](https://codecov.io/gh/nw0/session-manager/branch/master/graph/badge.svg)](https://codecov.io/gh/nw0/session-manager)
[![](https://tokei.rs/b1/github/nw0/session-manager)](https://github.com/nw0/session-manager)

A simple terminal multiplexer.

The objective is a simple terminal multiplexer (more [`dvtm`](https://github.com/martanne/dvtm) than [`tmux`](https://github.com/tmux/tmux)), but without the `ncurses` dependency.
I'd also like to detach.


## Status
`session-manager` can multiplex terminal applications, and they do appear to work correctly.
It's still a little rough around the edges, but should behave correctly.

See the [issue tracker](https://github.com/nw0/session-manager/issues) for status.


## Contributing
Contributions welcome, especially for tests, performance, or colour support.
