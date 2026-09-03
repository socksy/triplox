//! Row vs columnar layout equivalence, plus a storage-size report.

use edn::kw;
use edn::Keyword;

use crate::codec;
use crate::memory_log::MemoryLog;
use crate::node::Node;
use crate::ops::{DataType, EntityRef, TxOp};
use crate::segment::{Segment, SegmentLayout};
use crate::slate::DEFAULT_SCAN_OPTIONS;
use crate::transaction::TxKey;
use triplox_client::node::{Database, QueryNode, SubmitNode};
use triplox_client::transaction::TransactionResult;

const QUERIES: &[&str] = &[
    "[:find ?a ?b :where [?a :g/to ?b]]",
    "[:find ?a ?b ?c :where [?a :g/to ?b] [?b :g/to ?c] [?a :g/to ?c]]",
    "[:find (count ?c) :where [?a :g/to ?b] [?b :g/to ?c]]",
    "[:find ?a (count ?b) :where [?a :g/to ?b]]",
    "[:find ?b :where [?a :g/id 4] [?a :g/to ?b]]",
    "[:find ?e :where [?e :g/weight ?w] [(> ?w 20)]]",
    "[:find (sum ?w) :where [?e :g/weight ?w]]",
    "[:find ?e :where [?e :g/label \"label-7\"]]",
    "[:find ?id :where [?e :g/id ?id]]",
];

fn schema_attr(ident: Keyword, value_type: &str, cardinality: &str, unique: bool) -> TxOp {
    let mut fields = vec![
        (kw!(:db/ident), DataType::Keyword(ident)),
        (
            kw!(:db/valueType),
            DataType::Keyword(Keyword::namespaced("db.type", value_type)),
        ),
        (
            kw!(:db/cardinality),
            DataType::Keyword(Keyword::namespaced("db.cardinality", cardinality)),
        ),
    ];
    if unique {
        fields.push((
            kw!(:db/unique),
            DataType::Keyword(Keyword::namespaced("db.unique", "identity")),
        ));
    }
    TxOp::put(fields)
}

async fn commit(node: &Node<MemoryLog>, ops: Vec<TxOp>) -> TxKey {
    match node.execute_tx(ops).await.expect("tx executes") {
        TransactionResult::TxCommitted(key) => key,
        TransactionResult::TxAborted(_, err) => panic!("tx aborted: {err}"),
    }
}

/// Seeded graph of `n` vertices with `fanout` deterministic out-edges each, then
/// one retraction. Returns the tx key taken before the retraction so callers can
/// query as-of it.
async fn build_graph(node: &Node<MemoryLog>, n: i64, fanout: i64) -> TxKey {
    commit(
        node,
        vec![
            schema_attr(kw!(:g/id), "long", "one", true),
            schema_attr(kw!(:g/to), "ref", "many", false),
            schema_attr(kw!(:g/label), "string", "one", false),
            schema_attr(kw!(:g/weight), "long", "one", false),
        ],
    )
    .await;

    let vertices: Vec<TxOp> = (0..n)
        .map(|i| {
            TxOp::put([
                (kw!(:g/id), DataType::Long(i)),
                (kw!(:g/label), DataType::String(format!("label-{i}"))),
                (kw!(:g/weight), DataType::Long(i % 40)),
            ])
        })
        .collect();
    for chunk in vertices.chunks(1000) {
        commit(node, chunk.to_vec()).await;
    }

    let db = node.db().await.expect("db");
    let mut eid = vec![0i64; n as usize];
    for row in db
        .query("[:find ?id ?e :where [?e :g/id ?id]]")
        .await
        .expect("ids")
    {
        let [DataType::Long(id), DataType::Long(e)] = row.as_slice() else {
            panic!("unexpected row {row:?}");
        };
        eid[*id as usize] = *e;
    }

    let mut edges = Vec::new();
    for i in 0..n {
        for k in 1..=fanout {
            let j = (i * 7 + k * 3) % n;
            if i != j {
                edges.push((i, j));
            }
        }
    }
    let edge_ops: Vec<TxOp> = edges
        .iter()
        .map(|(from, to)| TxOp::Add {
            entity: EntityRef::Id(eid[*from as usize]),
            attribute: kw!(:g/to),
            value: DataType::Long(eid[*to as usize]),
        })
        .collect();
    let mut before_retract = None;
    for chunk in edge_ops.chunks(1000) {
        before_retract = Some(commit(node, chunk.to_vec()).await);
    }
    let before_retract = before_retract.expect("at least one edge batch");

    let (from, to) = edges[0];
    commit(
        node,
        vec![TxOp::Retract {
            entity: EntityRef::Id(eid[from as usize]),
            attribute: kw!(:g/to),
            value: DataType::Long(eid[to as usize]),
        }],
    )
    .await;

    before_retract
}

async fn results(node: &Node<MemoryLog>, as_of: Option<TxKey>) -> Vec<Vec<Vec<DataType>>> {
    let db = match as_of {
        Some(key) => node.db_as_of(key).await.expect("db_as_of"),
        None => node.db().await.expect("db"),
    };
    let mut all = Vec::new();
    for q in QUERIES {
        let mut rows = db.query(*q).await.unwrap_or_else(|e| panic!("{q}: {e}"));
        rows.sort_by_key(|r| format!("{r:?}"));
        all.push(rows);
    }
    all
}

/// (keys, value bytes, key bytes) per index byte, plus decoded datom count.
async fn storage_report(node: &Node<MemoryLog>) -> Vec<(u8, u64, u64, u64, u64)> {
    let db = node.slate.db.clone();
    let mut iter = db
        .scan_with_options(.., &DEFAULT_SCAN_OPTIONS)
        .await
        .expect("scan");
    let mut per_index: std::collections::BTreeMap<u8, (u64, u64, u64, u64)> = Default::default();
    while let Some(kv) = iter.next().await.expect("next") {
        let index = kv.key[0];
        let entry = per_index.entry(index).or_default();
        entry.0 += 1;
        entry.1 += kv.key.len() as u64;
        entry.2 += kv.value.len() as u64;
        entry.3 += if crate::segment::is_segmented_index(index) && !kv.value.is_empty() {
            Segment::count(&kv.value).expect("segment count") as u64
        } else {
            1
        };
    }
    per_index
        .into_iter()
        .map(|(i, (keys, kb, vb, datoms))| (i, keys, kb, vb, datoms))
        .collect()
}

fn index_name(index: u8) -> &'static str {
    match index {
        codec::EAV => "EAV",
        codec::AVE => "AVE",
        codec::AEV => "AEV",
        codec::AE => "AE",
        codec::AV => "AV",
        codec::VAE => "VAE",
        codec::META_INDEX => "META",
        _ => "?",
    }
}

fn print_report(label: &str, report: &[(u8, u64, u64, u64, u64)]) {
    let (mut tk, mut tkb, mut tvb, mut td) = (0, 0, 0, 0);
    println!("--- {label} ---");
    println!(
        "{:<6} {:>8} {:>12} {:>12} {:>10}",
        "index", "keys", "key_bytes", "val_bytes", "datoms"
    );
    for (i, keys, kb, vb, datoms) in report {
        println!(
            "{:<6} {keys:>8} {kb:>12} {vb:>12} {datoms:>10}",
            index_name(*i)
        );
        tk += keys;
        tkb += kb;
        tvb += vb;
        td += datoms;
    }
    println!("{:<6} {tk:>8} {tkb:>12} {tvb:>12} {td:>10}", "TOTAL");
    println!("{label} total_bytes={}", tkb + tvb);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn row_and_columnar_layouts_agree() {
    let row = Node::memory_node_with_layout(SegmentLayout::Row).await;
    let row_as_of = build_graph(&row, 30, 3).await;
    let row_now = results(&row, None).await;
    let row_then = results(&row, Some(row_as_of)).await;
    let row_storage = storage_report(&row).await;

    let col = Node::memory_node_with_layout(SegmentLayout::Columnar { segment_size: 64 }).await;
    let col_as_of = build_graph(&col, 30, 3).await;
    let col_now = results(&col, None).await;
    let col_then = results(&col, Some(col_as_of)).await;
    let col_storage = storage_report(&col).await;

    for (i, (a, b)) in row_now.iter().zip(&col_now).enumerate() {
        assert_eq!(a, b, "query {} disagrees: {}", i, QUERIES[i]);
    }
    for (i, (a, b)) in row_then.iter().zip(&col_then).enumerate() {
        assert_eq!(a, b, "as-of query {} disagrees: {}", i, QUERIES[i]);
    }
    assert!(row_now[0].len() > 50, "graph should have edges");
    assert!(
        row_then[0].len() > row_now[0].len(),
        "retraction should be visible only after as-of"
    );

    print_report("row", &row_storage);
    print_report("columnar", &col_storage);

    let row_datoms: u64 = row_storage
        .iter()
        .filter(|(i, ..)| *i == codec::AEV)
        .map(|(_, _, _, _, d)| *d)
        .sum();
    let col_datoms: u64 = col_storage
        .iter()
        .filter(|(i, ..)| *i == codec::AEV)
        .map(|(_, _, _, _, d)| *d)
        .sum();
    assert_eq!(row_datoms, col_datoms, "AEV datom count must match");

    row.close().await.unwrap();
    col.close().await.unwrap();
}

/// Storage-only report at benchmark scale. Ignored by default because it ingests
/// tens of thousands of datoms in a debug build.
///
/// `VERTICES`, `FANOUT` and `TRIPLOX_SEGMENT_SIZE` tune the graph.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore]
async fn storage_report_at_scale() {
    let n: i64 = std::env::var("VERTICES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(2000);
    let fanout: i64 = std::env::var("FANOUT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(20);
    let segment_size: usize = std::env::var("TRIPLOX_SEGMENT_SIZE")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1024);
    println!("vertices={n} fanout={fanout} segment_size={segment_size}");

    let row = Node::memory_node_with_layout(SegmentLayout::Row).await;
    build_graph(&row, n, fanout).await;
    print_report("row", &storage_report(&row).await);
    row.close().await.unwrap();

    let col = Node::memory_node_with_layout(SegmentLayout::Columnar { segment_size }).await;
    build_graph(&col, n, fanout).await;
    print_report("columnar", &storage_report(&col).await);
    col.close().await.unwrap();
}
