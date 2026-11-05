//! Two-tier search (GUI-SPEC §6). Plain data; the workspace runs [`scan`]
//! on a background thread.
//!
//! 1. **Instant**, in memory: every free-text term is looked up in the
//!    title, short ID, labels, assignee and project of each summary
//!    ([`classify`], via core's `term_tier`).
//! 2. **Full text**: an issue with a term the instant tier didn't find is a
//!    [`Candidate`]; [`scan`] reads its file and runs core's search over
//!    the body and comments for just the missing terms, keeping snippets.
//!
//! Every term must match somewhere. Results are tiered, not ranked
//! ([`rank`]): title → label/ID → body → comment, then newest first. An
//! issue's tier is its weakest term's best location (all terms in the
//! title is a title match; one term only in a comment is a comment match).

use lastissue::index::Index;
pub use lastissue::query::Tier;
use lastissue::query::{Filter, term_tier};
use lastissue::search::Terms;
use lastissue::source::IssueSummary;
use lastissue::store::mbox;
use rayon::prelude::*;
use std::path::PathBuf;

/// Where a full-text term matched: `body` or `#n` (comment number), and
/// the line around the hit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snippet {
    pub loc: String,
    pub text: String,
}

/// One search result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hit {
    pub path: PathBuf,
    pub id: String,
    /// Unix time of the newest message, for ordering.
    pub updated: i64,
    pub tier: Tier,
    /// Empty for instant hits.
    pub snippets: Vec<Snippet>,
}

/// An issue the instant tier couldn't decide: `missing` terms (indices)
/// need the full-text scan; `tier` is the weakest tier found so far.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub path: PathBuf,
    pub id: String,
    pub updated: i64,
    pub tier: Tier,
    pub missing: Vec<usize>,
}

fn updated(s: &IssueSummary) -> i64 {
    s.updated.or(s.created).map_or(0, |d| d.timestamp())
}

/// The instant tier's verdict on one summary for lowercased `terms`: a hit
/// when every term was found (no terms: a title-tier hit), else a
/// candidate for the full-text scan.
pub fn classify(s: &IssueSummary, terms: &[String]) -> Result<Hit, Candidate> {
    let mut tier = Tier::Title;
    let mut missing = Vec::new();
    for (i, t) in terms.iter().enumerate() {
        match term_tier(s, t) {
            Some(x) => tier = tier.max(x),
            None => missing.push(i),
        }
    }
    if missing.is_empty() {
        Ok(Hit {
            path: s.path.clone(),
            id: s.id.clone(),
            updated: updated(s),
            tier,
            snippets: Vec::new(),
        })
    } else {
        Err(Candidate {
            path: s.path.clone(),
            id: s.id.clone(),
            updated: updated(s),
            tier,
            missing,
        })
    }
}

/// The instant results and the full-text candidates for a resolved filter
/// over the whole index.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Prepared {
    pub hits: Vec<Hit>,
    pub candidates: Vec<Candidate>,
}

pub fn prepare(index: &Index, filter: &Filter) -> Prepared {
    let blockers = filter.blockers(index.iter());
    let mut out = Prepared::default();
    for s in index.iter() {
        if !filter.matches_summary_fields(s, &blockers) {
            continue;
        }
        match classify(s, &filter.text) {
            Ok(h) => out.hits.push(h),
            Err(c) => out.candidates.push(c),
        }
    }
    rank(&mut out.hits);
    out
}

/// The free-text terms compiled for the scan, one matcher per term (so
/// each candidate is checked for its missing terms only).
#[derive(Debug, Clone)]
pub struct FullText(Vec<Terms>);

impl FullText {
    pub fn new(terms: &[String]) -> Self {
        FullText(
            terms
                .iter()
                .map(|t| Terms::new(std::slice::from_ref(t), false).expect("literal terms compile"))
                .collect(),
        )
    }
}

/// Snippets kept per hit.
const SNIPPETS: usize = 3;

/// Run the full-text tier on one candidate: raw prefilter first, then a
/// parse and line match for each missing term over the body and comments.
pub fn scan_one(c: &Candidate, ft: &FullText) -> Option<Hit> {
    let bytes = std::fs::read(&c.path).ok()?;
    if !c.missing.iter().all(|&i| ft.0[i].prefilter(&bytes)) {
        return None;
    }
    let msgs = mbox::parse_mbox(&bytes).ok()?;
    let mut tier = c.tier;
    let mut snippets: Vec<Snippet> = Vec::new();
    for &i in &c.missing {
        let m = ft.0[i].matches(&msgs)?;
        let mut best: Option<Tier> = None;
        for line in &m.shown {
            let Some((loc, text)) = line.split_once(": ") else {
                continue;
            };
            let (t, loc) = match loc {
                "subject" => (Tier::Title, None),
                "body" => (Tier::Body, Some("body".to_string())),
                n => (
                    Tier::Comment,
                    Some(format!("#{}", n.trim_start_matches('/'))),
                ),
            };
            best = Some(best.map_or(t, |b| b.min(t)));
            if let Some(loc) = loc {
                let s = Snippet {
                    loc,
                    text: text.to_string(),
                };
                if !snippets.contains(&s) {
                    snippets.push(s);
                }
            }
        }
        tier = tier.max(best?);
    }
    snippets.truncate(SNIPPETS);
    Some(Hit {
        path: c.path.clone(),
        id: c.id.clone(),
        updated: c.updated,
        tier,
        snippets,
    })
}

/// [`scan_one`] over a batch, in parallel.
pub fn scan(cands: &[Candidate], ft: &FullText) -> Vec<Hit> {
    cands.par_iter().filter_map(|c| scan_one(c, ft)).collect()
}

/// Tiered order (GUI-SPEC §6): tier, then newest first, then ID.
pub fn rank(hits: &mut [Hit]) {
    hits.sort_by(|a, b| {
        a.tier
            .cmp(&b.tier)
            .then(b.updated.cmp(&a.updated))
            .then(a.id.cmp(&b.id))
    });
}

/// Fold a scanned batch into ranked results, replacing any earlier hit for
/// the same file.
pub fn merge(hits: &mut Vec<Hit>, new: Vec<Hit>) {
    if new.is_empty() {
        return;
    }
    hits.retain(|h| !new.iter().any(|n| n.path == h.path));
    hits.extend(new);
    rank(hits);
}

#[cfg(test)]
mod tests {
    use super::*;
    use lastissue::identity::Identity;
    use lastissue::index;
    use lastissue::model::{Actor, State};
    use lastissue::ops::{self, CommentTarget, NewIssue};
    use lastissue::query::ListQuery;
    use lastissue::store::{self, Store};

    struct Fx {
        _dir: tempfile::TempDir,
        s: Store,
        me: Identity,
        ix: Index,
    }

    fn fx() -> Fx {
        let dir = tempfile::tempdir().unwrap();
        let s = Store::new(store::init(dir.path()).unwrap());
        let me = Identity::fixed(Actor {
            from: "Ada <ada@example.com>".into(),
            sender: None,
        });
        for (title, body, labels, comment, state) in [
            (
                "Login crashes",
                "Steps: submit an empty form",
                vec![],
                None,
                State::Todo,
            ),
            (
                "Avatar upload",
                "the crash happens on login too",
                vec![],
                None,
                State::Todo,
            ),
            ("Docs typo", "nothing", vec!["login"], None, State::Todo),
            (
                "Slow startup",
                "profile it",
                vec![],
                Some("found a login loop, crashes later"),
                State::Done,
            ),
            ("Unrelated", "zzz", vec![], None, State::Todo),
        ] {
            let n = NewIssue {
                title: title.into(),
                body: body.into(),
                labels: labels.into_iter().map(String::from).collect(),
                state: Some(state),
                ..NewIssue::default()
            };
            let out = ops::create_issue(&s, &me, &n).unwrap();
            if let Some(c) = comment {
                let t = CommentTarget {
                    issue: out.issue.clone(),
                    reply_to: None,
                };
                ops::add_comment(&s, &me, &t, c).unwrap();
            }
        }
        let mut ix = Index::default();
        ix.apply(index::scan(&s).unwrap());
        Fx {
            _dir: dir,
            s,
            me,
            ix,
        }
    }

    fn filter(f: &Fx, q: &str) -> Filter {
        let mut lq = ListQuery::default();
        lq.filter(q).unwrap();
        Filter::resolve(&f.s, &f.me, &lq).unwrap()
    }

    fn run(f: &Fx, q: &str) -> Vec<(String, Tier, Vec<String>)> {
        let flt = filter(f, q);
        let p = prepare(&f.ix, &flt);
        let mut hits = p.hits;
        let ft = FullText::new(&flt.text);
        merge(&mut hits, scan(&p.candidates, &ft));
        hits.into_iter()
            .map(|h| {
                let title = f.ix.get(&h.path).unwrap().title.clone();
                let snips = h
                    .snippets
                    .iter()
                    .map(|s| format!("{}: {}", s.loc, s.text))
                    .collect();
                (title, h.tier, snips)
            })
            .collect()
    }

    #[test]
    fn tiers_order_title_field_body_comment() {
        let f = fx();
        let r = run(&f, "login");
        let got: Vec<(&str, Tier)> = r.iter().map(|(t, tier, _)| (t.as_str(), *tier)).collect();
        assert_eq!(
            got,
            [
                ("Login crashes", Tier::Title),
                ("Docs typo", Tier::Field),
                ("Avatar upload", Tier::Body),
                // A search includes closed issues (as `issue list --search`).
                ("Slow startup", Tier::Comment),
            ]
        );
        assert_eq!(r[2].2, ["body: the crash happens on login too"]);
        assert_eq!(r[3].2, ["#1: found a login loop, crashes later"]);
    }

    #[test]
    fn every_term_must_match_and_tier_is_the_weakest() {
        let f = fx();
        // "login" in the title, "empty" only in the body.
        let r = run(&f, "login empty");
        assert_eq!(r.len(), 1);
        assert_eq!((r[0].0.as_str(), r[0].1), ("Login crashes", Tier::Body));
        assert!(run(&f, "login nowhere-at-all").is_empty());
        // Tokens narrow the set before any text matching.
        let r = run(&f, "login state:done");
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].0, "Slow startup");
    }

    #[test]
    fn merge_replaces_and_reranks() {
        let h = |id: &str, tier, updated| Hit {
            path: PathBuf::from(id),
            id: id.into(),
            updated,
            tier,
            snippets: Vec::new(),
        };
        let mut hits = vec![h("a", Tier::Body, 5), h("b", Tier::Title, 1)];
        rank(&mut hits);
        merge(
            &mut hits,
            vec![h("c", Tier::Body, 9), h("a", Tier::Comment, 5)],
        );
        let ids: Vec<&str> = hits.iter().map(|h| h.id.as_str()).collect();
        assert_eq!(ids, ["b", "c", "a"]);
    }
}
