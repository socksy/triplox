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

/// Encoded-value bounds on the V position of one triple pattern. Encodings are
/// order preserving within a type, so comparisons only apply when the type tag
/// (first byte) matches; otherwise the bound is ignored.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ValueBounds {
    pub lower: Option<Bound>,
    pub upper: Option<Bound>,
}

fn tag(encoded: &[u8]) -> Option<u8> {
    encoded.first().copied()
}

impl ValueBounds {
    pub(crate) fn is_empty(&self) -> bool {
        self.lower.is_none() && self.upper.is_none()
    }

    // Keep the tighter of two bounds of the same type; otherwise keep the existing one.
    fn tighten(existing: &mut Option<Bound>, candidate: Bound, keep_greater: bool) {
        match existing {
            None => *existing = Some(candidate),
            Some(current) if tag(&current.value) == tag(&candidate.value) => {
                let replace = match candidate.value.cmp(&current.value) {
                    std::cmp::Ordering::Greater => keep_greater,
                    std::cmp::Ordering::Less => !keep_greater,
                    std::cmp::Ordering::Equal => !candidate.inclusive,
                };
                if replace {
                    *current = candidate;
                }
            }
            Some(_) => {}
        }
    }

    pub(crate) fn add_lower(&mut self, value: Bytes, inclusive: bool) {
        Self::tighten(&mut self.lower, Bound { value, inclusive }, true);
    }

    pub(crate) fn add_upper(&mut self, value: Bytes, inclusive: bool) {
        Self::tighten(&mut self.upper, Bound { value, inclusive }, false);
    }

    /// True when the bound's type tag equals `encoded`'s, i.e. byte comparison is meaningful.
    fn comparable(bound: &Bound, encoded: &[u8]) -> bool {
        tag(&bound.value) == tag(encoded)
    }

    /// Whether a run whose V values span `[min_v, max_v]` may contain a matching value.
    pub(crate) fn run_may_match(&self, min_v: &[u8], max_v: &[u8]) -> bool {
        if let Some(lower) = &self.lower {
            if Self::comparable(lower, max_v) {
                match max_v.cmp(lower.value.as_ref()) {
                    std::cmp::Ordering::Less => return false,
                    std::cmp::Ordering::Equal if !lower.inclusive => return false,
                    _ => {}
                }
            }
        }
        if let Some(upper) = &self.upper {
            if Self::comparable(upper, min_v) {
                match min_v.cmp(upper.value.as_ref()) {
                    std::cmp::Ordering::Greater => return false,
                    std::cmp::Ordering::Equal if !upper.inclusive => return false,
                    _ => {}
                }
            }
        }
        true
    }

    /// Whether a single encoded value is definitely outside the bounds.
    pub(crate) fn excludes(&self, encoded: &[u8]) -> bool {
        !self.run_may_match(encoded, encoded)
    }

    /// Whether `encoded` lies above the upper bound (same type only).
    pub(crate) fn above_upper(&self, encoded: &[u8]) -> bool {
        match &self.upper {
            Some(upper) if Self::comparable(upper, encoded) => match encoded.cmp(&upper.value) {
                std::cmp::Ordering::Greater => true,
                std::cmp::Ordering::Equal => !upper.inclusive,
                std::cmp::Ordering::Less => false,
            },
            _ => false,
        }
    }

    /// Seek target for a sorted scan of values with the same type tag as `first_value`.
    pub(crate) fn seek_target(&self, first_value: &[u8]) -> Option<&Bytes> {
        match &self.lower {
            Some(lower) if Self::comparable(lower, first_value) => Some(&lower.value),
            _ => None,
        }
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
    use crate::ops::DataType;

    fn enc(value: i64) -> Bytes {
        Bytes::from(DataType::Long(value).encode())
    }

    #[test]
    fn bounds_prune_runs_at_boundaries() {
        let mut gt = ValueBounds::default();
        gt.add_lower(enc(900), false);
        assert!(!gt.run_may_match(&enc(0), &enc(900)));
        assert!(gt.run_may_match(&enc(0), &enc(901)));

        let mut ge = ValueBounds::default();
        ge.add_lower(enc(900), true);
        assert!(ge.run_may_match(&enc(0), &enc(900)));
        assert!(!ge.run_may_match(&enc(0), &enc(899)));

        let mut lt = ValueBounds::default();
        lt.add_upper(enc(10), false);
        assert!(!lt.run_may_match(&enc(10), &enc(20)));
        assert!(lt.run_may_match(&enc(9), &enc(20)));

        let mut le = ValueBounds::default();
        le.add_upper(enc(10), true);
        assert!(le.run_may_match(&enc(10), &enc(20)));
        assert!(!le.run_may_match(&enc(11), &enc(20)));
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
}
