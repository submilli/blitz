//! Checked attribute attachment and value changes through ordinary DOM reactions.
use super::{AttributeId, AttributeNodeError as Error, Location};
use crate::{BaseDocument, DomString, NodeId, QualName};

impl BaseDocument {
    pub fn try_set_attribute_node_value(
        &mut self,
        id: AttributeId,
        value: DomString,
    ) -> Result<(), Error> {
        let record = self
            .attribute_nodes
            .get(id)
            .ok_or(Error::InvalidAttribute)?;
        let Some(owner) = record.owner_element() else {
            let record = self
                .attribute_nodes
                .get_mut(id)
                .ok_or(Error::InvalidAttribute)?;
            if let Location::Detached { value: stored, .. } = &mut record.location {
                *stored = value;
                return Ok(());
            }
            return Err(Error::InvalidAttribute);
        };
        let name = record.name.clone();
        if !self
            .get_node(owner)
            .and_then(|node| node.element_data())
            .is_some_and(|element| {
                element
                    .attrs()
                    .iter()
                    .any(|attribute| attribute.name == name)
            })
        {
            return Err(Error::InvalidAttribute);
        }
        self.mutate().set_attribute(owner, name, value);
        Ok(())
    }

    pub fn try_remove_attribute_node(
        &mut self,
        owner: NodeId,
        id: AttributeId,
    ) -> Result<AttributeId, Error> {
        self.check_attribute_element(owner)?;
        let record = self
            .attribute_nodes
            .get(id)
            .ok_or(Error::InvalidAttribute)?;
        if record.owner_element() != Some(owner) {
            return Err(Error::NotFound);
        }
        let name = record.name.clone();
        self.mutate().clear_attribute(owner, name);
        Ok(id)
    }

    /// Adopting an attached Attr removes it from its element first. A detached
    /// Attr keeps its exact value while its directed document ownership changes.
    pub fn try_adopt_attribute_node(
        &mut self,
        id: AttributeId,
        document: NodeId,
    ) -> Result<AttributeId, Error> {
        self.check_attribute_document(document)?;
        let record = self
            .attribute_nodes
            .get(id)
            .ok_or(Error::InvalidAttribute)?;
        if let Some(owner) = record.owner_element() {
            self.try_remove_attribute_node(owner, id)?;
        }
        let record = self
            .attribute_nodes
            .get_mut(id)
            .ok_or(Error::InvalidAttribute)?;
        if let Location::Detached {
            document: stored, ..
        } = &mut record.location
        {
            *stored = document;
            Ok(id)
        } else {
            Err(Error::InvalidAttribute)
        }
    }

    /// Replace by namespace/local name, preserving the old position and facade.
    /// The new Attr becomes attached and follows its element's node document.
    pub fn try_attach_attribute_node(
        &mut self,
        owner: NodeId,
        id: AttributeId,
        limit: usize,
    ) -> Result<Option<AttributeId>, Error> {
        self.check_attribute_element(owner)?;
        let record = self
            .attribute_nodes
            .get(id)
            .ok_or(Error::InvalidAttribute)?;
        if let Some(previous) = record.owner_element() {
            return if previous == owner {
                Ok(Some(id))
            } else {
                Err(Error::InUse)
            };
        }
        let name = record.name.clone();
        let value = self
            .attribute_node_value(id)
            .ok_or(Error::InvalidAttribute)?
            .clone();
        let previous = self.attribute_to_replace(owner, &name)?;
        let old_id = previous
            .as_ref()
            .map(|(_, name, _)| self.try_ensure_attribute_node(owner, name, limit))
            .transpose()?
            .flatten();
        let mut mutator = self.mutate();
        if let Some((index, old_name, old_value)) = previous {
            if old_name != name {
                // Namespace/local name are unchanged, so this only replaces the
                // prefix. Snapshot before changing the native qualified name;
                // the following normal set emits one value-change reaction.
                if mutator
                    .doc
                    .get_node(owner)
                    .is_some_and(|node| node.flags.is_in_document())
                {
                    mutator.doc.snapshot_node(owner);
                }
                let attribute = mutator
                    .doc
                    .nodes
                    .get_mut(owner)
                    .and_then(|node| node.element_data_mut())
                    .and_then(|element| element.attrs.get_mut(index))
                    .expect("index selected from the unchanged native attribute list");
                attribute.name = name.clone();
            }
            mutator
                .doc
                .detach_attribute_record(owner, &old_name, &old_value);
        }
        mutator.set_attribute(owner, name, value);
        mutator.doc.attribute_nodes.attach(id, owner)?;
        Ok(old_id)
    }

    fn check_attribute_element(&self, owner: NodeId) -> Result<(), Error> {
        if self
            .get_node(owner)
            .and_then(|node| node.element_data())
            .is_some()
        {
            Ok(())
        } else {
            Err(Error::InvalidNode)
        }
    }

    fn attribute_to_replace(
        &self,
        owner: NodeId,
        name: &QualName,
    ) -> Result<Option<(usize, QualName, DomString)>, Error> {
        let element = self
            .get_node(owner)
            .and_then(|node| node.element_data())
            .ok_or(Error::InvalidNode)?;
        let mut matching = element.attrs().iter().enumerate().filter(|(_, attribute)| {
            attribute.name.ns == name.ns && attribute.name.local == name.local
        });
        let previous = matching
            .next()
            .map(|(index, attribute)| (index, attribute.name.clone(), attribute.value.clone()));
        // Raw native construction can contain duplicate namespace/local names;
        // reject that invalid state before any mutation or facade allocation.
        if matching.next().is_some() {
            return Err(Error::InvalidAttribute);
        }
        Ok(previous)
    }
}
