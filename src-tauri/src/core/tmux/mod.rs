//! tmux control mode (`tmux -CC`) integration.
//!
//! [`detect`] sniffs control-mode notifications in a normal terminal stream;
//! [`session`] drives the control channel once detected, mapping tmux panes
//! onto virtual sessions so each gets its own terminal (native scrollback,
//! selection and input) in the frontend.

pub(crate) mod control;
pub(crate) mod detect;
mod handshake;
pub(crate) mod session;

pub(crate) use detect::{TmuxControlDetector, TmuxFeed, is_tmux_control_command};
pub(crate) use session::{ControlExit, run_control_session};

pub(crate) use handshake::verify as verify_control_channel;
