use std::sync::Arc;

use anyhow::{ensure, Result};
use bytes::Bytes;
use edn::query::Variable;

use crate::query::binding_bag::BindingBag;

// Columnar binding set with structural sharing: a batch either owns a column or refers to a
// parent's column through a row map, so selections and extensions never copy existing values.
#[derive(Clone)]
enum Column {
    Own(Arc<Vec<Bytes>>),
    Parent(usize),
}

pub(crate) struct Batch {
    pub(crate) variables: Vec<Variable>,
    columns: Vec<Column>,
    parent: Option<(Arc<Batch>, Arc<Vec<u32>>)>,
    len: usize,
}

// A resolved column: `values[index[row]]` is the value of `row`.
pub(crate) struct ColumnView {
    values: Arc<Vec<Bytes>>,
    index: Vec<u32>,
}

impl ColumnView {
    #[inline]
    pub(crate) fn get(&self, row: usize) -> &Bytes {
        &self.values[self.index[row] as usize]
    }

    pub(crate) fn len(&self) -> usize {
        self.index.len()
    }
}

impl Batch {
    pub(crate) fn unit() -> Self {
        Self {
            variables: Vec::new(),
            columns: Vec::new(),
            parent: None,
            len: 1,
        }
    }

    pub(crate) fn from_binding_bag(bag: &BindingBag) -> Self {
        let mut columns: Vec<Vec<Bytes>> = (0..bag.variables.len())
            .map(|_| Vec::with_capacity(bag.rows.len()))
            .collect();
        for row in &bag.rows {
            for (column, value) in columns.iter_mut().zip(row) {
                column.push(value.clone());
            }
        }
        Self {
            variables: bag.variables.clone(),
            columns: columns
                .into_iter()
                .map(|column| Column::Own(Arc::new(column)))
                .collect(),
            parent: None,
            len: bag.rows.len(),
        }
    }

    pub(crate) fn to_binding_bag(&self) -> Result<BindingBag> {
        let views: Vec<ColumnView> = (0..self.columns.len()).map(|c| self.view(c)).collect();
        let rows = (0..self.len)
            .map(|row| views.iter().map(|view| view.get(row).clone()).collect())
            .collect();
        BindingBag::new(self.variables.clone(), rows)
    }

    pub(crate) fn len(&self) -> usize {
        self.len
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub(crate) fn column_index(&self, variable: &Variable) -> Result<usize> {
        self.variables
            .iter()
            .position(|v| v == variable)
            .ok_or_else(|| anyhow::anyhow!("Unknown batch variable: {variable}"))
    }

    pub(crate) fn contains(&self, variable: &Variable) -> bool {
        self.variables.contains(variable)
    }

    fn resolve(&self, column: usize, rows: Vec<u32>) -> ColumnView {
        match &self.columns[column] {
            Column::Own(values) => ColumnView {
                values: Arc::clone(values),
                index: rows,
            },
            Column::Parent(parent_column) => {
                let (parent, map) = self.parent.as_ref().expect("parent column without parent");
                let mapped = rows.iter().map(|row| map[*row as usize]).collect();
                parent.resolve(*parent_column, mapped)
            }
        }
    }

    pub(crate) fn view(&self, column: usize) -> ColumnView {
        self.resolve(column, (0..self.len as u32).collect())
    }

    pub(crate) fn view_of(&self, variable: &Variable) -> Result<ColumnView> {
        Ok(self.view(self.column_index(variable)?))
    }

    fn is_pure_selection(&self) -> bool {
        self.parent.is_some()
            && self
                .columns
                .iter()
                .enumerate()
                .all(|(i, column)| matches!(column, Column::Parent(p) if *p == i))
    }

    // Keeps the requested rows; chained selections are composed so the parent chain only grows
    // with extensions.
    pub(crate) fn select(self: &Arc<Self>, rows: Vec<u32>) -> Batch {
        let len = rows.len();
        let (parent, map) = if self.is_pure_selection() {
            let (parent, own_map) = self.parent.as_ref().expect("checked above");
            let composed = rows.iter().map(|row| own_map[*row as usize]).collect();
            (Arc::clone(parent), Arc::new(composed))
        } else {
            (Arc::clone(self), Arc::new(rows))
        };
        Batch {
            variables: self.variables.clone(),
            columns: (0..self.columns.len()).map(Column::Parent).collect(),
            parent: Some((parent, map)),
            len,
        }
    }

    pub(crate) fn extend(
        self: &Arc<Self>,
        parent_rows: Vec<u32>,
        variable: Variable,
        values: Vec<Bytes>,
    ) -> Result<Batch> {
        ensure!(
            parent_rows.len() == values.len(),
            "Batch extension has {} parent rows for {} values",
            parent_rows.len(),
            values.len()
        );
        ensure!(
            !self.variables.contains(&variable),
            "Batch already binds {variable}"
        );
        let len = values.len();
        let mut columns: Vec<Column> = (0..self.columns.len()).map(Column::Parent).collect();
        columns.push(Column::Own(Arc::new(values)));
        let mut variables = self.variables.clone();
        variables.push(variable);
        Ok(Batch {
            variables,
            columns,
            parent: Some((Arc::clone(self), Arc::new(parent_rows))),
            len,
        })
    }

    pub(crate) fn reorder(&self, target: &[Variable]) -> Result<Batch> {
        ensure!(
            target.len() == self.variables.len(),
            "Reordered layout must contain exactly the current variables"
        );
        let columns = target
            .iter()
            .map(|variable| Ok(self.columns[self.column_index(variable)?].clone()))
            .collect::<Result<Vec<_>>>()?;
        Ok(Batch {
            variables: target.to_vec(),
            columns,
            parent: self.parent.clone(),
            len: self.len,
        })
    }

    pub(crate) fn chunks(self: &Arc<Self>, size: usize) -> Vec<Arc<Batch>> {
        if self.len <= size {
            return vec![Arc::clone(self)];
        }
        (0..self.len)
            .step_by(size)
            .map(|start| {
                let end = (start + size).min(self.len);
                Arc::new(self.select((start as u32..end as u32).collect()))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use edn::query::ToVariable;

    fn bytes(value: &str) -> Bytes {
        Bytes::copy_from_slice(value.as_bytes())
    }

    fn bag(variables: &[&str], rows: &[&[&str]]) -> BindingBag {
        BindingBag::new(
            variables.iter().map(|v| v.to_var()).collect(),
            rows.iter()
                .map(|row| row.iter().map(|v| bytes(v)).collect())
                .collect(),
        )
        .unwrap()
    }

    #[test]
    fn extend_select_reorder_round_trip_through_binding_bags() {
        let base = Arc::new(Batch::from_binding_bag(&bag(&["?x"], &[&["a"], &["b"]])));
        let extended = Arc::new(
            base.extend(
                vec![0, 0, 1],
                "?y".to_var(),
                vec![bytes("1"), bytes("2"), bytes("3")],
            )
            .unwrap(),
        );
        assert_eq!(
            extended.to_binding_bag().unwrap(),
            bag(&["?x", "?y"], &[&["a", "1"], &["a", "2"], &["b", "3"]])
        );

        let selected = Arc::new(extended.select(vec![2, 0]));
        let reselected = Arc::new(selected.select(vec![1]));
        assert!(reselected.is_pure_selection());
        assert_eq!(
            reselected.to_binding_bag().unwrap(),
            bag(&["?x", "?y"], &[&["a", "1"]])
        );

        let reordered = selected.reorder(&["?y".to_var(), "?x".to_var()]).unwrap();
        assert_eq!(
            reordered.to_binding_bag().unwrap(),
            bag(&["?y", "?x"], &[&["3", "b"], &["1", "a"]])
        );

        let chunks = extended.chunks(2);
        assert_eq!(chunks.len(), 2);
        assert_eq!(
            chunks[1].to_binding_bag().unwrap(),
            bag(&["?x", "?y"], &[&["b", "3"]])
        );
        assert!(extended.reorder(&["?x".to_var()]).is_err());
        assert!(extended.extend(vec![0], "?x".to_var(), vec![]).is_err());
    }
}
