//! Shared end-to-end Datalog benchmark. Loads a seeded random graph into an
//! in-memory node and times a fixed set of queries so that storage and
//! execution experiments can be compared on identical data.
//!
//! Run: `cargo bench --bench datalog_bench`
//! Env: VERTICES (default 2000), EDGE_PROB (default 0.01), RUNS (default 5),
//!      BENCH_OUT (optional path for JSON results), ASOF=1 to add as-of queries
//!      at the basis after vertex ingest and half-way through edge ingest.
//!      Zone map skip counters are printed when TRIPLOX_ZONE_MAPS=1.

use std::time::Instant;

use edn::kw;
use edn::Keyword;
use triplox::ops::{DataType, EntityRef, TxOp};
use triplox::{Database, Node, QueryNode, SubmitNode, TransactionResult, DB};
use triplox_client::transaction::TxKey;

struct XorShift(u64);

impl XorShift {
    fn next_f64(&mut self) -> f64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        (x >> 11) as f64 / (1u64 << 53) as f64
    }
}

fn env_or<T: std::str::FromStr>(name: &str, default: T) -> T {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

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

fn schema() -> Vec<TxOp> {
    vec![
        schema_attr(kw!(:g/id), "long", "one", true),
        schema_attr(kw!(:g/to), "ref", "many", false),
        schema_attr(kw!(:g/label), "string", "one", false),
        schema_attr(kw!(:g/weight), "long", "one", false),
    ]
}

fn gnp_edges(n: i64, p: f64, seed: u64) -> Vec<(i64, i64)> {
    let mut rng = XorShift(seed);
    let mut edges = Vec::new();
    for i in 0..n {
        for j in 0..n {
            if i != j && rng.next_f64() < p {
                edges.push((i, j));
            }
        }
    }
    edges
}

async fn commit(node: &impl SubmitNode, ops: Vec<TxOp>) -> TxKey {
    match node.execute_tx(ops).await.expect("tx should execute") {
        TransactionResult::TxCommitted(tx_key) => tx_key,
        TransactionResult::TxAborted(_, err) => panic!("tx aborted: {err}"),
    }
}

const QUERIES: &[(&str, &str)] = &[
    (
        "triangles",
        "[:find ?a ?b ?c :where [?a :g/to ?b] [?b :g/to ?c] [?a :g/to ?c]]",
    ),
    (
        "two_hop_count",
        "[:find (count ?c) :where [?a :g/to ?b] [?b :g/to ?c]]",
    ),
    (
        "three_hop_count",
        "[:find (count ?d) :where [?a :g/to ?b] [?b :g/to ?c] [?c :g/to ?d]]",
    ),
    ("out_degree", "[:find ?a (count ?b) :where [?a :g/to ?b]]"),
    (
        "in_degree_top",
        "[:find ?b (count ?a) :where [?a :g/to ?b]]",
    ),
    (
        "neighbors_of_42",
        "[:find ?b :where [?a :g/id 42] [?a :g/to ?b]]",
    ),
    (
        "weight_filter",
        "[:find ?e :where [?e :g/weight ?w] [(> ?w 900)]]",
    ),
    ("weight_sum", "[:find (sum ?w) :where [?e :g/weight ?w]]"),
    (
        "heavy_neighbors",
        "[:find ?a ?b :where [?a :g/to ?b] [?b :g/weight ?w] [(> ?w 950)]]",
    ),
    (
        "label_lookup",
        "[:find ?e :where [?e :g/label \"label-777\"]]",
    ),
];

#[tokio::main(flavor = "multi_thread")]
async fn main() {
    let vertices: i64 = env_or("VERTICES", 2000);
    let edge_prob: f64 = env_or("EDGE_PROB", 0.01);
    let runs: usize = env_or("RUNS", 5);
    let batch: usize = env_or("BATCH_SIZE", 1000);
    let asof: bool = env_or("ASOF", 0u8) == 1;

    let node = Node::memory_node().await;
    commit(&node, schema()).await;

    let ingest_start = Instant::now();
    let vertex_ops: Vec<TxOp> = (0..vertices)
        .map(|i| {
            TxOp::put([
                (kw!(:g/id), DataType::Long(i)),
                (kw!(:g/label), DataType::String(format!("label-{i}"))),
                (kw!(:g/weight), DataType::Long(i % 1000)),
            ])
        })
        .collect();
    let mut basis_vertices = None;
    for chunk in vertex_ops.chunks(batch) {
        basis_vertices = Some(commit(&node, chunk.to_vec()).await);
    }
    let basis_vertices = basis_vertices.expect("at least one vertex chunk");

    let db = node.db().await.expect("db");
    let rows = db
        .query("[:find ?id ?e :where [?e :g/id ?id]]")
        .await
        .expect("id lookup");
    let mut eid = vec![0i64; vertices as usize];
    for row in rows {
        let [DataType::Long(id), DataType::Long(e)] = row.as_slice() else {
            panic!("unexpected id row {row:?}");
        };
        eid[*id as usize] = *e;
    }

    let edges = gnp_edges(vertices, edge_prob, 0x5eed);
    let edge_ops: Vec<TxOp> = edges
        .iter()
        .map(|(from, to)| TxOp::Add {
            entity: EntityRef::Id(eid[*from as usize]),
            attribute: kw!(:g/to),
            value: DataType::Long(eid[*to as usize]),
        })
        .collect();
    let chunks: Vec<&[TxOp]> = edge_ops.chunks(batch).collect();
    let mut basis_mid = basis_vertices;
    for (index, chunk) in chunks.iter().enumerate() {
        let tx_key = commit(&node, chunk.to_vec()).await;
        if index + 1 == chunks.len() / 2 {
            basis_mid = tx_key;
        }
    }
    let ingest_ms = ingest_start.elapsed().as_secs_f64() * 1000.0;

    println!(
        "vertices={vertices} edges={} edge_prob={edge_prob} runs={runs} ingest_ms={ingest_ms:.1}",
        edges.len()
    );
    println!(
        "{:<22} {:>10} {:>12} {:>12} {:>10} {:>10} {:>8}",
        "query", "rows", "min_ms", "median_ms", "v_skips", "t_skips", "build_ms"
    );

    let db = node.db().await.expect("db");
    let mut queries: Vec<(String, &str, DB)> = QUERIES
        .iter()
        .map(|(name, q)| (name.to_string(), *q, db.clone()))
        .collect();
    if asof {
        let db_vertices = node.db_as_of(basis_vertices).await.expect("db as of");
        let db_mid = node.db_as_of(basis_mid).await.expect("db as of mid");
        let out_degree = "[:find ?a (count ?b) :where [?a :g/to ?b]]";
        let two_hop = "[:find (count ?c) :where [?a :g/to ?b] [?b :g/to ?c]]";
        queries.push(("asof_out_degree".into(), out_degree, db_vertices.clone()));
        queries.push(("asof_two_hop_count".into(), two_hop, db_vertices));
        queries.push(("asof_mid_out_degree".into(), out_degree, db_mid));
    }
    let mut json = Vec::new();
    triplox::zone_map::take_stats();
    for (name, q, db) in &queries {
        let mut times = Vec::with_capacity(runs);
        let mut rows = 0;
        for _ in 0..runs {
            let start = Instant::now();
            let result = db.query(*q).await.unwrap_or_else(|e| panic!("{name}: {e}"));
            times.push(start.elapsed().as_secs_f64() * 1000.0);
            rows = result.len();
        }
        times.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let min = times[0];
        let median = times[times.len() / 2];
        // Skip counts are per execution; builds happen once and are reported as a total.
        let stats = triplox::zone_map::take_stats();
        let v_skips = stats.seeks_skipped / runs as u64;
        let t_skips = stats.runs_skipped_t / runs as u64;
        let build_ms = stats.build_micros as f64 / 1000.0;
        println!(
            "{name:<22} {rows:>10} {min:>12.2} {median:>12.2} {v_skips:>10} {t_skips:>10} {build_ms:>8.2}"
        );
        json.push(format!(
            "{{\"query\":\"{name}\",\"rows\":{rows},\"min_ms\":{min:.3},\"median_ms\":{median:.3},\"v_skips\":{v_skips},\"v_checks\":{},\"t_skips\":{t_skips},\"builds\":{},\"build_ms\":{build_ms:.3}}}",
            stats.seeks_checked / runs as u64,
            stats.builds
        ));
    }

    if let Ok(path) = std::env::var("BENCH_OUT") {
        let body = format!(
            "{{\"vertices\":{vertices},\"edges\":{},\"edge_prob\":{edge_prob},\"ingest_ms\":{ingest_ms:.1},\"queries\":[{}]}}\n",
            edges.len(),
            json.join(",")
        );
        std::fs::write(&path, body).expect("write BENCH_OUT");
    }

    node.close().await.expect("close");
}
