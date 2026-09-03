use std::collections::HashMap;
use std::sync::Arc;

use anyhow::{ensure, Context, Result};

use super::batch::Batch;
use super::BatchPattern;
use crate::query::binding_bag::BindingBag;
use crate::query::exec_pattern::{PatternId, Proposal};
use crate::query::stage::Stage;

// Batched generic join. Same stage/proposal structure and the same row ordering as
// `GenericJoinEngine`, but bindings live in columns with structural sharing instead of one
// `Vec<Bytes>` per row.
pub(crate) struct BatchedJoinEngine;

impl BatchedJoinEngine {
    // The batched path only exists for patterns that implement `BatchPattern`; anything else
    // (not/or/rules/functions) falls back to the row engine.
    pub(crate) fn supports(stages: &[Stage]) -> bool {
        stages.iter().all(|stage| {
            stage
                .participants()
                .iter()
                .all(|participant| participant.as_batch().is_some())
        })
    }

    fn batch_pattern(pattern: &dyn crate::query::exec_pattern::ExecPattern) -> &dyn BatchPattern {
        pattern
            .as_batch()
            .expect("supports() checked every participant")
    }

    fn validate_all<'a>(
        mut batch: Arc<Batch>,
        validators: impl IntoIterator<Item = &'a dyn crate::query::exec_pattern::ExecPattern>,
    ) -> Result<Arc<Batch>> {
        for validator in validators {
            let kept = Self::batch_pattern(validator)
                .validate_batch(&batch)
                .with_context(|| format!("Pattern {} failed during validation", validator.id()))?;
            batch = Arc::new(batch.select(kept));
        }
        Ok(batch)
    }

    fn propose(
        stage: &Stage,
        proposer: &dyn crate::query::exec_pattern::ExecPattern,
        input: &Arc<Batch>,
    ) -> Result<Arc<Batch>> {
        let added = stage.added();
        ensure!(
            added.len() == 1,
            "Batched stages extend exactly one variable, got {added:?}"
        );
        let (parent_rows, values) = Self::batch_pattern(proposer)
            .propose_batch(input, added)
            .with_context(|| format!("Pattern {} failed during proposal", proposer.id()))?;
        let extended = input.extend(parent_rows, added[0].clone(), values)?;
        let reordered = extended.reorder(stage.target_variables())?;
        let validators = stage
            .participants()
            .iter()
            .filter(|participant| participant.id() != proposer.id())
            .map(|participant| participant.as_ref());
        Self::validate_all(Arc::new(reordered), validators)
    }

    fn execute_proposing_stage(stage: &Stage, input: &Arc<Batch>) -> Result<Arc<Batch>> {
        if stage.proposers().len() == 1 {
            let proposer = stage
                .proposers()
                .next()
                .expect("proposing stages have at least one proposer");
            return Self::propose(stage, proposer.as_ref(), input);
        }

        let mut proposals = vec![Proposal::default(); input.len()];
        for proposer in stage.proposers() {
            Self::batch_pattern(proposer.as_ref())
                .count_batch(input, stage.added(), &mut proposals)
                .with_context(|| {
                    format!("Pattern {} failed while counting proposals", proposer.id())
                })?;
        }
        let mut shards: HashMap<PatternId, Vec<u32>> = HashMap::new();
        for (row, proposal) in proposals.iter().enumerate() {
            let Some(proposer) = proposal.proposer() else {
                continue;
            };
            shards.entry(proposer).or_default().push(row as u32);
        }
        let mut ordered_shards: Vec<_> = shards.into_iter().collect();
        // Multi-proposer output is grouped by proposer in first-occurrence order.
        ordered_shards.sort_unstable_by_key(|(_, rows)| rows[0]);

        let mut results = Vec::with_capacity(ordered_shards.len());
        for (proposer_id, rows) in ordered_shards {
            let proposer = stage
                .proposers()
                .find(|proposer| proposer.id() == proposer_id)
                .ok_or_else(|| anyhow::anyhow!("Unknown proposer id {proposer_id}"))?;
            let shard = Arc::new(input.select(rows));
            results.push(Self::propose(stage, proposer.as_ref(), &shard)?);
        }
        Ok(Arc::new(Batch::concat(
            stage.target_variables().to_vec(),
            &results,
        )?))
    }

    fn execute_stage(stage: &Stage, input: &Arc<Batch>) -> Result<Arc<Batch>> {
        let result = if stage.added().is_empty() {
            let validated = Self::validate_all(
                Arc::clone(input),
                stage
                    .participants()
                    .iter()
                    .map(|participant| participant.as_ref()),
            )?;
            Arc::new(validated.reorder(stage.target_variables())?)
        } else {
            Self::execute_proposing_stage(stage, input)?
        };
        ensure!(
            result.variables == stage.target_variables(),
            "Stage produced layout {:?}, expected {:?}",
            result.variables,
            stage.target_variables()
        );
        Ok(result)
    }

    pub(crate) fn execute(stages: &[Stage]) -> Result<BindingBag> {
        let mut batch = Arc::new(Batch::unit());
        for stage in stages {
            batch = Self::execute_stage(stage, &batch)?;
        }
        batch.to_binding_bag()
    }
}
