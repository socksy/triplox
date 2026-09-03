use std::collections::HashMap;

use anyhow::Result;
use edn::kw;
use slatedb::Db;

use crate::codec::{self, Encode};
use crate::ops::DataType;
use crate::partition::tx_eid_from_tx_id;
use crate::query::binding_bag::BindingBag;
use crate::query::engine::GenericJoinEngine;
use crate::query::plan::build_logical_plan;
use crate::query::test_support::db_at_tx_id;
use crate::query::vectorized::engine::BatchedJoinEngine;
use crate::slate::in_memory_slate;

const TO: i64 = 100;
const WEIGHT: i64 = 101;

async fn insert(slate: &Db, attribute: i64, entity: i64, value: &DataType, tx_id: i64) {
    let tx_eid = codec::encode_i64_bytes(tx_eid_from_tx_id(tx_id));
    let attribute = codec::encode_i64_bytes(attribute);
    let entity = DataType::Long(entity).encode();
    let value = value.encode();

    for (prefix, first, second) in [(codec::AEV, &entity, &value), (codec::AVE, &value, &entity)] {
        let mut key = vec![prefix];
        key.extend_from_slice(&attribute);
        key.extend_from_slice(first);
        key.extend_from_slice(second);
        key.extend_from_slice(&tx_eid);
        key.push(codec::ADD);
        slate.put(&key, b"").await.expect("put");
    }
    for (prefix, component) in [(codec::AE, &entity), (codec::AV, &value)] {
        let mut key = vec![prefix];
        key.extend_from_slice(&attribute);
        key.extend_from_slice(component);
        slate.put(&key, b"").await.expect("put");
    }
}

// Six vertices, a handful of triangles, and weights so predicates have something to filter.
async fn seed(slate: &Db) {
    let edges = [
        (1, 2),
        (1, 3),
        (1, 4),
        (2, 3),
        (2, 4),
        (3, 4),
        (4, 5),
        (5, 1),
        (5, 6),
        (6, 1),
    ];
    for (from, to) in edges {
        insert(slate, TO, from, &DataType::Long(to), 10).await;
    }
    for entity in 1..=6 {
        // Repeating weights let a pattern bind the same value on both endpoints of an edge.
        insert(
            slate,
            WEIGHT,
            entity,
            &DataType::Long((entity % 3) * 20 + 5),
            10,
        )
        .await;
    }
}

const QUERIES: &[&str] = &[
    "[:find ?a ?b :where [?a :g/to ?b]]",
    "[:find ?a ?b ?c :where [?a :g/to ?b] [?b :g/to ?c] [?a :g/to ?c]]",
    "[:find ?a ?b ?c :where [?a :g/to ?b] [?b :g/to ?c]]",
    "[:find ?a (count ?b) :where [?a :g/to ?b]]",
    "[:find (sum ?w) :where [?e :g/weight ?w]]",
    "[:find ?e :where [?e :g/weight ?w] [(> ?w 25)]]",
    "[:find ?a ?b :where [?a :g/to ?b] [?b :g/weight ?w] [(> ?w 25)]]",
    "[:find ?e :where [?e :g/weight 25]]",
    "[:find ?b :where [?a :g/weight 25] [?a :g/to ?b]]",
    "[:find ?a ?b ?w :where [?a :g/to ?b] [?a :g/weight ?w] [?b :g/weight ?w]]",
];

#[test]
fn batched_engine_matches_the_row_engine() -> Result<()> {
    let runtime = tokio::runtime::Runtime::new()?;
    let components = runtime.block_on(in_memory_slate());
    runtime.block_on(seed(components.db.as_ref()));
    let idents = HashMap::from([(kw!(:g/to), TO), (kw!(:g/weight), WEIGHT)]);

    for query in QUERIES {
        let parsed = edn::parse::parse_query(query).expect("parse");
        let logical = build_logical_plan(&parsed, &[])?;
        let db = db_at_tx_id(&components, runtime.handle(), idents.clone(), 20);
        let stages = logical.materialize(db, None)?;

        assert!(
            BatchedJoinEngine::supports(&stages),
            "{query} should run on the batched engine"
        );
        let rows = GenericJoinEngine::execute(&stages, BindingBag::unit())?;
        let batched = BatchedJoinEngine::execute(&stages)?;

        assert_eq!(batched.variables, rows.variables, "layout for {query}");
        assert_eq!(batched.rows, rows.rows, "rows for {query}");
        assert!(!rows.rows.is_empty(), "{query} produced no rows to compare");
    }
    Ok(())
}

#[test]
fn unsupported_patterns_are_not_claimed_by_the_batched_engine() -> Result<()> {
    let runtime = tokio::runtime::Runtime::new()?;
    let components = runtime.block_on(in_memory_slate());
    let parsed =
        edn::parse::parse_query("[:find ?x :in [?x ...] :where (not (or [(= ?x 2)] [(= ?x 3)]))]")
            .expect("parse");
    let arguments = [crate::ops::QueryArg::Collection(vec![
        DataType::Long(1),
        DataType::Long(2),
    ])];
    let logical = build_logical_plan(&parsed, &arguments)?;
    let db = db_at_tx_id(&components, runtime.handle(), HashMap::new(), 20);
    let stages = logical.materialize(db, None)?;

    assert!(!BatchedJoinEngine::supports(&stages));
    Ok(())
}
