//! Pure admission policy shared with firmware: input never crosses a USB session
//! boundary or survives its fixed execution deadline.

/// Maximum time from receiver admission to completion of a HID action.
pub const INPUT_TTL_MS: u64 = 30_000;

/// A job is usable only while the same USB session is active and its deadline
/// has not been reached. `accepted_ms` and `now_ms` are monotonic milliseconds.
pub const fn is_current(
    accepted_epoch: u32,
    current_epoch: u32,
    accepted_ms: u64,
    now_ms: u64,
    active: bool,
) -> bool {
    active && accepted_epoch == current_epoch && now_ms < accepted_ms.saturating_add(INPUT_TTL_MS)
}

/// Cancel an action when the USB session changes or its deadline is reached.
/// The cancellation and deadline futures are polled first, so an action that is
/// ready at the same time as expiry cannot start or resume after the deadline.
pub async fn run_until_invalid<C, D, F>(cancel: C, deadline: D, action: F) -> bool
where
    C: core::future::Future<Output = ()>,
    D: core::future::Future<Output = ()>,
    F: core::future::Future<Output = ()>,
{
    use embassy_futures::select::{Either, select};
    matches!(
        select(cancel, select(deadline, action)).await,
        Either::Second(Either::Second(()))
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancellation_and_deadline_win_before_a_ready_action() {
        use core::{
            cell::Cell,
            future::{pending, ready},
        };
        let executed = Cell::new(false);
        let action = async { executed.set(true) };
        assert!(!embassy_futures::block_on(run_until_invalid(
            ready(()),
            pending(),
            action
        )));
        assert!(!executed.get());
        let action = async { executed.set(true) };
        assert!(!embassy_futures::block_on(run_until_invalid(
            pending(),
            ready(()),
            action
        )));
        assert!(!executed.get());
        let action = async { executed.set(true) };
        assert!(embassy_futures::block_on(run_until_invalid(
            pending(),
            pending(),
            action
        )));
        assert!(executed.get());
    }

    #[test]
    fn expires_at_deadline_and_across_suspend_or_reconnect() {
        assert!(is_current(9, 9, 1_000, 30_999, true));
        assert!(!is_current(9, 9, 1_000, 31_000, true));
        assert!(!is_current(9, 9, 1_000, 2_000, false));
        assert!(!is_current(9, 10, 1_000, 2_000, true));
        assert!(is_current(10, 10, 2_000, 2_001, true));
    }
}
