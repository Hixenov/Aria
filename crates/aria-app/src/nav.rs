//! Sidebar routes and the capped back/forward navigation history.

use std::collections::VecDeque;

use crate::ui::theme::icons;

/// Maximum number of entries kept on the back stack.
pub const HISTORY_CAP: usize = 50;

/// The nine sidebar pages.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Route {
    Home,
    Search,
    Library,
    Liked,
    Playlists,
    Albums,
    Artists,
    LocalMusic,
    Settings,
}

impl Route {
    /// Every route, in sidebar order.
    pub const ALL: [Route; 9] = [
        Route::Home,
        Route::Search,
        Route::Library,
        Route::Liked,
        Route::Playlists,
        Route::Albums,
        Route::Artists,
        Route::LocalMusic,
        Route::Settings,
    ];

    /// Fluent message id for the sidebar label.
    pub fn label_key(self) -> &'static str {
        match self {
            Route::Home => "nav-home",
            Route::Search => "nav-search",
            Route::Library => "nav-library",
            Route::Liked => "nav-liked",
            Route::Playlists => "nav-playlists",
            Route::Albums => "nav-albums",
            Route::Artists => "nav-artists",
            Route::LocalMusic => "nav-local",
            Route::Settings => "nav-settings",
        }
    }

    /// Phosphor icon codepoint for the sidebar entry.
    pub fn icon(self) -> &'static str {
        match self {
            Route::Home => icons::HOUSE,
            Route::Search => icons::MAGNIFYING_GLASS,
            Route::Library => icons::BOOKS,
            Route::Liked => icons::HEART,
            Route::Playlists => icons::PLAYLIST,
            Route::Albums => icons::VINYL_RECORD,
            Route::Artists => icons::MICROPHONE_STAGE,
            Route::LocalMusic => icons::FOLDER_SIMPLE,
            Route::Settings => icons::GEAR,
        }
    }

    /// The delivery phase that implements this page, or `None` for pages with no phase.
    pub fn phase(self) -> Option<u8> {
        match self {
            Route::Home => Some(2),
            Route::Search | Route::Library | Route::Playlists | Route::Albums | Route::Artists => {
                Some(3)
            }
            Route::Liked | Route::LocalMusic | Route::Settings => None,
        }
    }
}

/// Back/forward navigation over [`Route`]s, with a capped back stack.
#[derive(Debug)]
pub struct History {
    current: Route,
    back: VecDeque<Route>,
    forward: Vec<Route>,
}

impl History {
    /// Starts at `start` with empty back and forward stacks.
    pub fn new(start: Route) -> History {
        History {
            current: start,
            back: VecDeque::new(),
            forward: Vec::new(),
        }
    }

    /// The route currently shown.
    pub fn current(&self) -> Route {
        self.current
    }

    /// Moves to `to`. Navigating to the current route does nothing; otherwise the
    /// current route goes on the back stack (dropping the oldest entry above the cap)
    /// and the forward stack is cleared.
    pub fn navigate(&mut self, to: Route) {
        if to == self.current {
            return;
        }
        self.back.push_back(self.current);
        if self.back.len() > HISTORY_CAP {
            self.back.pop_front();
        }
        self.forward.clear();
        self.current = to;
    }

    /// Steps back one entry. Returns `false` when there is nothing to go back to.
    pub fn back(&mut self) -> bool {
        match self.back.pop_back() {
            Some(prev) => {
                self.forward.push(self.current);
                self.current = prev;
                true
            }
            None => false,
        }
    }

    /// Steps forward one entry. Returns `false` when there is nothing to go forward to.
    pub fn forward(&mut self) -> bool {
        match self.forward.pop() {
            Some(next) => {
                self.back.push_back(self.current);
                self.current = next;
                true
            }
            None => false,
        }
    }

    /// Whether [`History::back`] would move.
    pub fn can_back(&self) -> bool {
        !self.back.is_empty()
    }

    /// Whether [`History::forward`] would move.
    pub fn can_forward(&self) -> bool {
        !self.forward.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn navigate_back_forward() {
        let mut h = History::new(Route::Home);
        h.navigate(Route::Search);
        h.navigate(Route::Albums);
        assert!(h.back());
        assert_eq!(h.current(), Route::Search);
        assert!(h.forward());
        assert_eq!(h.current(), Route::Albums);
        assert!(!h.forward());
    }

    #[test]
    fn navigate_clears_forward() {
        let mut h = History::new(Route::Home);
        h.navigate(Route::Search);
        h.back();
        h.navigate(Route::Artists);
        assert!(!h.can_forward());
    }

    #[test]
    fn same_route_is_noop() {
        let mut h = History::new(Route::Home);
        h.navigate(Route::Home);
        assert!(!h.can_back());
    }

    #[test]
    fn back_stack_is_capped() {
        let mut h = History::new(Route::Home);
        for i in 0..200 {
            h.navigate(if i % 2 == 0 {
                Route::Search
            } else {
                Route::Home
            });
        }
        let mut n = 0;
        while h.back() {
            n += 1;
        }
        assert_eq!(n, HISTORY_CAP);
    }

    #[test]
    fn every_route_has_a_label_key_in_english() {
        let ids = aria_core::i18n::message_ids(aria_core::i18n::SOURCES[0].1);
        for r in Route::ALL {
            assert!(ids.contains(r.label_key()), "{r:?}");
        }
    }
}
