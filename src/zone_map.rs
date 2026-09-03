//! Zone maps: per-run min/max summaries over an index prefix, used to skip
//! seeks and runs that cannot contain matching datoms.
//!
//! A zone map covers one `(index, attribute)` prefix. Keys are split into runs
//! of `RUN_SIZE` consecutive keys (in key order) and each run records the
//! min/max of one non-key-prefix component (V for AEV, E for AVE) and the
//! oldest/newest tx id. Maps are built in memory by scanning the prefix and are
//! cached per basis: a map built when the basis was `S` stays sound for any
//! query at basis `<= S` because index keys are never deleted and every later
//! key carries a tx id `> S`.
//!
//! Enabled with `TRIPLOX_ZONE_MAPS=1` (or `set_enabled`).

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Instant;

use anyhow::Error;
use bytes::Bytes;
use slatedb::DbReadOps;
use tokio::runtime::Handle;

use crate::codec;
use crate::protocol::{TAG_BIG_INT, TAG_DOUBLE, TAG_FLOAT, TAG_INSTANT, TAG_LONG, TAG_STRING};
use crate::slate::DEFAULT_SCAN_OPTIONS;

pub const RUN_SIZE: usize = 256;

static ENABLED: OnceLock<AtomicBool> = OnceLock::new();

fn enabled_flag() -> &'static AtomicBool {
    ENABLED.get_or_init(|| {
        let on = std::env::var("TRIPLOX_ZONE_MAPS")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false);
        AtomicBool::new(on)
    })
}

pub fn enabled() -> bool {
    enabled_flag().load(Ordering::Relaxed)
}

pub fn set_enabled(on: bool) {
    enabled_flag().store(on, Ordering::Relaxed);
}

// ---------------------------------------------------------------------------
// Counters (process-wide, reset by `take_stats`)
// ---------------------------------------------------------------------------

static SEEKS_SKIPPED: AtomicU64 = AtomicU64::new(0);
static SEEKS_CHECKED: AtomicU64 = AtomicU64::new(0);
static RUNS_SKIPPED_T: AtomicU64 = AtomicU64::new(0);
static BUILDS: AtomicU64 = AtomicU64::new(0);
static BUILD_MICROS: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ZoneMapStats {
    /// AEV per-entity seeks skipped because no run in the entity's range can match the V bounds.
    pub seeks_skipped: u64,
    /// AEV per-entity seeks that consulted a zone map.
    pub seeks_checked: u64,
    /// Runs jumped over by the temporal filter because every key was newer than the basis.
    pub runs_skipped_t: u64,
    pub builds: u64,
    pub build_micros: u64,
}

pub fn take_stats() -> ZoneMapStats {
    ZoneMapStats {
        seeks_skipped: SEEKS_SKIPPED.swap(0, Ordering::Relaxed),
        seeks_checked: SEEKS_CHECKED.swap(0, Ordering::Relaxed),
        runs_skipped_t: RUNS_SKIPPED_T.swap(0, Ordering::Relaxed),
        builds: BUILDS.swap(0, Ordering::Relaxed),
        build_micros: BUILD_MICROS.swap(0, Ordering::Relaxed),
    }
}

// ---------------------------------------------------------------------------
// Value bounds pushed down from comparison predicates
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Bound {
    pub value: Bytes,
    pub inclusive: bool,
}

/// Encoded-value bounds on the V position of one triple pattern. Comparisons are
/// only meaningful between encodings of the same type, so a bound is ignored
/// unless the value's type tag matches and that tag has a known byte order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ValueBounds {
    pub lower: Option<Bound>,
    pub upper: Option<Bound>,
}

fn tag(encoded: &[u8]) -> Option<u8> {
    encoded.first().copied()
}

/// How an encoding's byte order relates to the type's own order. `encode_i64`
/// XORs with `i64::MAX`, which is strictly order *reversing* over the whole
/// range, so integers and instants sort descending in index keys; floats and
/// strings sort ascending.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Direction {
    Ascending,
    Descending,
}

fn direction(tag: u8) -> Option<Direction> {
    match tag {
        TAG_LONG | TAG_BIG_INT | TAG_INSTANT => Some(Direction::Descending),
        TAG_DOUBLE | TAG_FLOAT | TAG_STRING => Some(Direction::Ascending),
        _ => None,
    }
}

impl ValueBounds {
    pub(crate) fn is_empty(&self) -> bool {
        self.lower.is_none() && self.upper.is_none()
    }

    /// Order of two same-type encoded values in the type's own order, or `None`
    /// when the tags differ or the encoding's direction is unknown.
    fn value_cmp(left: &[u8], right: &[u8]) -> Option<std::cmp::Ordering> {
        let left_tag = tag(left)?;
        if tag(right) != Some(left_tag) {
            return None;
        }
        let raw = left.cmp(right);
        Some(match direction(left_tag)? {
            Direction::Ascending => raw,
            Direction::Descending => raw.reverse(),
        })
    }

    // Keep the tighter of two bounds of the same type; otherwise keep the existing one.
    fn tighten(existing: &mut Option<Bound>, candidate: Bound, keep_greater: bool) {
        match existing {
            None => *existing = Some(candidate),
            Some(current) => {
                let Some(order) = Self::value_cmp(&candidate.value, &current.value) else {
                    return;
                };
                let replace = match order {
                    std::cmp::Ordering::Greater => keep_greater,
                    std::cmp::Ordering::Less => !keep_greater,
                    std::cmp::Ordering::Equal => !candidate.inclusive,
                };
                if replace {
                    *current = candidate;
                }
            }
        }
    }

    pub(crate) fn add_lower(&mut self, value: Bytes, inclusive: bool) {
        Self::tighten(&mut self.lower, Bound { value, inclusive }, true);
    }

    pub(crate) fn add_upper(&mut self, value: Bytes, inclusive: bool) {
        Self::tighten(&mut self.upper, Bound { value, inclusive }, false);
    }

    /// Whether a run whose encoded values span the byte range `[byte_min, byte_max]`
    /// may contain a matching value.
    pub(crate) fn run_may_match(&self, byte_min: &[u8], byte_max: &[u8]) -> bool {
        // Under a descending encoding the byte extremes are the value extremes swapped.
        let descending = tag(byte_min).and_then(direction) == Some(Direction::Descending);
        let (value_min, value_max) = if descending {
            (byte_max, byte_min)
        } else {
            (byte_min, byte_max)
        };
        if let Some(lower) = &self.lower {
            match Self::value_cmp(value_max, &lower.value) {
                Some(std::cmp::Ordering::Less) => return false,
                Some(std::cmp::Ordering::Equal) if !lower.inclusive => return false,
                _ => {}
            }
        }
        if let Some(upper) = &self.upper {
            match Self::value_cmp(value_min, &upper.value) {
                Some(std::cmp::Ordering::Greater) => return false,
                Some(std::cmp::Ordering::Equal) if !upper.inclusive => return false,
                _ => {}
            }
        }
        true
    }

    /// Whether a single encoded value is definitely outside the bounds.
    pub(crate) fn excludes(&self, encoded: &[u8]) -> bool {
        !self.run_may_match(encoded, encoded)
    }

    /// The bound a byte-ordered scan reaches first, i.e. where it should start.
    fn byte_start_bound(&self, tag_source: &[u8]) -> Option<&Bound> {
        let bound = match tag(tag_source).and_then(direction)? {
            Direction::Ascending => self.lower.as_ref(),
            Direction::Descending => self.upper.as_ref(),
        }?;
        (tag(&bound.value) == tag(tag_source)).then_some(bound)
    }

    /// The bound a byte-ordered scan reaches last, i.e. where it should stop.
    fn byte_end_bound(&self, tag_source: &[u8]) -> Option<&Bound> {
        let bound = match tag(tag_source).and_then(direction)? {
            Direction::Ascending => self.upper.as_ref(),
            Direction::Descending => self.lower.as_ref(),
        }?;
        (tag(&bound.value) == tag(tag_source)).then_some(bound)
    }

    /// Whether a byte-ordered scan has passed the last value that can match.
    pub(crate) fn past_byte_end(&self, encoded: &[u8]) -> bool {
        match self.byte_end_bound(encoded) {
            Some(end) => match encoded.cmp(&end.value) {
                std::cmp::Ordering::Greater => true,
                std::cmp::Ordering::Equal => !end.inclusive,
                std::cmp::Ordering::Less => false,
            },
            None => false,
        }
    }

    /// Seek target for a byte-ordered scan of values with `first_value`'s type tag.
    pub(crate) fn seek_target(&self, first_value: &[u8]) -> Option<&Bytes> {
        self.byte_start_bound(first_value).map(|bound| &bound.value)
    }
}

// ---------------------------------------------------------------------------
// Zone map
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
struct Run {
    /// First key of the run; the run ends at the next run's start.
    start: Bytes,
    min_component: Bytes,
    max_component: Bytes,
    /// Largest encoded tx id in the run = oldest tx (encoding is descending).
    oldest_ts: [u8; 8],
}

/// Which non-prefix component the run min/max summarises.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Component {
    /// V in `[AEV][attr][E][V][T][op]`
    AevValue,
    /// E in `[AVE][attr][V][E][T][op]`
    AveEntity,
}

impl Component {
    pub(crate) fn for_index(index: u8) -> Option<Self> {
        match index {
            codec::AEV => Some(Self::AevValue),
            codec::AVE => Some(Self::AveEntity),
            _ => None,
        }
    }

    fn extract<'a>(self, key: &'a [u8]) -> &'a [u8] {
        let end = key.len() - codec::TX_EID_OP_SUFFIX;
        match self {
            Self::AevValue => {
                &key[codec::CODEC_LENGTH + codec::ATTRIBUTE_LENGTH + codec::ENTITY_LENGTH..end]
            }
            Self::AveEntity => &key[end - codec::ENTITY_LENGTH..end],
        }
    }
}

#[derive(Debug)]
pub(crate) struct ZoneMap {
    runs: Vec<Run>,
    keys: u64,
}

impl ZoneMap {
    /// Build by scanning every key under `prefix`. Runs are `RUN_SIZE` keys in key order.
    pub(crate) fn build<D>(
        prefix: &[u8],
        slate: &D,
        handle: &Handle,
        component: Component,
    ) -> Result<Self, Error>
    where
        D: DbReadOps + Send + Sync,
    {
        let started = Instant::now();
        let mut iterator =
            handle.block_on(slate.scan_prefix_with_options(prefix, .., &DEFAULT_SCAN_OPTIONS))?;
        let mut runs: Vec<Run> = Vec::new();
        let mut in_run = 0usize;
        let mut keys = 0u64;
        while let Some(kv) = handle.block_on(iterator.next())? {
            let key = kv.key;
            let comp = component.extract(&key);
            let ts: [u8; 8] = key
                [key.len() - codec::TX_EID_OP_SUFFIX..key.len() - codec::OP_LENGTH]
                .try_into()
                .expect("tx id suffix");
            keys += 1;
            if in_run == RUN_SIZE || runs.is_empty() {
                runs.push(Run {
                    min_component: Bytes::copy_from_slice(comp),
                    max_component: Bytes::copy_from_slice(comp),
                    oldest_ts: ts,
                    start: key.clone(),
                });
                in_run = 0;
            } else {
                let run = runs.last_mut().expect("run exists");
                if comp < run.min_component.as_ref() {
                    run.min_component = Bytes::copy_from_slice(comp);
                }
                if comp > run.max_component.as_ref() {
                    run.max_component = Bytes::copy_from_slice(comp);
                }
                if ts > run.oldest_ts {
                    run.oldest_ts = ts;
                }
            }
            in_run += 1;
        }
        BUILDS.fetch_add(1, Ordering::Relaxed);
        BUILD_MICROS.fetch_add(started.elapsed().as_micros() as u64, Ordering::Relaxed);
        Ok(Self { runs, keys })
    }

    pub(crate) fn runs(&self) -> usize {
        self.runs.len()
    }

    pub(crate) fn keys(&self) -> u64 {
        self.keys
    }

    /// Index of the run whose start is `<= key`, if any.
    fn run_index(&self, key: &[u8]) -> Option<usize> {
        self.runs
            .partition_point(|run| run.start.as_ref() <= key)
            .checked_sub(1)
    }

    /// Whether some key in `[lo, hi)` (hi = None means unbounded) may match `bounds`.
    /// Keys before the first run cannot exist at any basis the map is valid for.
    pub(crate) fn range_may_match(
        &self,
        lo: &[u8],
        hi: Option<&[u8]>,
        bounds: &ValueBounds,
    ) -> bool {
        SEEKS_CHECKED.fetch_add(1, Ordering::Relaxed);
        let first = self.run_index(lo).unwrap_or(0);
        let end = match hi {
            Some(hi) => self.runs.partition_point(|run| run.start.as_ref() < hi),
            None => self.runs.len(),
        };
        let may = self.runs[first..end.max(first)]
            .iter()
            .any(|run| bounds.run_may_match(&run.min_component, &run.max_component));
        if !may {
            SEEKS_SKIPPED.fetch_add(1, Ordering::Relaxed);
        }
        may
    }

    /// If every key in `key`'s run is newer than the basis (`as_of_encoded` is the
    /// descending-encoded basis tx), return the start of the next run to seek to.
    /// Returns `Err(())`-like `None` for the last run, which has no successor to jump to.
    pub(crate) fn newer_run_end(&self, key: &[u8], as_of_encoded: &[u8; 8]) -> Option<&Bytes> {
        let index = self.run_index(key)?;
        let run = &self.runs[index];
        if run.oldest_ts < *as_of_encoded {
            let next = self.runs.get(index + 1)?;
            RUNS_SKIPPED_T.fetch_add(1, Ordering::Relaxed);
            Some(&next.start)
        } else {
            None
        }
    }
}

// ---------------------------------------------------------------------------
// Cache: one map per (index, attribute), tagged with the basis it was built at
// ---------------------------------------------------------------------------

#[derive(Default)]
pub struct ZoneMapCache {
    maps: Mutex<HashMap<(u8, i64), (i64, Arc<ZoneMap>)>>,
}

impl std::fmt::Debug for ZoneMapCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ZoneMapCache").finish_non_exhaustive()
    }
}

impl ZoneMapCache {
    /// Return the cached map for `(index, attribute)` if it was built at a basis `>= basis`,
    /// otherwise build one via `build` and cache it under `basis`.
    pub(crate) fn get_or_build(
        &self,
        index: u8,
        attribute: i64,
        basis: i64,
        build: impl FnOnce() -> Result<ZoneMap, Error>,
    ) -> Result<Arc<ZoneMap>, Error> {
        if let Some((built_at, map)) = self.maps.lock().unwrap().get(&(index, attribute)) {
            if *built_at >= basis {
                return Ok(Arc::clone(map));
            }
        }
        let map = Arc::new(build()?);
        self.maps
            .lock()
            .unwrap()
            .insert((index, attribute), (basis, Arc::clone(&map)));
        Ok(map)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::Encode;
    use crate::node::{Node, QueryNode, SubmitNode};
    use crate::ops::{DataType, TxOp};
    use crate::memory_log::MemoryLog;
    use crate::schema::test_schema_tx;
    use edn::kw;
    use triplox_client::node::Database;
    use triplox_client::transaction::{TransactionResult, TxKey};

    fn enc(value: i64) -> Bytes {
        Bytes::from(DataType::Long(value).encode())
    }

    /// Byte extremes of a run holding `values`, as `ZoneMap::build` records them.
    fn run(values: &[i64]) -> (Bytes, Bytes) {
        let mut encoded: Vec<Bytes> = values.iter().copied().map(enc).collect();
        encoded.sort();
        (encoded[0].clone(), encoded[encoded.len() - 1].clone())
    }

    fn may_match(bounds: &ValueBounds, values: &[i64]) -> bool {
        let (byte_min, byte_max) = run(values);
        bounds.run_may_match(&byte_min, &byte_max)
    }

    // Long keys are encoded with `value ^ i64::MAX`, so their byte order is the
    // reverse of their numeric order; the bounds must not assume ascending bytes.
    #[test]
    fn long_encoding_is_order_reversing() {
        assert!(enc(901) < enc(900));
        assert!(enc(0) < enc(-1));
        assert!(enc(i64::MAX) < enc(i64::MIN));
    }

    #[test]
    fn bounds_prune_runs_at_boundaries() {
        let mut gt = ValueBounds::default();
        gt.add_lower(enc(900), false);
        assert!(!may_match(&gt, &[0, 900]));
        assert!(may_match(&gt, &[0, 901]));

        let mut ge = ValueBounds::default();
        ge.add_lower(enc(900), true);
        assert!(may_match(&ge, &[0, 900]));
        assert!(!may_match(&ge, &[0, 899]));

        let mut lt = ValueBounds::default();
        lt.add_upper(enc(10), false);
        assert!(!may_match(&lt, &[10, 20]));
        assert!(may_match(&lt, &[9, 20]));

        let mut le = ValueBounds::default();
        le.add_upper(enc(10), true);
        assert!(may_match(&le, &[10, 20]));
        assert!(!may_match(&le, &[11, 20]));

        let mut window = ValueBounds::default();
        window.add_lower(enc(100), true);
        window.add_upper(enc(200), false);
        assert!(may_match(&window, &[150, 150]));
        assert!(!may_match(&window, &[200, 300]));
        assert!(!may_match(&window, &[0, 99]));
        assert!(may_match(&window, &[0, 300]));
    }

    #[test]
    fn bounds_handle_negative_values() {
        let mut gt = ValueBounds::default();
        gt.add_lower(enc(-1), false);
        assert!(!may_match(&gt, &[-10, -1]));
        assert!(may_match(&gt, &[-10, 0]));
        assert!(gt.excludes(&enc(-1)));
        assert!(!gt.excludes(&enc(0)));
    }

    #[test]
    fn bounds_ignore_mismatched_types() {
        let mut gt = ValueBounds::default();
        gt.add_lower(enc(900), false);
        let s = Bytes::from(DataType::String("a".into()).encode());
        assert!(gt.run_may_match(&s, &s));
        assert!(!gt.excludes(&s));
        assert!(gt.excludes(&enc(900)));
        assert!(!gt.excludes(&enc(901)));
    }

    #[test]
    fn tighten_keeps_tighter_bound() {
        let mut b = ValueBounds::default();
        b.add_lower(enc(5), true);
        b.add_lower(enc(7), false);
        b.add_lower(enc(6), true);
        assert_eq!(b.lower.as_ref().unwrap().value, enc(7));
        b.add_upper(enc(20), true);
        b.add_upper(enc(20), false);
        assert!(!b.upper.as_ref().unwrap().inclusive);
        b.add_upper(enc(30), false);
        assert_eq!(b.upper.as_ref().unwrap().value, enc(20));
    }

    // For a descending encoding the scan starts at the upper bound and stops at the lower.
    #[test]
    fn byte_scan_bounds_follow_the_encoding_direction() {
        let mut long = ValueBounds::default();
        long.add_lower(enc(900), false);
        long.add_upper(enc(950), true);
        assert_eq!(long.seek_target(&enc(0)), Some(&enc(950)));
        assert!(!long.past_byte_end(&enc(901)));
        assert!(long.past_byte_end(&enc(900)));
        assert!(long.past_byte_end(&enc(899)));

        let text = |v: &str| Bytes::from(DataType::String(v.into()).encode());
        let mut string = ValueBounds::default();
        string.add_lower(text("m"), true);
        string.add_upper(text("q"), false);
        assert_eq!(string.seek_target(&text("a")), Some(&text("m")));
        assert!(!string.past_byte_end(&text("p")));
        assert!(string.past_byte_end(&text("q")));
    }

    // The toggle is process-wide, so the on/off tests take turns.
    static TOGGLE: Mutex<()> = Mutex::new(());

    // Ages spread across 0..300 so a run's value range covers almost everything.
    fn scattered(index: i64) -> i64 {
        (index * 7) % 300
    }

    const EQUIVALENCE_QUERIES: &[&str] = &[
        "[:find ?e :where [?e :age ?a] [(> ?a 200)]]",
        "[:find ?e :where [?e :age ?a] [(>= ?a 200)]]",
        "[:find ?e :where [?e :age ?a] [(< ?a 20)]]",
        "[:find ?e :where [?e :age ?a] [(<= ?a 20)]]",
        "[:find ?e ?a :where [?e :age ?a] [(>= ?a 150)] [(< ?a 160)]]",
        "[:find ?e :where [?e :age ?a] [(> ?a 100000)]]",
        "[:find ?e :where [?e :age ?a] [(>= ?a -5)]]",
        // Bound entity from the first pattern, so the AEV scan seeks per entity.
        "[:find ?e ?n :where [?e :name ?n] [?e :age ?a] [(> ?a 290)]]",
        // String bounds exercise the type-tag check.
        "[:find ?e :where [?e :name ?n] [(> ?n \"name-0500\")]]",
        "[:find (count ?e) :where [?e :age ?a]]",
    ];

    async fn commit(node: &Node<MemoryLog>, ops: Vec<TxOp>) -> TxKey {
        match node.execute_tx(ops).await.expect("tx executes") {
            TransactionResult::TxCommitted(key) => key,
            TransactionResult::TxAborted(_, error) => panic!("tx aborted: {error}"),
        }
    }

    /// 600 entities, so an AEV run boundary (RUN_SIZE = 256) falls inside the
    /// attribute. `age` maps an entity's insertion index to its `:age` value.
    async fn seeded_node(age: impl Fn(i64) -> i64) -> (Node<MemoryLog>, TxKey) {
        let node = Node::memory_node().await;
        commit(&node, test_schema_tx()).await;
        let mut early = None;
        for chunk in 0..4 {
            let ops = (0..150)
                .map(|offset| {
                    let index = chunk * 150 + offset;
                    vec![
                        TxOp::Add {
                            entity: format!("e{index}").into(),
                            attribute: kw!(:age),
                            value: DataType::Long(age(index)),
                        },
                        TxOp::Add {
                            entity: format!("e{index}").into(),
                            attribute: kw!(:name),
                            value: DataType::String(format!("name-{index:04}")),
                        },
                    ]
                })
                .flatten()
                .collect::<Vec<_>>();
            let key = commit(&node, ops).await;
            if chunk == 1 {
                early = Some(key);
            }
        }
        (node, early.expect("early basis"))
    }

    async fn run_all(node: &Node<MemoryLog>, early: TxKey) -> Vec<Vec<Vec<DataType>>> {
        let latest = node.db().await.expect("db");
        let as_of = node.db_as_of(early).await.expect("db as of");
        let mut out = Vec::new();
        for query in EQUIVALENCE_QUERIES {
            for db in [&latest, &as_of] {
                let mut rows = db.query(*query).await.expect("query runs");
                rows.sort_by_key(|row| format!("{row:?}"));
                out.push(rows);
            }
        }
        out
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn results_match_with_zone_maps_on_and_off() {
        let _guard = TOGGLE.lock().unwrap_or_else(|error| error.into_inner());
        let previous = enabled();

        set_enabled(false);
        let (node, early) = seeded_node(scattered).await;
        let off = run_all(&node, early).await;

        set_enabled(true);
        // A fresh node so the cache is built from scratch under the toggle.
        let (node_on, early_on) = seeded_node(scattered).await;
        let on = run_all(&node_on, early_on).await;
        // Same node, warm cache built at the latest basis, queried again including as-of.
        let on_same_node = run_all(&node, early).await;
        set_enabled(previous);

        for (index, query) in EQUIVALENCE_QUERIES.iter().enumerate() {
            for (offset, label) in [(0, "latest"), (1, "as-of")] {
                let slot = index * 2 + offset;
                assert_eq!(
                    off[slot], on[slot],
                    "{label} rows differ for {query} (cold cache)"
                );
                assert_eq!(
                    off[slot], on_same_node[slot],
                    "{label} rows differ for {query} (warm cache)"
                );
            }
        }
        assert!(
            off.iter().any(|rows| !rows.is_empty()),
            "queries produced no rows at all"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn zone_map_prunes_and_counts_skips() {
        let _guard = TOGGLE.lock().unwrap_or_else(|error| error.into_inner());
        let previous = enabled();
        set_enabled(true);
        // Ages that track insertion order, so a run's value range is narrow.
        let (node, _) = seeded_node(|index| index).await;
        let db = node.db().await.expect("db");
        let query = "[:find ?e ?n :where [?e :name ?n] [?e :age ?a] [(> ?a 550)]]";
        // Warm the cache, then measure one execution.
        let _ = db.query(query).await.expect("query runs");
        take_stats();
        let rows = db.query(query).await.expect("query runs");
        let stats = take_stats();
        set_enabled(previous);
        assert_eq!(rows.len(), 49, "ages 551..599");
        assert!(
            stats.seeks_checked > 0,
            "zone map was never consulted: {stats:?}"
        );
        assert!(
            stats.seeks_skipped > 0,
            "no per-entity seeks were pruned: {stats:?}"
        );
    }
}
