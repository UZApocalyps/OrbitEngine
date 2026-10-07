//! Multi-selection of the asset library, without any UI: click, Ctrl+click
//! (toggle), Shift+click (range from the anchor, in the order shown),
//! select all, marquee. Items are named by their manifest path, so the
//! selection survives a rescan, a re-sort and a declaration (an undeclared
//! file keeps its path when it becomes an asset).

use std::collections::HashSet;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Selection {
    items: HashSet<String>,
    /// Where a Shift+click range starts: the last plain or Ctrl click.
    anchor: Option<String>,
}

impl Selection {
    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn contains(&self, path: &str) -> bool {
        self.items.contains(path)
    }

    #[cfg(test)]
    pub fn anchor(&self) -> Option<&str> {
        self.anchor.as_deref()
    }

    /// The selected paths, in the order of `order` (what is shown).
    pub fn ordered<'a>(&self, order: &'a [String]) -> Vec<&'a str> {
        order
            .iter()
            .filter(|p| self.items.contains(p.as_str()))
            .map(String::as_str)
            .collect()
    }

    pub fn clear(&mut self) {
        self.items.clear();
        self.anchor = None;
    }

    /// Exactly `path`.
    pub fn set_one(&mut self, path: &str) {
        self.items.clear();
        self.items.insert(path.to_owned());
        self.anchor = Some(path.to_owned());
    }

    /// A click on `clicked`; `order` is the list as shown.
    pub fn click(&mut self, order: &[String], clicked: &str, ctrl: bool, shift: bool) {
        if shift {
            let anchor = self.anchor.clone().unwrap_or_else(|| clicked.to_owned());
            let a = order.iter().position(|p| *p == anchor);
            let b = order.iter().position(|p| p == clicked);
            let (Some(a), Some(b)) = (a, b) else {
                self.set_one(clicked);
                return;
            };
            if !ctrl {
                self.items.clear();
            }
            let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
            self.items.extend(order[lo..=hi].iter().cloned());
            // The anchor stays: Shift+click again re-ranges from it.
            self.anchor = Some(anchor);
        } else if ctrl {
            if !self.items.remove(clicked) {
                self.items.insert(clicked.to_owned());
            }
            self.anchor = Some(clicked.to_owned());
        } else {
            self.set_one(clicked);
        }
    }

    /// Everything shown.
    pub fn select_all(&mut self, order: &[String]) {
        self.items = order.iter().cloned().collect();
        if self.anchor.as_ref().is_none_or(|a| !self.items.contains(a)) {
            self.anchor = order.first().cloned();
        }
    }

    /// The marquee's result: `hits`, added to `base` when `additive`.
    pub fn marquee(&mut self, base: &Selection, hits: &[String], additive: bool) {
        self.items = if additive {
            base.items.clone()
        } else {
            HashSet::new()
        };
        self.items.extend(hits.iter().cloned());
        self.anchor = hits.first().cloned().or_else(|| base.anchor.clone());
    }

    /// Forgets what no longer exists (after a rescan). Returns whether
    /// anything was dropped.
    pub fn retain(&mut self, exists: impl Fn(&str) -> bool) -> bool {
        let before = self.items.len();
        self.items.retain(|p| exists(p));
        if self.anchor.as_deref().is_some_and(|a| !exists(a)) {
            self.anchor = None;
        }
        self.items.len() != before
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn order() -> Vec<String> {
        ["a", "b", "c", "d", "e"].iter().map(|s| (*s).to_owned()).collect()
    }

    fn sel(s: &Selection, order: &[String]) -> Vec<String> {
        s.ordered(order).into_iter().map(str::to_owned).collect()
    }

    #[test]
    fn plain_click_selects_one_and_ctrl_toggles() {
        let o = order();
        let mut s = Selection::default();
        s.click(&o, "b", false, false);
        assert_eq!(sel(&s, &o), ["b"]);
        s.click(&o, "d", true, false);
        assert_eq!(sel(&s, &o), ["b", "d"]);
        s.click(&o, "b", true, false);
        assert_eq!(sel(&s, &o), ["d"]);
        s.click(&o, "e", false, false);
        assert_eq!(sel(&s, &o), ["e"]);
    }

    #[test]
    fn shift_selects_a_range_from_the_anchor_both_ways() {
        let o = order();
        let mut s = Selection::default();
        s.click(&o, "b", false, false);
        s.click(&o, "d", false, true);
        assert_eq!(sel(&s, &o), ["b", "c", "d"]);
        // Re-ranging from the same anchor shrinks / flips the range.
        s.click(&o, "a", false, true);
        assert_eq!(sel(&s, &o), ["a", "b"]);
        assert_eq!(s.anchor(), Some("b"));
        // Ctrl+Shift adds a range to what is selected.
        s.click(&o, "e", true, false);
        s.click(&o, "d", true, true);
        assert_eq!(sel(&s, &o), ["a", "b", "d", "e"]);
    }

    #[test]
    fn shift_without_anchor_or_with_a_vanished_anchor_selects_one() {
        let o = order();
        let mut s = Selection::default();
        s.click(&o, "c", false, true);
        assert_eq!(sel(&s, &o), ["c"]);
        s.click(&o, "a", false, false);
        s.retain(|p| p != "a");
        s.click(&o, "d", false, true);
        assert_eq!(sel(&s, &o), ["d"]);
    }

    #[test]
    fn select_all_marquee_and_retain() {
        let o = order();
        let mut s = Selection::default();
        s.select_all(&o);
        assert_eq!(s.len(), 5);
        assert!(s.retain(|p| p != "c"));
        assert!(!s.contains("c"));
        let base = s.clone();
        let hits = vec!["c".to_owned()];
        s.marquee(&base, &hits, false);
        assert_eq!(sel(&s, &o), ["c"]);
        s.marquee(&base, &hits, true);
        assert_eq!(s.len(), 5);
        s.clear();
        assert!(s.is_empty() && s.anchor().is_none());
    }
}
