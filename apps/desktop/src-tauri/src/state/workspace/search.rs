//! The `/` search box and the filter bar's full-text tier: instant
//! results under the lock, then debounced background scans in batches.

use super::{SharedHub, Sink, Workspace, emit, events, recompute};
use crate::dto::{SearchDto, VisibleDto};
use crate::format::error_line;
use crate::state::search::{self, Candidate, FullText, Hit};
use lastissue::query::{Filter, ListQuery};
use std::collections::HashSet;
use std::path::PathBuf;
use std::time::Duration;

/// The full-text tier starts after this long without a keystroke (§6).
const FULL_TEXT_DEBOUNCE: Duration = Duration::from_millis(150);
/// Files per background batch: small first, so early results stream in,
/// then doubling to amortize the round trips.
const FIRST_BATCH: usize = 64;
const MAX_BATCH: usize = 2048;

/// The filter bar's full-text tier: which candidate files have been
/// scanned for the current terms, and which matched.
#[derive(Default)]
pub(crate) struct FilterText {
    pub(super) terms: Vec<String>,
    pub(super) hits: HashSet<PathBuf>,
    pub(super) scanned: HashSet<PathBuf>,
    pub(crate) running: bool,
    pub(super) generation: u64,
}

/// The `/` search box (GUI-SPEC §6).
#[derive(Default)]
pub(crate) struct SearchState {
    pub(crate) generation: u64,
    pub(crate) query: String,
    pub(crate) hits: Vec<Hit>,
    pub(crate) scanning: bool,
    pub(crate) error: Option<String>,
}

/// The `/` box's text changed: instant results now, the full-text tier
/// after the debounce. Any scan of the previous text is dropped.
pub fn set_search(hub: &SharedHub, query: &str) {
    let mut ws = hub.lock();
    let generation = ws.search.generation + 1;
    ws.search = SearchState {
        generation,
        query: query.to_string(),
        ..SearchState::default()
    };
    let prepared = match (&ws.repo, query.trim().is_empty()) {
        (Some(r), false) => {
            let filter = (|| {
                let mut q = ListQuery::default();
                q.filter(query)?;
                Filter::resolve(&r.store, &ws.identity, &q)
            })();
            match filter {
                Ok(f) => Some((search::prepare(&r.index, &f), f.text)),
                Err(e) => {
                    ws.search.error = Some(error_line(&e));
                    None
                }
            }
        }
        _ => None,
    };
    if let Some((prepared, terms)) = prepared {
        ws.search.hits = prepared.hits;
        if !prepared.candidates.is_empty() {
            ws.search.scanning = true;
            spawn_scan(
                hub,
                prepared.candidates,
                terms,
                move |ws, sink, _paths, hits| {
                    if ws.search.generation != generation {
                        return false;
                    }
                    search::merge(&mut ws.search.hits, hits);
                    emit(sink, events::SEARCH, &SearchDto::new(ws));
                    true
                },
                move |ws, sink| {
                    if ws.search.generation == generation {
                        ws.search.scanning = false;
                        emit(sink, events::SEARCH, &SearchDto::new(ws));
                    }
                },
            );
        }
    }
    hub.emit(events::SEARCH, &SearchDto::new(&ws));
}

/// Run the full-text tier over `cands` in background batches after the
/// debounce. `batch` gets each batch's scanned paths and hits (under the
/// lock) and says whether to go on.
fn spawn_scan(
    hub: &SharedHub,
    cands: Vec<Candidate>,
    terms: Vec<String>,
    batch: impl Fn(&mut Workspace, &dyn Sink, Vec<PathBuf>, Vec<Hit>) -> bool + Send + 'static,
    done: impl FnOnce(&mut Workspace, &dyn Sink) + Send + 'static,
) {
    let hub = hub.clone();
    std::thread::spawn(move || {
        std::thread::sleep(FULL_TEXT_DEBOUNCE);
        let ft = FullText::new(&terms);
        let (mut start, mut size) = (0, FIRST_BATCH);
        while start < cands.len() {
            let end = (start + size).min(cands.len());
            let hits = search::scan(&cands[start..end], &ft);
            let paths = cands[start..end].iter().map(|c| c.path.clone()).collect();
            {
                let mut ws = hub.lock();
                if !batch(&mut ws, &*hub.sink, paths, hits) {
                    return;
                }
            }
            start = end;
            size = (size * 2).min(MAX_BATCH);
        }
        let mut ws = hub.lock();
        done(&mut ws, &*hub.sink);
    });
}

/// Scan the filter bar's unscanned candidates, unless a scan runs.
pub(super) fn scan_filter_text(hub: &SharedHub, ws: &mut Workspace) {
    if ws.filter_text.running {
        return;
    }
    let ft = &ws.filter_text;
    let todo: Vec<Candidate> = ws
        .visible
        .candidates
        .iter()
        .filter(|c| !ft.scanned.contains(&c.path))
        .cloned()
        .collect();
    if todo.is_empty() {
        return;
    }
    let (generation, terms) = (ft.generation, ft.terms.clone());
    ws.filter_text.running = true;
    let hub2 = hub.clone();
    let hub3 = hub.clone();
    spawn_scan(
        hub,
        todo,
        terms,
        move |ws, _, paths, hits| {
            if ws.filter_text.generation != generation {
                return false;
            }
            ws.filter_text.scanned.extend(paths);
            let any = !hits.is_empty();
            ws.filter_text.hits.extend(hits.into_iter().map(|h| h.path));
            if any {
                recompute(&hub2, ws, false);
            }
            true
        },
        move |ws, _| {
            if ws.filter_text.generation == generation {
                ws.filter_text.running = false;
                // Candidates may have grown meanwhile (scope change).
                recompute(&hub3, ws, true);
            }
        },
    );
    hub.emit(events::VISIBLE, &VisibleDto::new(ws));
}

#[cfg(test)]
mod tests {
    use super::super::fixture::{fx, wait};
    use super::super::set_view;
    use super::*;

    #[test]
    fn search_streams_full_text_hits() {
        let f = fx(&["alpha", "beta"]);
        set_search(&f.hub, "body of beta");
        wait("scan", || !f.hub.lock().search.scanning);
        let s = f.rec.last(events::SEARCH).unwrap();
        let hits = s["hits"].as_array().unwrap();
        assert_eq!(hits.len(), 1, "{s}");
        assert_eq!(hits[0]["tier"], "body");
    }

    #[test]
    fn filter_text_uses_full_text_tier() {
        let f = fx(&["alpha", "beta"]);
        let mut v = f.hub.lock().view.clone();
        v.filter = "\"body of alpha\"".into();
        set_view(&f.hub, v);
        wait("filter scan", || {
            f.rec
                .last(events::VISIBLE)
                .is_some_and(|v| v["total"] == 1 && v["scanning"] == false)
        });
    }
}
