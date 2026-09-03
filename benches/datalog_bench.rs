//! Shared end-to-end Datalog benchmark. Loads a seeded random graph into an
//! in-memory node and times a fixed set of queries so that storage and
//! execution experiments can be compared on identical data.
//!
//! Run: `cargo bench --bench datalog_bench`
//! Env: VERTICES (default 2000), EDGE_PROB (default 0.01), RUNS (default 5),
//!      BENCH_OUT (optional path for JSON results),
//!      QUERY_FILTER (optional comma-separated query names to run).

use std::time::Instant;

use edn::kw;
use edn::Keyword;
use triplox::ops::{DataType, EntityRef, TxOp};
use triplox::{Database, Node, QueryNode, SubmitNode, TransactionResult};

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

async fn commit(node: &impl SubmitNode, ops: Vec<TxOp>) {
    match node.execute_tx(ops).await.expect("tx should execute") {
        TransactionResult::TxCommitted(_) => {}
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
    for chunk in vertex_ops.chunks(batch) {
        commit(&node, chunk.to_vec()).await;
    }

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
    for chunk in edge_ops.chunks(batch) {
        commit(&node, chunk.to_vec()).await;
    }
    let ingest_ms = ingest_start.elapsed().as_secs_f64() * 1000.0;

    println!(
        "vertices={vertices} edges={} edge_prob={edge_prob} runs={runs} ingest_ms={ingest_ms:.1}",
        edges.len()
    );
    println!(
        "{:<18} {:>10} {:>12} {:>12}",
        "query", "rows", "min_ms", "median_ms"
    );

    let db = node.db().await.expect("db");
    let mut json = Vec::new();
    let filter: Option<Vec<String>> = std::env::var("QUERY_FILTER")
        .ok()
        .map(|v| v.split(',').map(str::to_string).collect());
    for (name, q) in QUERIES {
        if filter
            .as_ref()
            .is_some_and(|f| !f.iter().any(|n| n == name))
        {
            continue;
        }
        // Marks the stderr stream so a TRIPLOX_ENGINE_LOG run can be read per query.
        eprintln!("--query {name}");
        let mut times = Vec::with_capacity(runs);
        let mut rows = 0;
        for _ in 0..runs {
            let start = Instant::now();
            let result = db.query(*q).await.unwrap_or_else(|e| panic!("{name}: {e}"));
            times.push(start.elapsed().as_secs_f64() * 1000.0);
            rows = result.len();
        }
        let samples: Vec<String> = times.iter().map(|t| format!("{t:.3}")).collect();
        times.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let min = times[0];
        let median = times[times.len() / 2];
        println!("{name:<18} {rows:>10} {min:>12.2} {median:>12.2}");
        json.push(format!(
            "{{\"query\":\"{name}\",\"rows\":{rows},\"min_ms\":{min:.3},\"median_ms\":{median:.3},\"samples_ms\":[{}]}}",
            samples.join(",")
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
