//! Sparse matrix algebra for a small set of aggregate-only query shapes.
//!
//! Instead of enumerating the join relation and counting the rows, these shapes are
//! answered with vector/matrix products over the CSR adjacency matrices built by
//! `crate::query::adjacency`. Everything else falls back to the generic join.
//!
//! Only shapes whose answer is provably identical to the generic-join answer are
//! recognised; `try_execute` returns `Ok(None)` for anything else.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::Arc;

use anyhow::{Error, Result};
use edn::query::{
    ContainsVariables, Element, FindSpec, Limit, ParsedQuery, Pattern, PatternNonValuePlace,
    PatternValuePlace, Variable, WhereClause,
};
use slatedb::{DbMetadataOps, DbReadOps};

use crate::db_value::DB;
use crate::ops::{DataType, QueryArg};
use crate::query::adjacency::AdjMatrix;
use crate::query::bitset::{and_popcount, popcount, BitMatrix};
use crate::query::{clause_mentioned_variables, execute_query, AggregateFunc, QueryResult};

fn log_enabled() -> bool {
    std::env::var_os("TRIPLOX_MATRIX_ALGEBRA_LOG").is_some()
}

/// One `[?from :attr ?to]` clause whose attribute has an adjacency matrix.
struct Edge {
    from: Variable,
    to: Variable,
    matrix: Arc<AdjMatrix>,
}

/// The single find element, which must be one aggregate over one variable.
struct Agg {
    func: AggregateFunc,
    var: Variable,
}

/// Try to answer `query` with matrix algebra. `Ok(None)` means "not a shape we handle".
pub(crate) fn try_execute<D, M>(
    query: &ParsedQuery,
    args: &[QueryArg],
    db: &Arc<DB<D, M>>,
) -> Result<Option<QueryResult>>
where
    D: DbReadOps + Send + Sync + 'static,
    M: DbMetadataOps + Send + Sync + 'static,
{
    if !db.matrix_algebra() {
        return Ok(None);
    }
    // Anything that reshapes or filters the single output row is left to the normal path.
    if !args.is_empty()
        || !query.in_bindings.is_empty()
        || !query.with.is_empty()
        || query.order.is_some()
        || query.limit != Limit::None
    {
        return Ok(None);
    }
    let Some(agg) = single_aggregate(&query.find_spec) else {
        return Ok(None);
    };
    if !matches!(
        agg.func,
        AggregateFunc::Count | AggregateFunc::CountDistinct
    ) {
        return Ok(None);
    }

    let Some((edges, rest)) = split_edges(&query.where_clauses, db)? else {
        return Ok(None);
    };
    if edges.is_empty() {
        return Ok(None);
    }

    if let Some(count) = try_triangle(&edges, &rest, &agg)? {
        return Ok(Some(one_count(count)?));
    }
    if let Some(count) = try_chain(&edges, &rest, &agg, query, db)? {
        return Ok(Some(one_count(count)?));
    }
    Ok(None)
}

fn one_count(count: u128) -> Result<QueryResult> {
    let count = i64::try_from(count).map_err(|_| {
        anyhow::anyhow!("matrix algebra count {count} does not fit in a 64-bit integer")
    })?;
    Ok(vec![vec![DataType::Long(count)]])
}

fn single_aggregate(find: &FindSpec) -> Option<Agg> {
    let FindSpec::FindRel(elements) = find else {
        return None;
    };
    let [Element::Aggregate(aggregate)] = elements.as_slice() else {
        return None;
    };
    let [edn::query::FnArg::Variable(var)] = aggregate.args.as_slice() else {
        return None;
    };
    let name = aggregate.func.0.to_string();
    let func = match name.as_str() {
        "count" => AggregateFunc::Count,
        "count-distinct" => AggregateFunc::CountDistinct,
        _ => return None,
    };
    Some(Agg {
        func,
        var: var.clone(),
    })
}

/// Split the where clauses into variable-to-variable ref patterns backed by a matrix and
/// everything else. `Ok(None)` when a clause cannot be classified at all.
fn split_edges<'a, D, M>(
    clauses: &'a [WhereClause],
    db: &DB<D, M>,
) -> Result<Option<(Vec<Edge>, Vec<&'a WhereClause>)>>
where
    D: DbReadOps + Send + Sync + 'static,
    M: DbMetadataOps + Send + Sync + 'static,
{
    let mut edges = Vec::new();
    let mut rest = Vec::new();
    for clause in clauses {
        match edge_of(clause, db)? {
            Some(edge) => edges.push(edge),
            None => rest.push(clause),
        }
    }
    Ok(Some((edges, rest)))
}

fn edge_of<D, M>(clause: &WhereClause, db: &DB<D, M>) -> Result<Option<Edge>>
where
    D: DbReadOps + Send + Sync + 'static,
    M: DbMetadataOps + Send + Sync + 'static,
{
    let WhereClause::Pattern(Pattern {
        source,
        entity,
        attribute,
        value,
        tx,
    }) = clause
    else {
        return Ok(None);
    };
    if source.is_some() || !matches!(tx, PatternNonValuePlace::Placeholder) {
        return Ok(None);
    }
    let (PatternNonValuePlace::Variable(from), PatternValuePlace::Variable(to)) = (entity, value)
    else {
        return Ok(None);
    };
    if from == to {
        return Ok(None);
    }
    let Ok(attribute) = super::resolve_attribute_from_pattern(attribute, db.ident_map()) else {
        return Ok(None);
    };
    Ok(db.adjacency(attribute)?.map(|matrix| Edge {
        from: from.clone(),
        to: to.clone(),
        matrix,
    }))
}

// ---------------------------------------------------------------------------
// Chain of hops
// ---------------------------------------------------------------------------

/// `[?x0 :a1 ?x1] ... [?x(k-1) :ak ?xk]` with the inner variables used nowhere else,
/// optionally anchored by other clauses that mention only one endpoint.
fn try_chain<D, M>(
    edges: &[Edge],
    rest: &[&WhereClause],
    agg: &Agg,
    query: &ParsedQuery,
    db: &Arc<DB<D, M>>,
) -> Result<Option<u128>>
where
    D: DbReadOps + Send + Sync + 'static,
    M: DbMetadataOps + Send + Sync + 'static,
{
    let Some(order) = chain_order(edges) else {
        return Ok(None);
    };
    // Chain variables x0..xk in path order.
    let mut vars: Vec<&Variable> = vec![&edges[order[0]].from];
    for &index in &order {
        vars.push(&edges[index].to);
    }
    let chain: HashSet<&Variable> = vars.iter().copied().collect();
    if chain.len() != vars.len() {
        return Ok(None);
    }

    // The anchor is the only chain variable the remaining clauses may mention, and they
    // may mention nothing else at all, so the join relation stays anchor-set x paths.
    let mut mentioned: BTreeSet<Variable> = BTreeSet::new();
    for clause in rest {
        mentioned.extend(clause_mentioned_variables(clause));
    }
    let anchor: Option<&Variable> = if rest.is_empty() {
        None
    } else {
        let [only] = mentioned.iter().collect::<Vec<_>>()[..] else {
            return Ok(None);
        };
        let position = vars.iter().position(|v| *v == only);
        match position {
            Some(0) => Some(vars[0]),
            Some(p) if p == vars.len() - 1 => Some(vars[p]),
            _ => return Ok(None),
        }
    };

    let Some(agg_position) = vars.iter().position(|v| **v == agg.var) else {
        return Ok(None);
    };

    // Walk the chain from the anchored end so the anchor set seeds the first vector.
    let backwards = anchor.is_some_and(|a| a == *vars.last().unwrap());
    let hops: Vec<&AdjMatrix> = order.iter().map(|&i| edges[i].matrix.as_ref()).collect();
    let seed = match anchor {
        None => None,
        // A sub-query that the generic engine cannot answer on its own (an unbound `not`,
        // say) means the split was not sound, so hand the whole query back.
        Some(var) => match anchor_set(query, var, rest, db) {
            Ok(seed) => Some(seed),
            Err(_) => return Ok(None),
        },
    };
    let agg_position = if backwards {
        vars.len() - 1 - agg_position
    } else {
        agg_position
    };

    // Every hop over one attribute means one dense boolean matrix can serve them all.
    let bits = uniform(&hops).and_then(|matrix| matrix.bits(backwards));
    let count = match agg.func {
        AggregateFunc::Count => chain_paths(&hops, seed.as_deref(), backwards, bits),
        AggregateFunc::CountDistinct if agg_position == hops.len() => match bits {
            Some(bits) => chain_reachable_bits(bits, hops.len(), seed.as_deref()) as u128,
            None => chain_reachable(&hops, seed.as_deref(), backwards) as u128,
        },
        AggregateFunc::CountDistinct if agg_position == 0 => match bits {
            Some(bits) => chain_sources_bits(bits, hops.len(), seed.as_deref()) as u128,
            None => chain_sources(&hops, seed.as_deref(), backwards) as u128,
        },
        _ => return Ok(None),
    };
    if log_enabled() {
        eprintln!(
            "matrix algebra: chain hops={} anchored={} backwards={} agg={} bitset={} count={count}",
            hops.len(),
            seed.is_some(),
            backwards,
            agg.func,
            bits.is_some()
        );
    }
    Ok(Some(count))
}

/// Indices of `edges` in path order, or None when they do not form one simple path.
fn chain_order(edges: &[Edge]) -> Option<Vec<usize>> {
    let mut out: HashMap<&Variable, usize> = HashMap::new();
    let mut in_degree: HashMap<&Variable, usize> = HashMap::new();
    for (index, edge) in edges.iter().enumerate() {
        if out.insert(&edge.from, index).is_some() {
            return None;
        }
        *in_degree.entry(&edge.to).or_default() += 1;
    }
    if in_degree.values().any(|d| *d > 1) {
        return None;
    }
    let start = edges
        .iter()
        .map(|edge| &edge.from)
        .find(|from| !in_degree.contains_key(*from))?;
    let mut order = Vec::with_capacity(edges.len());
    let mut current = start;
    while let Some(&index) = out.get(current) {
        order.push(index);
        current = &edges[index].to;
        if order.len() > edges.len() {
            return None;
        }
    }
    (order.len() == edges.len()).then_some(order)
}

/// The distinct entity ids the anchoring clauses bind the anchor variable to.
fn anchor_set<D, M>(
    query: &ParsedQuery,
    var: &Variable,
    rest: &[&WhereClause],
    db: &Arc<DB<D, M>>,
) -> Result<Vec<i64>>
where
    D: DbReadOps + Send + Sync + 'static,
    M: DbMetadataOps + Send + Sync + 'static,
{
    let sub = ParsedQuery {
        find_spec: FindSpec::FindRel(vec![Element::Variable(var.clone())]),
        default_source: query.default_source.clone(),
        with: Vec::new(),
        in_bindings: Vec::new(),
        in_sources: BTreeSet::new(),
        limit: Limit::None,
        where_clauses: rest.iter().map(|clause| (*clause).clone()).collect(),
        order: None,
    };
    let rows = execute_query(&sub, &[], Arc::clone(db))?;
    rows.into_iter()
        .map(|row| match row.as_slice() {
            [DataType::Long(id)] => Ok(*id),
            other => Err(anyhow::anyhow!(
                "anchor variable {var} is not an entity id: {other:?}"
            )),
        })
        .collect::<Result<Vec<i64>, Error>>()
}

/// The single matrix behind every hop, when they all share one.
fn uniform<'a>(hops: &[&'a AdjMatrix]) -> Option<&'a AdjMatrix> {
    let first = *hops.first()?;
    hops.iter()
        .all(|matrix| std::ptr::eq(*matrix, first))
        .then_some(first)
}

fn hop_row<'a>(matrix: &'a AdjMatrix, node: i64, backwards: bool) -> &'a [i64] {
    if backwards {
        matrix.inn.row(node)
    } else {
        matrix.out.row(node)
    }
}

fn hop_keys(matrix: &AdjMatrix, backwards: bool) -> &[i64] {
    if backwards {
        matrix.inn.keys()
    } else {
        matrix.out.keys()
    }
}

/// Number of distinct paths, i.e. 1ᵀ A1 A2 ... Ak 1 over the integer semiring, with the
/// left vector restricted to `seed` when the chain is anchored.
fn chain_paths(
    hops: &[&AdjMatrix],
    seed: Option<&[i64]>,
    backwards: bool,
    bits: Option<&BitMatrix>,
) -> u128 {
    if let Some(bits) = bits {
        return chain_paths_dense(hops[0], bits, hops.len(), seed, backwards);
    }
    let hops: Vec<&&AdjMatrix> = if backwards {
        hops.iter().rev().collect()
    } else {
        hops.iter().collect()
    };
    let mut current: HashMap<i64, u128> = match seed {
        Some(seed) => seed.iter().map(|node| (*node, 1u128)).collect(),
        None => hop_keys(hops[0], backwards)
            .iter()
            .map(|node| (*node, 1u128))
            .collect(),
    };
    for matrix in &hops {
        let mut next: HashMap<i64, u128> = HashMap::with_capacity(current.len());
        for (node, weight) in &current {
            for target in hop_row(matrix, *node, backwards) {
                *next.entry(*target).or_default() += weight;
            }
        }
        current = next;
    }
    current.values().sum()
}

/// Size of the set of chain endpoints, i.e. the boolean-semiring product.
fn chain_reachable(hops: &[&AdjMatrix], seed: Option<&[i64]>, backwards: bool) -> usize {
    let hops: Vec<&&AdjMatrix> = if backwards {
        hops.iter().rev().collect()
    } else {
        hops.iter().collect()
    };
    let mut current: Vec<i64> = match seed {
        Some(seed) => seed.to_vec(),
        None => hop_keys(hops[0], backwards).to_vec(),
    };
    for matrix in &hops {
        let mut next: HashSet<i64> = HashSet::new();
        for node in &current {
            next.extend(hop_row(matrix, *node, backwards));
        }
        current = next.into_iter().collect();
    }
    current.len()
}

/// Size of the set of chain starts that have at least one complete path.
fn chain_sources(hops: &[&AdjMatrix], seed: Option<&[i64]>, backwards: bool) -> usize {
    let ordered: Vec<&&AdjMatrix> = if backwards {
        hops.iter().rev().collect()
    } else {
        hops.iter().collect()
    };
    // Nodes from which a suffix of the chain can still be completed, walked back to front.
    let mut live: Option<HashSet<i64>> = None;
    for matrix in ordered.iter().rev() {
        let mut next: HashSet<i64> = HashSet::new();
        for node in hop_keys(matrix, backwards) {
            let reaches = match &live {
                None => !hop_row(matrix, *node, backwards).is_empty(),
                Some(live) => hop_row(matrix, *node, backwards)
                    .iter()
                    .any(|target| live.contains(target)),
            };
            if reaches {
                next.insert(*node);
            }
        }
        live = Some(next);
    }
    let live = live.unwrap_or_default();
    match seed {
        Some(seed) => seed.iter().filter(|node| live.contains(node)).count(),
        None => live.len(),
    }
}

// ---------------------------------------------------------------------------
// Triangles
// ---------------------------------------------------------------------------

/// `[?a :a1 ?b] [?b :a2 ?c] [?a :a3 ?c]` counted as the masked product A1(A2 ∘ A3ᵀ).
fn try_triangle(edges: &[Edge], rest: &[&WhereClause], agg: &Agg) -> Result<Option<u128>> {
    if !rest.is_empty() || edges.len() != 3 || agg.func != AggregateFunc::Count {
        return Ok(None);
    }
    let mut out: HashMap<&Variable, Vec<usize>> = HashMap::new();
    for (index, edge) in edges.iter().enumerate() {
        out.entry(&edge.from).or_default().push(index);
    }
    let Some((apex, pair)) = out.iter().find(|(_, indices)| indices.len() == 2) else {
        return Ok(None);
    };
    let (first, second) = (edges[pair[0]].to.clone(), edges[pair[1]].to.clone());
    // The remaining edge closes the triangle between the apex's two targets.
    let Some(middle) = edges
        .iter()
        .find(|edge| edge.from != **apex)
        .filter(|edge| {
            (edge.from == first && edge.to == second) || (edge.from == second && edge.to == first)
        })
    else {
        return Ok(None);
    };
    let vars: HashSet<&Variable> = [*apex, &first, &second].into_iter().collect();
    if vars.len() != 3 || !vars.contains(&agg.var) {
        return Ok(None);
    }
    // apex -> mid -> tip and apex -> tip.
    let (mid_edge, tip_edge) = if middle.from == first {
        (&edges[pair[0]], &edges[pair[1]])
    } else {
        (&edges[pair[1]], &edges[pair[0]])
    };

    // One dense matrix can serve all three edges only when they are the same attribute.
    let shared = Arc::ptr_eq(&mid_edge.matrix, &tip_edge.matrix)
        && Arc::ptr_eq(&mid_edge.matrix, &middle.matrix);
    let bits = shared.then(|| mid_edge.matrix.bits(false)).flatten();
    if let Some(bits) = bits {
        let count = triangle_bits(&mid_edge.matrix, bits);
        if log_enabled() {
            eprintln!("matrix algebra: triangle bitset=true count={count}");
        }
        return Ok(Some(count));
    }

    let mut count: u128 = 0;
    for apex_node in mid_edge.matrix.out.keys() {
        let tips = tip_edge.matrix.out.row(*apex_node);
        if tips.is_empty() {
            continue;
        }
        for mid in mid_edge.matrix.out.row(*apex_node) {
            count += intersection_len(middle.matrix.out.row(*mid), tips) as u128;
        }
    }
    if log_enabled() {
        eprintln!("matrix algebra: triangle bitset=false count={count}");
    }
    Ok(Some(count))
}

/// Number of ids present in both sorted slices.
fn intersection_len(left: &[i64], right: &[i64]) -> usize {
    let (mut i, mut j, mut hits) = (0, 0, 0);
    while i < left.len() && j < right.len() {
        match left[i].cmp(&right[j]) {
            std::cmp::Ordering::Less => i += 1,
            std::cmp::Ordering::Greater => j += 1,
            std::cmp::Ordering::Equal => {
                hits += 1;
                i += 1;
                j += 1;
            }
        }
    }
    hits
}

#[cfg(test)]
mod tests {
    use super::*;

    use edn::kw;
    use edn::Keyword;

    use crate::memory_log::MemoryLog;
    use crate::ops::{EntityRef, TxOp};
    use crate::{Database, Node, QueryNode, SubmitNode, TransactionResult};
    use triplox_client::transaction::TxKey;

    #[test]
    fn counts_sorted_intersections() {
        assert_eq!(intersection_len(&[1, 2, 3, 7], &[2, 3, 9]), 2);
        assert_eq!(intersection_len(&[], &[1]), 0);
        assert_eq!(intersection_len(&[1, 2], &[3]), 0);
    }

    /// Queries the algebra path must answer, and which must match the generic join exactly.
    const ALGEBRA_QUERIES: &[&str] = &[
        "[:find (count ?b) :where [?a :g/to ?b]]",
        "[:find (count ?c) :where [?a :g/to ?b] [?b :g/to ?c]]",
        "[:find (count ?a) :where [?a :g/to ?b] [?b :g/to ?c]]",
        "[:find (count ?d) :where [?a :g/to ?b] [?b :g/to ?c] [?c :g/to ?d]]",
        "[:find (count-distinct ?b) :where [?a :g/to ?b]]",
        "[:find (count-distinct ?a) :where [?a :g/to ?b]]",
        "[:find (count-distinct ?c) :where [?a :g/to ?b] [?b :g/to ?c]]",
        "[:find (count-distinct ?a) :where [?a :g/to ?b] [?b :g/to ?c]]",
        "[:find (count-distinct ?d) :where [?a :g/to ?b] [?b :g/to ?c] [?c :g/to ?d]]",
        "[:find (count-distinct ?a) :where [?a :g/to ?b] [?b :g/to ?c] [?c :g/to ?d]]",
        // Bound start.
        "[:find (count ?c) :where [?a :g/id 3] [?a :g/to ?b] [?b :g/to ?c]]",
        "[:find (count-distinct ?c) :where [?a :g/id 3] [?a :g/to ?b] [?b :g/to ?c]]",
        "[:find (count-distinct ?a) :where [?a :g/id 3] [?a :g/to ?b] [?b :g/to ?c]]",
        // Bound end.
        "[:find (count ?a) :where [?c :g/id 3] [?a :g/to ?b] [?b :g/to ?c]]",
        "[:find (count-distinct ?a) :where [?c :g/id 3] [?a :g/to ?b] [?b :g/to ?c]]",
        // Triangles.
        "[:find (count ?a) :where [?a :g/to ?b] [?b :g/to ?c] [?a :g/to ?c]]",
        "[:find (count ?c) :where [?a :g/to ?b] [?b :g/to ?c] [?a :g/to ?c]]",
    ];

    /// Queries the algebra path must decline, still answered by the generic join.
    const FALLBACK_QUERIES: &[&str] = &[
        "[:find ?a ?b :where [?a :g/to ?b]]",
        "[:find ?a ?b ?c :where [?a :g/to ?b] [?b :g/to ?c] [?a :g/to ?c]]",
        "[:find ?a ?c :where [?a :g/to ?b] [?b :g/to ?c]]",
        "[:find ?a (count ?b) :where [?a :g/to ?b]]",
        "[:find ?b (count ?a) :where [?a :g/to ?b]]",
        "[:find ?b :where [?a :g/id 3] [?a :g/to ?b]]",
        "[:find ?a :where [?b :g/id 3] [?a :g/to ?b]]",
        "[:find ?a ?b :where [?a :g/to ?b] [?b :g/weight ?w] [(> ?w 50)]]",
        "[:find (sum ?w) :where [?e :g/weight ?w]]",
        "[:find (count ?e) :where [?e :g/weight ?w]]",
        // The extra clauses mention a second variable, so the relation is not a bare chain.
        "[:find (count ?c) :where [?a :g/to ?b] [?b :g/to ?c] [?c :g/weight ?w] [(> ?w 50)]]",
        // Two edges out of one variable: a fork, not a chain, and not a triangle.
        "[:find (count ?c) :where [?a :g/to ?b] [?a :g/to ?c]]",
        // count-distinct on an interior variable is not handled.
        "[:find (count-distinct ?b) :where [?a :g/to ?b] [?b :g/to ?c]]",
        // ORDER BY / LIMIT keep the whole query on the normal path.
        "[:find (count ?c) :where [?a :g/to ?b] [?b :g/to ?c] :limit 1]",
    ];

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

    async fn commit(node: &Node<MemoryLog>, ops: Vec<TxOp>) -> TxKey {
        match node.execute_tx(ops).await.unwrap() {
            TransactionResult::TxCommitted(key) => key,
            TransactionResult::TxAborted(_, err) => panic!("tx aborted: {err}"),
        }
    }

    struct XorShift(u64);

    impl XorShift {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }
    }

    /// A random directed graph plus some retractions; returns the node and the basis
    /// before the retractions so as-of queries have something to differ on.
    async fn graph_node(nodes: i64, seed: u64) -> (Node<MemoryLog>, TxKey) {
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
        commit(
            &node,
            (0..nodes)
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
        let mut eid = vec![0i64; nodes as usize];
        for row in rows {
            let [DataType::Long(id), DataType::Long(entity)] = row.as_slice() else {
                panic!("unexpected row {row:?}");
            };
            eid[*id as usize] = *entity;
        }
        let mut rng = XorShift(seed);
        let mut edges = Vec::new();
        let mut pairs = Vec::new();
        for i in 0..nodes {
            for j in 0..nodes {
                if i != j && rng.next() % 5 == 0 {
                    pairs.push((i, j));
                    edges.push(TxOp::Add {
                        entity: EntityRef::Id(eid[i as usize]),
                        attribute: kw!(:g/to),
                        value: DataType::Long(eid[j as usize]),
                    });
                }
            }
        }
        let basis = commit(&node, edges).await;
        let retractions: Vec<TxOp> = pairs
            .iter()
            .step_by(7)
            .map(|(i, j)| TxOp::Retract {
                entity: EntityRef::Id(eid[*i as usize]),
                attribute: kw!(:g/to),
                value: DataType::Long(eid[*j as usize]),
            })
            .collect();
        commit(&node, retractions).await;
        (node, basis)
    }

    fn sorted(rows: QueryResult) -> Vec<String> {
        let mut rows: Vec<String> = rows.into_iter().map(|row| format!("{row:?}")).collect();
        rows.sort();
        rows
    }

    /// Whether the algebra path claims this query, checked without running the engine twice.
    fn handled<D, M>(query: &str, db: &Arc<DB<D, M>>) -> bool
    where
        D: DbReadOps + Send + Sync + 'static,
        M: DbMetadataOps + Send + Sync + 'static,
    {
        let parsed = edn::parse::parse_query(query).expect("parse");
        try_execute(&parsed, &[], db).expect("algebra").is_some()
    }

    async fn assert_matches(
        baseline: &DB,
        adjacency: &DB,
        algebra: &Arc<DB>,
        queries: &[&str],
        expect_handled: bool,
    ) {
        for query in queries {
            // Matrix builds block on SlateDB, so this cannot run on the async worker.
            let (owned, db) = (query.to_string(), Arc::clone(algebra));
            let claimed = tokio::task::spawn_blocking(move || handled(&owned, &db))
                .await
                .unwrap();
            assert_eq!(
                claimed, expect_handled,
                "algebra path claimed {query} unexpectedly"
            );
            let expected = baseline.query(*query).await.unwrap();
            assert_eq!(
                sorted(adjacency.query(*query).await.unwrap()),
                sorted(expected.clone()),
                "adjacency path differs on {query}"
            );
            assert_eq!(
                sorted(algebra.as_ref().query(*query).await.unwrap()),
                sorted(expected),
                "algebra path differs on {query}"
            );
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn algebra_path_matches_generic_join() {
        for (nodes, seed) in [(24i64, 0x5eedu64), (31, 0xabcdef), (40, 0x1234_5678)] {
            let (node, basis) = graph_node(nodes, seed).await;
            let baseline = node.db_with_modes(false, false).await.unwrap();
            let adjacency = node.db_with_modes(true, false).await.unwrap();
            let algebra = Arc::new(node.db_with_modes(true, true).await.unwrap());
            assert_matches(&baseline, &adjacency, &algebra, ALGEBRA_QUERIES, true).await;
            assert_matches(&baseline, &adjacency, &algebra, FALLBACK_QUERIES, false).await;

            // The same comparison against an earlier basis, before the retractions.
            let baseline = node.db_as_of_with_modes(basis, false, false).await.unwrap();
            let adjacency = node.db_as_of_with_modes(basis, true, false).await.unwrap();
            let algebra = Arc::new(node.db_as_of_with_modes(basis, true, true).await.unwrap());
            assert_matches(&baseline, &adjacency, &algebra, ALGEBRA_QUERIES, true).await;

            node.close().await.unwrap();
        }
    }

    /// The dense NEON kernels and the sorted-merge kernels must agree.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn dense_and_sparse_kernels_agree() {
        let (node, _) = graph_node(40, 0x9e37).await;
        let db = node.db_with_modes(true, true).await.unwrap();
        tokio::task::spawn_blocking(move || {
            let to = db.ident_map()[&kw!(:g/to)];
            let matrix = db.adjacency(to).unwrap().unwrap();
            let hops = [matrix.as_ref(), matrix.as_ref(), matrix.as_ref()];
            let bits = matrix.bits(false).expect("dense form");

            assert_eq!(
                chain_paths(&hops, None, false, Some(bits)),
                chain_paths(&hops, None, false, None)
            );
            assert_eq!(
                chain_reachable_bits(bits, hops.len(), None),
                chain_reachable(&hops, None, false)
            );
            assert_eq!(
                chain_sources_bits(bits, hops.len(), None),
                chain_sources(&hops, None, false)
            );
        })
        .await
        .unwrap();
        node.close().await.unwrap();
    }
}

// ---------------------------------------------------------------------------
// Dense kernels
// ---------------------------------------------------------------------------

/// Path counts as a dense vector over the node index, one hop per iteration.
fn chain_paths_dense(
    matrix: &AdjMatrix,
    bits: &BitMatrix,
    hops: usize,
    seed: Option<&[i64]>,
    backwards: bool,
) -> u128 {
    let index = bits.index();
    let mut current = vec![0u128; index.len()];
    match seed {
        Some(seed) => {
            for node in seed {
                if let Some(position) = index.position(*node) {
                    current[position] = 1;
                }
            }
        }
        None => current.fill(1),
    }
    for _ in 0..hops {
        let mut next = vec![0u128; index.len()];
        for (position, weight) in current.iter().enumerate() {
            if *weight == 0 {
                continue;
            }
            for target in hop_row(matrix, index.id(position), backwards) {
                if let Some(target) = index.position(*target) {
                    next[target] += weight;
                }
            }
        }
        current = next;
    }
    current.iter().sum()
}

/// Endpoints of the chain as a bit set: `hops` boolean matrix-vector products.
fn chain_reachable_bits(bits: &BitMatrix, hops: usize, seed: Option<&[i64]>) -> usize {
    let mut current = match seed {
        Some(seed) => bits.row_of_ids(seed),
        None => bits.all_rows(),
    };
    for _ in 0..hops {
        current = bits.spread(&current);
    }
    popcount(&current) as usize
}

/// Chain starts that complete a whole path, found by walking the mask back to front.
fn chain_sources_bits(bits: &BitMatrix, hops: usize, seed: Option<&[i64]>) -> usize {
    let mut live: Option<Vec<u64>> = None;
    for _ in 0..hops {
        let mut next = bits.zero_row();
        for position in 0..bits.index().len() {
            let row = bits.row(position);
            let reaches = match &live {
                None => popcount(row) > 0,
                Some(live) => and_popcount(row, live) > 0,
            };
            if reaches {
                next[position / 64] |= 1u64 << (position % 64);
            }
        }
        live = Some(next);
    }
    let live = live.unwrap_or_else(|| bits.zero_row());
    match seed {
        Some(seed) => and_popcount(&live, &bits.row_of_ids(seed)) as usize,
        None => popcount(&live) as usize,
    }
}

/// Triangles as the masked product `sum(A2 masked by A1) . A3`, one AND+popcount per
/// (apex, mid) pair.
fn triangle_bits(matrix: &AdjMatrix, bits: &BitMatrix) -> u128 {
    let index = bits.index();
    let mut count: u128 = 0;
    for apex in 0..index.len() {
        let tips = bits.row(apex);
        if popcount(tips) == 0 {
            continue;
        }
        for mid in matrix.out.row(index.id(apex)) {
            if let Some(mid) = index.position(*mid) {
                count += u128::from(and_popcount(bits.row(mid), tips));
            }
        }
    }
    count
}
