//! Native terminal frontend adapted from Zeron (MIT). This module creates no
//! shell or PTY. The existing Kivio terminal service remains the session owner.
//! Subscribe to `TerminalEvent`, forward Write/Resize to that owner, and deliver
//! its output using `TerminalView::feed_output`. Keep the subscription alive on
//! the hosting pane; dropping this view does not itself close the owned PTY.
mod emulator;
mod palette;
mod panel;
mod view;
pub use panel::{TerminalEvent, TerminalView};

mod host;
pub use host::TerminalHost;
