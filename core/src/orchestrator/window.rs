//! The orchestrator's in-flight window.
//!
//! The orchestrator reads events ahead of its durable cursor and handles
//! events of different workflows concurrently, so events resolve out of
//! order. The window records every fetched event that is not yet
//! acknowledged, in cursor order, and yields the cursor that may be stored:
//! the end of the contiguous resolved prefix. The durable cursor therefore
//! never passes an unresolved event.

use std::collections::VecDeque;

use crate::events::EventCursor;

/// Fetched, unacknowledged events in cursor order
#[derive(Debug, Default)]
pub struct InFlightWindow {
    entries: VecDeque<(EventCursor, bool)>,
}

impl InFlightWindow {
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of fetched events not yet acknowledged
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Record a fetched event. Events must be pushed in cursor order.
    pub fn push(&mut self, cursor: EventCursor) {
        debug_assert!(
            self.entries.back().is_none_or(|(last, _)| *last < cursor),
            "events must enter the window in cursor order"
        );
        self.entries.push_back((cursor, false));
    }

    /// Mark an event resolved. Returns the cursor to acknowledge if the
    /// resolved prefix grew: the last event of the prefix, which has been
    /// removed from the window together with everything before it.
    pub fn resolve(&mut self, cursor: EventCursor) -> Option<EventCursor> {
        if let Ok(index) = self.entries.binary_search_by(|(c, _)| c.cmp(&cursor)) {
            self.entries[index].1 = true;
        }

        let mut acknowledged = None;
        while let Some((front, true)) = self.entries.front().copied() {
            acknowledged = Some(front);
            self.entries.pop_front();
        }
        acknowledged
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cursor(position: i64) -> EventCursor {
        EventCursor { tx: 1, position }
    }

    #[test]
    fn test_window_acknowledges_only_the_resolved_prefix() {
        let mut window = InFlightWindow::new();
        (1..=4).for_each(|p| window.push(cursor(p)));

        // Later events resolving first acknowledge nothing
        assert_eq!(window.resolve(cursor(3)), None);
        assert_eq!(window.resolve(cursor(2)), None);
        assert_eq!(window.len(), 4);

        // The front resolving releases the whole resolved prefix
        assert_eq!(window.resolve(cursor(1)), Some(cursor(3)));
        assert_eq!(window.len(), 1);

        assert_eq!(window.resolve(cursor(4)), Some(cursor(4)));
        assert!(window.is_empty());
    }

    #[test]
    fn test_window_ignores_an_unknown_cursor() {
        let mut window = InFlightWindow::new();
        window.push(cursor(1));
        assert_eq!(window.resolve(cursor(9)), None);
        assert_eq!(window.len(), 1);
    }
}
