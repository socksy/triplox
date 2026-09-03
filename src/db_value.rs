use std::sync::Arc;

use anyhow::Error;
use tokio::runtime::Handle;

use crate::indexer::latest_tx_key_from_sdb;
use crate::ops::QueryArg;
use crate::partition::tx_eid_from_tx_id;
use crate::query::adjacency::{AdjMatrix, AdjacencyCache};
use crate::query::{execute_query, QueryResult};
use crate::schema::IdentMap;
use crate::segment::SegmentLayout;
use triplox_client::node::{Database, IntoQuery};
use triplox_client::transaction::TxKey;

pub struct DB<D = slatedb::Db, M = slatedb::Db>
where
    D: slatedb::DbReadOps + Send + Sync + 'static,
    M: slatedb::DbMetadataOps + Send + Sync + 'static,
{
    sdb: Arc<D>,
    ident_map: Arc<IdentMap>,
    handle: Handle,
    tx_key: TxKey,
    range_stats: Arc<slatedb_estimates::RangeStats<M>>,
    layout: SegmentLayout,
    // Present only when ref-attribute patterns should be served from adjacency matrices.
    adjacency: Option<Adjacency>,
}

#[derive(Clone)]
pub(crate) struct Adjacency {
    cache: Arc<AdjacencyCache>,
    ref_attributes: Arc<std::collections::HashSet<i64>>,
}

impl<D, M> Clone for DB<D, M>
where
    D: slatedb::DbReadOps + Send + Sync + 'static,
    M: slatedb::DbMetadataOps + Send + Sync + 'static,
{
    fn clone(&self) -> Self {
        Self {
            sdb: Arc::clone(&self.sdb),
            ident_map: Arc::clone(&self.ident_map),
            handle: self.handle.clone(),
            tx_key: self.tx_key,
            range_stats: Arc::clone(&self.range_stats),
            layout: self.layout,
            adjacency: self.adjacency.clone(),
        }
    }
}

#[allow(unused)]
impl<D, M> DB<D, M>
where
    D: slatedb::DbReadOps + Send + Sync + 'static,
    M: slatedb::DbMetadataOps + Send + Sync + 'static,
{
    pub fn new(
        sdb: Arc<D>,
        ident_map: IdentMap,
        handle: Handle,
        tx_key: TxKey,
        range_stats: Arc<slatedb_estimates::RangeStats<M>>,
    ) -> Self {
        Self {
            sdb,
            ident_map: Arc::new(ident_map),
            handle,
            tx_key,
            range_stats,
            layout: SegmentLayout::from_env(),
            adjacency: None,
        }
    }

    /// Serve ref-attribute triple patterns from cached adjacency matrices.
    pub(crate) fn with_adjacency(
        mut self,
        cache: Arc<AdjacencyCache>,
        ref_attributes: std::collections::HashSet<i64>,
    ) -> Self {
        self.adjacency = Some(Adjacency {
            cache,
            ref_attributes: Arc::new(ref_attributes),
        });
        self
    }

    pub(crate) fn without_adjacency(mut self) -> Self {
        self.adjacency = None;
        self
    }

    /// The adjacency matrix for `attribute`, or None when it is not a ref attribute or
    /// matrices are disabled on this DB value.
    pub(crate) fn adjacency(&self, attribute: i64) -> Result<Option<Arc<AdjMatrix>>, Error> {
        match &self.adjacency {
            Some(adjacency) if adjacency.ref_attributes.contains(&attribute) => {
                adjacency.cache.get_or_build(self, attribute).map(Some)
            }
            _ => Ok(None),
        }
    }

    pub fn with_layout(mut self, layout: SegmentLayout) -> Self {
        self.layout = layout;
        self
    }

    /// Construct a DB from a SlateDB instance by scanning EAV for TX_PARTITION entities to find the latest TxKey.
    pub async fn from_latest_sdb(
        sdb: Arc<D>,
        ident_map: IdentMap,
        handle: Handle,
        range_stats: Arc<slatedb_estimates::RangeStats<M>>,
    ) -> Result<Self, Error> {
        let tx_key = latest_tx_key_from_sdb(sdb.as_ref()).await?;
        Ok(Self {
            sdb,
            ident_map: Arc::new(ident_map),
            handle,
            tx_key,
            range_stats,
            layout: SegmentLayout::from_env(),
            adjacency: None,
        })
    }

    pub fn tx_key(&self) -> TxKey {
        self.tx_key
    }

    pub(crate) fn layout(&self) -> SegmentLayout {
        self.layout
    }

    pub(crate) fn sdb(&self) -> &D {
        self.sdb.as_ref()
    }

    pub(crate) fn ident_map(&self) -> &IdentMap {
        self.ident_map.as_ref()
    }

    pub(crate) fn handle(&self) -> &Handle {
        &self.handle
    }

    pub(crate) fn as_of(&self) -> i64 {
        tx_eid_from_tx_id(self.tx_key.tx_id)
    }

    pub(crate) fn range_stats(&self) -> &Arc<slatedb_estimates::RangeStats<M>> {
        &self.range_stats
    }
}

impl<D, M> Database for DB<D, M>
where
    D: slatedb::DbReadOps + Send + Sync + 'static,
    M: slatedb::DbMetadataOps + Send + Sync + 'static,
{
    async fn query(&self, query: impl IntoQuery) -> Result<QueryResult, Error> {
        self.query_with_args(query, &[]).await
    }

    /// Execute a query against this database basis.
    /// Runs the sync join algorithm in a blocking task to avoid blocking the async runtime.
    async fn query_with_args(
        &self,
        query: impl IntoQuery,
        args: &[QueryArg],
    ) -> Result<QueryResult, Error> {
        let db = Arc::new(self.clone());
        let query = query.into_query()?;
        let args = args.to_vec();

        tokio::task::spawn_blocking(move || execute_query(&query, &args, db))
            .await
            .map_err(|e| anyhow::anyhow!("Query task failed: {}", e))?
    }
}
