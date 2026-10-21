//! Access to the host's local status projection.
use super::StatusBar;

pub fn from_state(state: &crate::state::UiState) -> StatusBar {
    StatusBar::from_state(state)
}
