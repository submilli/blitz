//! Read-only metadata, bounded creation and explicit attribute-record collection.
use super::{AttributeId, AttributeNode, AttributeNodeError as Error, Location};
use crate::{BaseDocument, DomString, NodeData, NodeId, QualName};
use std::collections::HashSet;

impl BaseDocument {
    pub fn attribute_node(&self, id: AttributeId) -> Option<&AttributeNode> {
        self.attribute_nodes.get(id)
    }

    pub fn attribute_node_count(&self) -> usize {
        self.attribute_nodes.len()
    }

    /// Attached attributes follow their element's current node document.
    pub fn attribute_node_document(&self, id: AttributeId) -> Option<NodeId> {
        match &self.attribute_nodes.get(id)?.location {
            Location::Attached(owner) => self.get_node(*owner).map(|_| self.node_document(*owner)),
            Location::Detached { document, .. } => self.get_node(*document).map(|_| *document),
        }
    }

    pub fn attribute_node_value(&self, id: AttributeId) -> Option<&DomString> {
        let record = self.attribute_nodes.get(id)?;
        match &record.location {
            Location::Detached { value, .. } => Some(value),
            Location::Attached(owner) => self
                .get_node(*owner)?
                .element_data()?
                .attrs()
                .iter()
                .find(|attribute| attribute.name == record.name)
                .map(|attribute| &attribute.value),
        }
    }

    /// Ensure the facade identity of an existing exact native attribute name.
    /// This does not create an attribute on the element.
    pub fn try_ensure_attribute_node(
        &mut self,
        owner: NodeId,
        name: &QualName,
        limit: usize,
    ) -> Result<Option<AttributeId>, Error> {
        let element = self
            .get_node(owner)
            .and_then(|node| node.element_data())
            .ok_or(Error::InvalidNode)?;
        if !element
            .attrs()
            .iter()
            .any(|attribute| attribute.name == *name)
        {
            return Ok(None);
        }
        self.attribute_nodes
            .ensure(owner, name.clone(), limit)
            .map(Some)
    }

    /// Create a detached native attribute from an already validated qualified
    /// name. Web API name conversion and validation precede this typed operation.
    pub fn try_create_attribute_node(
        &mut self,
        document: NodeId,
        name: QualName,
        limit: usize,
    ) -> Result<AttributeId, Error> {
        self.check_attribute_document(document)?;
        self.attribute_nodes
            .create(document, name, DomString::new(), limit)
    }

    pub fn try_clone_attribute_node(
        &mut self,
        id: AttributeId,
        document: NodeId,
        limit: usize,
    ) -> Result<AttributeId, Error> {
        self.check_attribute_document(document)?;
        let record = self
            .attribute_nodes
            .get(id)
            .ok_or(Error::InvalidAttribute)?;
        let name = record.name.clone();
        let value = self
            .attribute_node_value(id)
            .ok_or(Error::InvalidAttribute)?
            .clone();
        self.attribute_nodes.create(document, name, value, limit)
    }

    /// Native ownership without pretending attribute objects are tree nodes.
    /// Attached identities share their element's component; detached identities
    /// have only a directed edge to their retained node document.
    pub fn attribute_node_references(&self) -> impl Iterator<Item = (AttributeId, NodeId, bool)> {
        self.attribute_nodes
            .iter()
            .map(|(id, record)| match record.location {
                Location::Attached(owner) => (id, owner, true),
                Location::Detached { document, .. } => (id, document, false),
            })
    }

    /// Remove only facade records unreachable from embedder wrappers. Actual
    /// element attributes remain intact. Run before tree-node reclamation.
    pub fn reclaim_attribute_nodes(&mut self, live: &[AttributeId]) -> Vec<AttributeId> {
        self.attribute_nodes
            .reclaim(&live.iter().copied().collect::<HashSet<_>>())
    }

    /// Logical retained payload for embedder admission; attached values already
    /// belong to the element, while detached values need their own reservation.
    pub fn attribute_node_payload_units(&self) -> usize {
        self.attribute_nodes
            .iter()
            .fold(0usize, |units, (_, record)| {
                let name = record
                    .name
                    .local
                    .as_ref()
                    .encode_utf16()
                    .count()
                    .saturating_add(record.name.ns.as_ref().encode_utf16().count())
                    .saturating_add(
                        record
                            .name
                            .prefix
                            .as_ref()
                            .map_or(0, |prefix| prefix.as_ref().encode_utf16().count()),
                    );
                let value = match &record.location {
                    Location::Attached(_) => 0,
                    Location::Detached { value, .. } => value.utf16_len_bound(),
                };
                units.saturating_add(name).saturating_add(value)
            })
    }

    pub(super) fn check_attribute_document(&self, id: NodeId) -> Result<(), Error> {
        if self
            .get_node(id)
            .is_some_and(|node| matches!(node.data, NodeData::Document(_)))
        {
            Ok(())
        } else {
            Err(Error::InvalidNode)
        }
    }
    pub(crate) fn detach_attribute_record(
        &mut self,
        owner: NodeId,
        name: &QualName,
        value: &DomString,
    ) {
        if self.attribute_nodes.len() == 0 || self.attribute_nodes.find(owner, name).is_none() {
            return;
        }
        let Some(_) = self.get_node(owner) else {
            return;
        };
        let document = self.node_document(owner);
        self.attribute_nodes
            .detach(owner, name, document, value.clone());
    }

    pub(crate) fn prune_attribute_records_for_tree(&mut self, live: &HashSet<NodeId>) {
        self.attribute_nodes.prune_nodes(live);
    }
}
