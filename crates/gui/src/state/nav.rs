//! Keyboard navigation and multi-selection over the visible issues
//! (GUI-SPEC §7). Plain data over columns of short IDs: the list is one
//! column in display order, the board one column per group.
//!
//! The cursor is the keyboard's position (`j`/`k`, `h`/`l`); the
//! selection is the set `x` toggles, which issue commands then act on in
//! bulk. The issue open in the detail pane is separate (`Enter`/`o`).

/// Where `id` sits: (column, row).
pub fn locate(cols: &[Vec<String>], id: &str) -> Option<(usize, usize)> {
    cols.iter()
        .enumerate()
        .find_map(|(c, col)| col.iter().position(|x| x == id).map(|r| (c, r)))
}

fn first(cols: &[Vec<String>]) -> Option<String> {
    cols.iter().find_map(|c| c.first().cloned())
}

/// `j`/`k`: `step` rows within the cursor's column, stopping at the ends.
/// Without a (visible) cursor, the first issue.
pub fn vertical(cols: &[Vec<String>], cursor: Option<&str>, step: isize) -> Option<String> {
    let Some((c, r)) = cursor.and_then(|id| locate(cols, id)) else {
        return first(cols);
    };
    let col = &cols[c];
    let r = r.saturating_add_signed(step).min(col.len() - 1);
    col.get(r).cloned()
}

/// `h`/`l` on the board: the next non-empty column in that direction, at
/// the same row or the column's last.
pub fn horizontal(cols: &[Vec<String>], cursor: Option<&str>, step: isize) -> Option<String> {
    let Some((c, r)) = cursor.and_then(|id| locate(cols, id)) else {
        return first(cols);
    };
    let mut i = c;
    loop {
        i = match i.checked_add_signed(step.signum()) {
            Some(i) if i < cols.len() => i,
            _ => return cols[c].get(r).cloned(),
        };
        if let Some(last) = cols[i].len().checked_sub(1) {
            return cols[i].get(r.min(last)).cloned();
        }
    }
}

/// Short IDs `x` has selected, in the order they were picked.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Selection(Vec<String>);

impl Selection {
    pub fn toggle(&mut self, id: &str) {
        match self.0.iter().position(|x| x == id) {
            Some(i) => {
                self.0.remove(i);
            }
            None => self.0.push(id.to_string()),
        }
    }

    pub fn contains(&self, id: &str) -> bool {
        self.0.iter().any(|x| x == id)
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn ids(&self) -> &[String] {
        &self.0
    }

    pub fn clear(&mut self) {
        self.0.clear();
    }

    /// Select every visible issue (`⌘A`).
    pub fn set_all(&mut self, cols: &[Vec<String>]) {
        self.0 = cols.iter().flatten().cloned().collect();
    }

    /// Drop IDs that no longer exist.
    pub fn retain(&mut self, keep: impl Fn(&str) -> bool) {
        self.0.retain(|x| keep(x));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cols(spec: &[&[&str]]) -> Vec<Vec<String>> {
        spec.iter()
            .map(|c| c.iter().map(|s| s.to_string()).collect())
            .collect()
    }

    #[test]
    fn vertical_moves_within_a_column_and_stops_at_ends() {
        let c = cols(&[&["a", "b", "c"]]);
        assert_eq!(vertical(&c, None, 1).as_deref(), Some("a"));
        assert_eq!(vertical(&c, Some("gone"), -1).as_deref(), Some("a"));
        assert_eq!(vertical(&c, Some("a"), 1).as_deref(), Some("b"));
        assert_eq!(vertical(&c, Some("c"), 1).as_deref(), Some("c"));
        assert_eq!(vertical(&c, Some("a"), -1).as_deref(), Some("a"));
        assert_eq!(vertical(&cols(&[&[]]), None, 1), None);
    }

    #[test]
    fn horizontal_skips_empty_columns_and_clamps_the_row() {
        let c = cols(&[&["a1", "a2", "a3"], &[], &["c1"], &["d1", "d2"]]);
        assert_eq!(horizontal(&c, Some("a3"), 1).as_deref(), Some("c1"));
        assert_eq!(horizontal(&c, Some("c1"), 1).as_deref(), Some("d1"));
        assert_eq!(horizontal(&c, Some("d2"), -1).as_deref(), Some("c1"));
        assert_eq!(horizontal(&c, Some("c1"), -1).as_deref(), Some("a1"));
        // At the edge: stay.
        assert_eq!(horizontal(&c, Some("d2"), 1).as_deref(), Some("d2"));
        assert_eq!(horizontal(&c, None, 1).as_deref(), Some("a1"));
    }

    #[test]
    fn selection_toggles_in_pick_order() {
        let mut s = Selection::default();
        s.toggle("b");
        s.toggle("a");
        s.toggle("c");
        s.toggle("a");
        assert_eq!(s.ids(), ["b", "c"]);
        s.retain(|x| x != "b");
        assert_eq!(s.ids(), ["c"]);
        s.set_all(&cols(&[&["x"], &["y"]]));
        assert_eq!(s.ids(), ["x", "y"]);
        s.clear();
        assert!(s.is_empty());
    }
}
