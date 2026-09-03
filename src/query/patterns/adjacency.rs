//! Triple pattern over a ref attribute served from an in-memory adjacency matrix
//! instead of SlateDB iterators. Same contract as `TriplePattern`.

use std::sync::Arc;

use anyhow::{ensure, Result};
use edn::query::Variable;

use crate::query::adjacency::{decode_entity, AdjMatrix, Csr};
use crate::query::binding_bag::{BindingBag, BindingRow};
use crate::query::exec_pattern::{ExecPattern, PatternId, Proposal};
use crate::query::patterns::triple::TripleTerm;

#[derive(Clone, Copy)]
enum Position {
    Entity,
    Value,
}

pub(crate) struct AdjacencyPattern {
    id: PatternId,
    variables: Vec<Variable>,
    entity: TripleTerm,
    value: TripleTerm,
    matrix: Arc<AdjMatrix>,
}

impl AdjacencyPattern {
    /// Returns None when a constant term is not an entity id, so the caller can fall back.
    pub(crate) fn new(
        id: PatternId,
        entity: TripleTerm,
        value: TripleTerm,
        matrix: Arc<AdjMatrix>,
    ) -> Result<Option<Self>> {
        for term in [&entity, &value] {
            if let TripleTerm::Constant(constant) = term {
                if decode_entity(constant).is_none() {
                    return Ok(None);
                }
            }
        }
        let mut variables = Vec::new();
        for term in [&entity, &value] {
            if let TripleTerm::Variable(variable) = term {
                ensure!(
                    !variables.contains(variable),
                    "Triple pattern {id} repeats variable {variable}"
                );
                variables.push(variable.clone());
            }
        }
        Ok(Some(Self {
            id,
            variables,
            entity,
            value,
            matrix,
        }))
    }

    fn position_for_variable(&self, variable: &Variable) -> Option<Position> {
        match (&self.entity, &self.value) {
            (TripleTerm::Variable(v), _) if v == variable => Some(Position::Entity),
            (_, TripleTerm::Variable(v)) if v == variable => Some(Position::Value),
            _ => None,
        }
    }

    // The CSR whose rows are keyed by the term opposite to `position`.
    fn csr_for(&self, position: Position) -> &Csr {
        match position {
            Position::Entity => &self.matrix.inn,
            Position::Value => &self.matrix.out,
        }
    }

    // The CSR whose keys are every id occurring at `position`.
    fn keys_for(&self, position: Position) -> &Csr {
        match position {
            Position::Entity => &self.matrix.out,
            Position::Value => &self.matrix.inn,
        }
    }

    fn other_term(&self, position: Position) -> &TripleTerm {
        match position {
            Position::Entity => &self.value,
            Position::Value => &self.entity,
        }
    }

    /// Per input row: the CSR row for the proposed position, the full key list when the
    /// other side is unbound, or nothing when the other side is not an entity id.
    fn row_keys(&self, input: &BindingBag, position: Position) -> Result<Vec<RowKey>> {
        Ok(match self.other_term(position) {
            TripleTerm::Variable(other) if input.column_indexes.contains_key(other) => {
                let index = input.column_index(other)?;
                input
                    .rows
                    .iter()
                    .map(|row| decode_entity(&row[index]).map_or(RowKey::Missing, RowKey::Key))
                    .collect()
            }
            TripleTerm::Variable(_) => vec![RowKey::All; input.rows.len()],
            TripleTerm::Constant(constant) => {
                let key = decode_entity(constant).map_or(RowKey::Missing, RowKey::Key);
                vec![key; input.rows.len()]
            }
        })
    }

    fn validate(&self, input: &BindingBag) -> Result<BindingBag> {
        let bound = |term: &TripleTerm| match term {
            TripleTerm::Variable(variable) => input
                .column_indexes
                .get(variable)
                .map(|index| Bound::Column(*index)),
            TripleTerm::Constant(constant) => Some(Bound::Constant(decode_entity(constant))),
        };
        let (entity, value) = (bound(&self.entity), bound(&self.value));
        ensure!(
            entity.is_some() || value.is_some(),
            "Triple pattern {} has no bound variables to validate",
            self.id
        );
        let mut matched = Vec::new();
        for (row_index, row) in input.rows.iter().enumerate() {
            let ok = match (entity, value) {
                (Some(entity), Some(value)) => match (entity.resolve(row), value.resolve(row)) {
                    (Some(entity), Some(value)) => self.matrix.out.contains(entity, value),
                    _ => false,
                },
                (Some(entity), None) => entity
                    .resolve(row)
                    .is_some_and(|e| self.matrix.out.has_key(e)),
                (None, Some(value)) => value
                    .resolve(row)
                    .is_some_and(|v| self.matrix.inn.has_key(v)),
                (None, None) => unreachable!(),
            };
            if ok {
                matched.push(row_index);
            }
        }
        input.select_rows(&matched)
    }

    fn propose(&self, input: &BindingBag, added: &[Variable]) -> Result<BindingBag> {
        let position = self.proposed_position(input, added)?;
        let csr = self.csr_for(position);
        let extensions: Vec<Vec<BindingRow>> = self
            .row_keys(input, position)?
            .into_iter()
            .map(|key| {
                let values = match key {
                    RowKey::Key(key) => csr.row_encoded(key),
                    RowKey::All => self.keys_for(position).keys_encoded(),
                    RowKey::Missing => Vec::new(),
                };
                values.into_iter().map(|value| vec![value]).collect()
            })
            .collect();
        input.extend_rows(added.to_vec(), extensions)
    }

    fn proposed_position(&self, input: &BindingBag, added: &[Variable]) -> Result<Position> {
        ensure!(
            added.len() == 1,
            "Triple pattern {} can propose exactly one variable, got {added:?}",
            self.id
        );
        ensure!(
            !input.variables.contains(&added[0]),
            "Triple pattern {} cannot add already-bound variable {}",
            self.id,
            added[0]
        );
        self.position_for_variable(&added[0]).ok_or_else(|| {
            anyhow::anyhow!(
                "Triple pattern {} cannot propose variable {}",
                self.id,
                added[0]
            )
        })
    }
}

#[derive(Clone, Copy)]
enum RowKey {
    All,
    Key(i64),
    Missing,
}

#[derive(Clone, Copy)]
enum Bound {
    Column(usize),
    Constant(Option<i64>),
}

impl Bound {
    fn resolve(self, row: &BindingRow) -> Option<i64> {
        match self {
            Self::Column(index) => decode_entity(&row[index]),
            Self::Constant(id) => id,
        }
    }
}

impl ExecPattern for AdjacencyPattern {
    fn id(&self) -> PatternId {
        self.id
    }

    fn variables(&self) -> &[Variable] {
        &self.variables
    }

    fn count(
        &self,
        input: &BindingBag,
        added: &[Variable],
        proposals: &mut [Proposal],
    ) -> Result<()> {
        ensure!(
            proposals.len() == input.rows.len(),
            "Triple pattern {} received {} proposals for {} input rows",
            self.id,
            proposals.len(),
            input.rows.len()
        );
        if input.rows.is_empty() {
            return Ok(());
        }
        let position = self.proposed_position(input, added)?;
        let csr = self.csr_for(position);
        for (key, proposal) in self.row_keys(input, position)?.into_iter().zip(proposals) {
            let count = match key {
                RowKey::Key(key) => csr.nnz(key),
                RowKey::All => self.keys_for(position).keys().len(),
                RowKey::Missing => 0,
            };
            proposal.consider(self.id, count);
        }
        Ok(())
    }

    fn candidate_sets<'a>(
        &'a self,
        input: &BindingBag,
        added: &[Variable],
    ) -> Result<Option<Vec<&'a [i64]>>> {
        let position = self.proposed_position(input, added)?;
        let csr = self.csr_for(position);
        let sets = self
            .row_keys(input, position)?
            .into_iter()
            .map(|key| match key {
                RowKey::Key(key) => csr.row(key),
                RowKey::All => self.keys_for(position).keys(),
                RowKey::Missing => &[][..],
            })
            .collect();
        Ok(Some(sets))
    }

    fn join(
        &self,
        input: &BindingBag,
        added: &[Variable],
        target_variables: &[Variable],
    ) -> Result<BindingBag> {
        if added.is_empty() {
            let res = self.validate(input)?;
            ensure!(
                target_variables == res.variables,
                "Triple pattern target_layout {:?} doesn't match the computed layout {:?}",
                target_variables,
                res.variables
            );
            Ok(res)
        } else {
            self.propose(input, added)?.reorder(target_variables)
        }
    }
}

#[cfg(test)]
mod tests {
    use edn::kw;
    use edn::Keyword;

    use crate::ops::{DataType, EntityRef, TxOp};
    use crate::{Database, Node, QueryNode, SubmitNode, TransactionResult};

    fn schema_attr(ident: Keyword, value_type: &str, cardinality: &str) -> TxOp {
        TxOp::put([
            (kw!(:db/ident), DataType::Keyword(ident)),
            (
                kw!(:db/valueType),
                DataType::Keyword(Keyword::namespaced("db.type", value_type)),
            ),
            (
                kw!(:db/cardinality),
                DataType::Keyword(Keyword::namespaced("db.cardinality", cardinality)),
            ),
        ])
    }

    async fn commit(node: &Node<crate::memory_log::MemoryLog>, ops: Vec<TxOp>) {
        match node.execute_tx(ops).await.unwrap() {
            TransactionResult::TxCommitted(_) => {}
            TransactionResult::TxAborted(_, err) => panic!("tx aborted: {err}"),
        }
    }

    // Deterministic small graph: edge i -> j when (i * 7 + j * 13) % 5 == 0, plus a retraction.
    async fn graph_node() -> Node<crate::memory_log::MemoryLog> {
        let node = Node::memory_node().await;
        commit(
            &node,
            vec![
                schema_attr(kw!(:g/id), "long", "one"),
                schema_attr(kw!(:g/to), "ref", "many"),
                schema_attr(kw!(:g/weight), "long", "one"),
            ],
        )
        .await;
        let n = 40i64;
        commit(
            &node,
            (0..n)
                .map(|i| {
                    TxOp::put([
                        (kw!(:g/id), DataType::Long(i)),
                        (kw!(:g/weight), DataType::Long(i * 37 % 100)),
                    ])
                })
                .collect(),
        )
        .await;
        let rows = node
            .db()
            .await
            .unwrap()
            .query("[:find ?id ?e :where [?e :g/id ?id]]")
            .await
            .unwrap();
        let mut eid = vec![0i64; n as usize];
        for row in rows {
            let [DataType::Long(id), DataType::Long(e)] = row.as_slice() else {
                panic!("unexpected row {row:?}");
            };
            eid[*id as usize] = *e;
        }
        let mut edges = Vec::new();
        for i in 0..n {
            for j in 0..n {
                if i != j && (i * 7 + j * 13) % 5 == 0 {
                    edges.push(TxOp::Add {
                        entity: EntityRef::Id(eid[i as usize]),
                        attribute: kw!(:g/to),
                        value: DataType::Long(eid[j as usize]),
                    });
                }
            }
        }
        commit(&node, edges).await;
        // A retracted edge must disappear from the matrix too.
        commit(
            &node,
            vec![TxOp::Retract {
                entity: EntityRef::Id(eid[0]),
                attribute: kw!(:g/to),
                value: DataType::Long(eid[5]),
            }],
        )
        .await;
        node
    }

    fn sorted(rows: Vec<Vec<DataType>>) -> Vec<String> {
        let mut rows: Vec<String> = rows.into_iter().map(|row| format!("{row:?}")).collect();
        rows.sort();
        rows
    }

    const QUERIES: &[&str] = &[
        "[:find ?a ?b :where [?a :g/to ?b]]",
        "[:find ?a ?b ?c :where [?a :g/to ?b] [?b :g/to ?c] [?a :g/to ?c]]",
        "[:find ?a ?c :where [?a :g/to ?b] [?b :g/to ?c]]",
        "[:find (count ?c) :where [?a :g/to ?b] [?b :g/to ?c]]",
        "[:find ?a (count ?b) :where [?a :g/to ?b]]",
        "[:find ?b (count ?a) :where [?a :g/to ?b]]",
        "[:find ?b :where [?a :g/id 3] [?a :g/to ?b]]",
        "[:find ?a :where [?b :g/id 3] [?a :g/to ?b]]",
        "[:find ?a ?b :where [?a :g/to ?b] [?b :g/weight ?w] [(> ?w 50)]]",
        "[:find ?a :where [?a :g/id ?i] [?b :g/id 3] (not [?a :g/to ?b])]",
        // Both sides constant: exercises the validate path with no proposals.
        "[:find ?a ?b :where [?a :g/id 0] [?b :g/id 10] [?a :g/to ?b]]",
    ];

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn adjacency_path_matches_iterator_path() {
        let node = graph_node().await;
        let on = node.db_with_adjacency(true).await.unwrap();
        let off = node.db_with_adjacency(false).await.unwrap();
        for query in QUERIES {
            let expected = off.query(*query).await.unwrap();
            let actual = on.query(*query).await.unwrap();
            assert!(!expected.is_empty(), "{query} returned nothing");
            assert_eq!(sorted(actual), sorted(expected), "{query}");
        }

        // Matrices are built inside the blocking query task, so by now this is a cache hit.
        let to = on.ident_map()[&kw!(:g/to)];
        let id = on.ident_map()[&kw!(:g/id)];
        assert!(on.adjacency(id).unwrap().is_none());
        assert!(off.adjacency(to).unwrap().is_none());
        let matrix = on.adjacency(to).unwrap().unwrap();
        let rows = on
            .query("[:find ?a ?b :where [?a :g/id 0] [?b :g/id 5]]")
            .await
            .unwrap();
        let [DataType::Long(a), DataType::Long(b)] = rows[0].as_slice() else {
            panic!("unexpected row");
        };
        assert!(!matrix.out.contains(*a, *b));
        node.close().await.unwrap();
    }
}
