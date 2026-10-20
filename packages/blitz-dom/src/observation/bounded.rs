//! Bounded observation text, sharing one admission across repeated projections.
use crate::{BaseDocument, Node, NodeData, NodeId};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ObservationLimit;
impl std::fmt::Display for ObservationLimit {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("observation text limit exceeded")
    }
}
impl std::error::Error for ObservationLimit {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ObservationQueryError {
    Limit,
    InvalidSelector,
}
impl std::fmt::Display for ObservationQueryError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Limit => "observation query limit exceeded",
            Self::InvalidSelector => "invalid observation selector",
        })
    }
}
impl std::error::Error for ObservationQueryError {}

/// Visits include nodes and inspected attribute slots. Text is charged by a
/// conservative UTF-8 bound before any UTF-16 projection or output allocation.
/// A failed projection leaves the shared budget failed, and returns no partial text.
pub struct ObservationBudget {
    visits: usize,
    bytes: usize,
    failed: bool,
}
impl ObservationBudget {
    pub fn new(visits: usize, bytes: usize) -> Self {
        Self {
            visits,
            bytes,
            failed: false,
        }
    }
    pub fn remaining(&self) -> (usize, usize) {
        (self.visits, self.bytes)
    }
    fn admit(&mut self, visits: usize, bytes: usize) -> Result<(), ObservationLimit> {
        if !self.failed
            && let (Some(visits), Some(bytes)) = (
                self.visits.checked_sub(visits),
                self.bytes.checked_sub(bytes),
            )
        {
            self.visits = visits;
            self.bytes = bytes;
            return Ok(());
        }
        self.failed = true;
        Err(ObservationLimit)
    }
    fn node(&mut self, node: &Node) -> Result<(), ObservationLimit> {
        self.admit(
            1usize.saturating_add(node.attrs().map_or(0, |attrs| attrs.len())),
            0,
        )
    }
}

impl BaseDocument {
    /// Admit all light-tree attribute projections before using the existing CSS
    /// matcher. Matching may inspect ancestors/relatives and cache lossy UTF-8
    /// attributes even when the selector has no matches. This conservative
    /// preflight bounds that payload; it is not a per-comparison CSS CPU meter.
    pub fn try_observation_query(
        &self,
        selector: &str,
        budget: &mut ObservationBudget,
    ) -> Result<Vec<NodeId>, ObservationQueryError> {
        budget
            .admit(0, selector.len())
            .map_err(|_| ObservationQueryError::Limit)?;
        self.admit_observation_attributes(budget)
            .map_err(|_| ObservationQueryError::Limit)?;
        self.query_selector_all(selector)
            .map(|nodes| nodes.into_iter().collect())
            .map_err(|_| ObservationQueryError::InvalidSelector)
    }

    fn admit_observation_attributes(
        &self,
        budget: &mut ObservationBudget,
    ) -> Result<(), ObservationLimit> {
        let root = self.root_node();
        budget.node(root)?;
        let mut pending = vec![(root, 0)];
        while let Some((node, child)) = pending.last_mut() {
            if *child == 0 {
                for attr in node.attrs().into_iter().flatten() {
                    budget.admit(0, attr.value.utf16_len_bound().saturating_mul(3))?;
                }
            }
            if let Some(next) = node.children.get(*child) {
                let next = &node.tree()[*next];
                budget.node(next)?;
                *child += 1;
                pending.push((next, 0));
            } else {
                pending.pop();
            }
        }
        Ok(())
    }
}

impl Node {
    /// The same redacted light-tree text as observation_text, admitted before
    /// copying. Cursor traversal bounds working memory by admitted depth rather
    /// than allocating a whole unvisited sibling list.
    pub fn try_observation_text(
        &self,
        budget: &mut ObservationBudget,
    ) -> Result<String, ObservationLimit> {
        budget.node(self)?;
        let mut output = String::new();
        let mut pending = vec![(self, 0)];
        while let Some((node, child)) = pending.last_mut() {
            if *child == 0 {
                if node.masks_observation() {
                    budget.admit(0, "[REDACTED]".len())?;
                    output.push_str("[REDACTED]");
                    pending.pop();
                    continue;
                }
                if let NodeData::Text(text) = &node.data {
                    budget.admit(0, text.content.utf16_len_bound().saturating_mul(3))?;
                    output.push_str(text.content.as_str_lossy());
                    pending.pop();
                    continue;
                }
            }
            if let Some(next) = node.children.get(*child) {
                let next = &node.tree()[*next];
                budget.node(next)?;
                *child += 1;
                pending.push((next, 0));
            } else {
                pending.pop();
            }
        }
        Ok(output)
    }
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
