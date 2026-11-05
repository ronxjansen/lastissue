//! Scale benchmarks (GUI-SPEC §3, §11 M6): 10k and 100k synthetic issues.
//!
//! `cargo bench -p lastissue-core --bench scale` (set `LASTISSUE_BENCH_DIR`
//! to choose where the stores are generated; `BENCH_SIZES=10000` to limit
//! the sizes). Each group mirrors a GUI path:
//!
//! - `scan`: opening a repo (`index::scan` + `Index::apply`).
//! - `view`: re-evaluating the list on a keystroke or watcher change
//!   (resolve, `matches_summary_fields`, instant text, sort, group).
//! - `fulltext`: the background tier over every file (read, prefilter,
//!   parse, line match), all candidates at once.
//! - `by_id`: the index lookup behind selection and the URL scheme.
//! - `activity`: parsing a long issue and building its timeline.
//!
//! The index's memory (an estimate of heap bytes, plus the RSS growth) is
//! printed once per size before the timings.

mod synth;

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use lastissue::identity::Identity;
use lastissue::index::{self, Index};
use lastissue::model::{self, Actor};
use lastissue::query::{self, Filter, Grouping, ListQuery, Sort};
use lastissue::search::Terms;
use lastissue::source::IssueSummary;
use lastissue::store::{Store, mbox};
use rayon::prelude::*;
use std::hint::black_box;
use std::time::Duration;

fn sizes() -> Vec<usize> {
    std::env::var("BENCH_SIZES")
        .ok()
        .map(|s| s.split(',').filter_map(|n| n.trim().parse().ok()).collect())
        .unwrap_or_else(|| vec![10_000, 100_000])
}

fn label(n: usize) -> String {
    if n.is_multiple_of(1000) {
        format!("{}k", n / 1000)
    } else {
        n.to_string()
    }
}

fn me() -> Identity {
    Identity::fixed(Actor {
        from: "Ada <ada@example.com>".into(),
        sender: None,
    })
}

fn load(store: &Store) -> Index {
    let mut ix = Index::default();
    ix.apply(index::scan(store).unwrap());
    ix
}

/// Heap bytes one summary holds (strings, vectors, its own size, and the
/// index's path key), ignoring allocator overhead.
fn heap_bytes(s: &IssueSummary) -> usize {
    let st = |x: &String| x.capacity();
    let ost = |x: &Option<String>| x.as_ref().map_or(0, String::capacity);
    let vs = |v: &Vec<String>| {
        v.capacity() * size_of::<String>() + v.iter().map(String::capacity).sum::<usize>()
    };
    size_of::<IssueSummary>()
        + 2 * s.path.as_os_str().len()
        + st(&s.id)
        + ost(&s.project)
        + ost(&s.project_slug)
        + st(&s.message_id)
        + st(&s.title)
        + vs(&s.labels)
        + ost(&s.assignee)
        + st(&s.author)
        + ost(&s.sender)
        + vs(&s.watchers)
        + ost(&s.parent)
        + vs(&s.blocked_by)
        + vs(&s.related)
}

/// Resident set size in KiB, via `ps` (no unsafe in this workspace).
fn rss_kib() -> Option<u64> {
    let out = std::process::Command::new("ps")
        .args(["-o", "rss=", "-p", &std::process::id().to_string()])
        .output()
        .ok()?;
    String::from_utf8(out.stdout).ok()?.trim().parse().ok()
}

#[allow(clippy::cast_precision_loss)]
fn report_memory(n: usize, store: &Store) {
    let before = rss_kib();
    let ix = load(store);
    let after = rss_kib();
    let heap: usize = ix.iter().map(heap_bytes).sum();
    let rss = match (before, after) {
        (Some(b), Some(a)) => format!("{:.1} MiB", a.saturating_sub(b) as f64 / 1024.0),
        _ => "?".into(),
    };
    eprintln!(
        "memory/{}: {} issues, index heap ≈ {:.1} MiB ({} B/issue), RSS growth {rss}",
        label(n),
        ix.len(),
        heap as f64 / (1024.0 * 1024.0),
        heap / ix.len().max(1),
    );
}

/// What `ViewState::compute_with` does per evaluation.
fn view(ix: &Index, store: &Store, me: &Identity, filter: &str) -> usize {
    let mut q = ListQuery::default();
    q.filter(filter).unwrap();
    let f = Filter::resolve(store, me, &q).unwrap();
    let blockers = f.blockers(ix.iter());
    let mut items: Vec<&IssueSummary> = ix
        .iter()
        .filter(|s| f.matches_summary_fields(s, &blockers))
        .filter(|s| query::instant_match(s, &f.text))
        .collect();
    query::sort_summaries(&mut items, Sort::Priority);
    let groups = query::group(&items, Grouping::State, &[]);
    let paths: Vec<std::path::PathBuf> = groups
        .iter()
        .flat_map(|g| g.items.iter().map(|s| s.path.clone()))
        .collect();
    paths.len()
}

/// The full-text tier over every file for one term.
fn fulltext(ix: &Index, term: &str) -> usize {
    let t = Terms::new(&[term.to_string()], false).unwrap();
    let paths: Vec<&std::path::Path> = ix.iter().map(|s| s.path.as_path()).collect();
    paths
        .par_iter()
        .filter(|p| {
            let Ok(bytes) = std::fs::read(p) else {
                return false;
            };
            if !t.prefilter(&bytes) {
                return false;
            }
            let Ok(msgs) = mbox::parse_mbox(&bytes) else {
                return false;
            };
            t.matches(&msgs).is_some()
        })
        .count()
}

fn benches(c: &mut Criterion) {
    let me = me();
    for n in sizes() {
        let root = synth::ensure(n);
        let store = Store::new(root);
        report_memory(n, &store);
        let ix = load(&store);
        let l = label(n);
        let slow = |g: &mut criterion::BenchmarkGroup<'_, _>| {
            g.sample_size(10).measurement_time(Duration::from_secs(10));
        };

        let mut g = c.benchmark_group("scan");
        slow(&mut g);
        g.bench_function(&l, |b| b.iter(|| black_box(load(&store).len())));
        g.finish();

        let mut g = c.benchmark_group("view");
        g.sample_size(20);
        for (name, filter) in [
            ("all", ""),
            ("tokens", "state:todo p:high label:bug"),
            ("text", "crash login"),
            ("ready", "is:ready"),
        ] {
            g.bench_with_input(BenchmarkId::new(name, &l), &filter, |b, f| {
                b.iter(|| black_box(view(&ix, &store, &me, f)));
            });
        }
        g.finish();

        let mut g = c.benchmark_group("fulltext");
        slow(&mut g);
        for term in [synth::RARE, synth::COMMON] {
            g.bench_with_input(BenchmarkId::new(term, &l), &term, |b, t| {
                b.iter(|| black_box(fulltext(&ix, t)));
            });
        }
        g.finish();

        let mut g = c.benchmark_group("by_id");
        let last = synth::short_id(n - 1);
        g.bench_function(&l, |b| b.iter(|| black_box(ix.by_id(&last).is_some())));
        g.finish();
    }

    let mut g = c.benchmark_group("activity");
    for comments in [100, 1000, 5000] {
        let bytes = synth::big_issue(comments);
        g.bench_with_input(BenchmarkId::from_parameter(comments), &bytes, |b, bytes| {
            b.iter(|| {
                let msgs = mbox::parse_mbox(bytes).unwrap();
                black_box(model::activity(&msgs).len())
            });
        });
    }
    g.finish();
}

criterion_group!(scale, benches);
criterion_main!(scale);
