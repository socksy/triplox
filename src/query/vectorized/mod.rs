pub(crate) mod batch;
pub(crate) mod engine;

#[cfg(test)]
mod tests;

use anyhow::Result;
use bytes::Bytes;
use edn::query::Variable;

use crate::query::exec_pattern::Proposal;
use batch::Batch;

// Columnar counterpart of `ExecPattern`. A pattern that implements it can run inside the batched
// engine, which never materializes intermediate rows: proposals come back as a parent-row map plus
// a value column, and validation comes back as the row indices to keep.
pub(crate) trait BatchPattern: Send + Sync {
    // Same contract as `ExecPattern::count`.
    fn count_batch(
        &self,
        batch: &Batch,
        added: &[Variable],
        proposals: &mut [Proposal],
    ) -> Result<()>;

    // Extends `batch` by one variable. `parent_rows` is ascending and pairs positionally with
    // `values`, so `(parent_rows[i], values[i])` is one output row.
    fn propose_batch(&self, batch: &Batch, added: &[Variable]) -> Result<(Vec<u32>, Vec<Bytes>)>;

    // Row indices of `batch` that satisfy this pattern, ascending.
    fn validate_batch(&self, batch: &Batch) -> Result<Vec<u32>>;

    // Columnar counterpart of `ExecPattern::candidate_sets`: sorted entity-id candidates per row
    // of `batch`, when they are available without IO.
    fn candidate_sets_batch<'a>(
        &'a self,
        _batch: &Batch,
        _added: &[Variable],
    ) -> Result<Option<Vec<&'a [i64]>>> {
        Ok(None)
    }
}

// Row indices of `len` rows ordered by `compare`, which is a total order over row indices.
pub(crate) fn sorted_rows(
    len: usize,
    compare: impl Fn(u32, u32) -> std::cmp::Ordering,
) -> Vec<u32> {
    let mut order: Vec<u32> = (0..len as u32).collect();
    order.sort_unstable_by(|left, right| compare(*left, *right));
    order
}
