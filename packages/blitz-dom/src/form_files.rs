//! File selection is a host-prepared memory handoff, never filesystem access.
use crate::{BaseDocument, DocumentMutator, NodeId, ns};
use blitz_traits::net::FormFile;

/// Admission failure leaves the previous selection untouched.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormFileError {
    NotFileInput,
    DataLimit,
    InvalidMetadata,
}
impl std::fmt::Display for FormFileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::NotFileInput => "Not a file input",
            Self::DataLimit => "File selection data limit exceeded",
            Self::InvalidMetadata => "Invalid file metadata",
        })
    }
}
impl std::error::Error for FormFileError {}

impl DocumentMutator<'_> {
    /// Replace a selection with host-authorized snapshots. Hosts perform bounded
    /// async reads before this call; it only validates and transfers memory.
    /// Selection is capped at 256 files and 16 MiB per document (including names).
    /// Input/change dispatch is the caller's responsibility.
    pub fn set_form_files(
        &mut self,
        id: NodeId,
        files: Vec<FormFile>,
    ) -> Result<(), FormFileError> {
        if !self.doc.is_file_input(id) {
            return Err(FormFileError::NotFileInput);
        }
        if files.len() > 256 {
            return Err(FormFileError::DataLimit);
        }
        let mut total = 0usize;
        for file in &files {
            // Names cannot carry a host path; MIME values cannot inject headers.
            if file.name.contains(['/', '\\'])
                || file.name.len() > 4096
                || file.content_type.len() > 1024
                || !file
                    .content_type
                    .bytes()
                    .all(|b| (0x20..=0x7e).contains(&b))
            {
                return Err(FormFileError::InvalidMetadata);
            }
            total = total
                .saturating_add(file.bytes.len())
                .saturating_add(file.name.len())
                .saturating_add(file.content_type.len());
        }
        for (node_id, node) in self.doc.nodes.iter() {
            if node_id == id {
                continue;
            }
            if let Some(e) = node.element_data() {
                for file in e.form_state.files.iter() {
                    total = total
                        .saturating_add(file.bytes.len())
                        .saturating_add(file.name.len())
                        .saturating_add(file.content_type.len());
                }
            }
        }
        if total > 16 * 1024 * 1024 {
            return Err(FormFileError::DataLimit);
        }
        if let Some(e) = self.doc.nodes[id].element_data_mut() {
            e.clear_file_selection();
            e.form_state.files = files.into();
        }
        Ok(())
    }
}

impl BaseDocument {
    /// Whether this node uses HTML's filename value mode.
    pub fn is_file_input(&self, id: NodeId) -> bool {
        self.get_node(id)
            .and_then(|n| n.element_data())
            .is_some_and(|e| {
                e.name.ns == ns!(html)
                    && &*e.name.local == "input"
                    && e.attr(markup5ever::local_name!("type"))
                        .is_some_and(|t| t.eq_ignore_ascii_case("file"))
            })
    }
}

impl crate::ElementData {
    pub(crate) fn selected_file_name(&self) -> Option<std::borrow::Cow<'_, str>> {
        if let Some(file) = self.form_state.files.first() {
            return Some((&file.name).into());
        }
        #[cfg(feature = "file-input")]
        {
            self.file_data()?
                .first()?
                .file_name()
                .map(|n| n.to_string_lossy())
        }
        #[cfg(not(feature = "file-input"))]
        None
    }
}

#[cfg(all(test, feature = "file-input"))]
mod tests {
    use crate::{BaseDocument, DocumentConfig, node::SpecialElementData};
    use markup5ever::{QualName, local_name, ns};

    #[test]
    fn legacy_selection_exposes_only_fakepath_and_clears_with_snapshots() {
        let mut doc = BaseDocument::new(DocumentConfig::default());
        let id = doc
            .mutate()
            .create_element(QualName::new(None, ns!(html), local_name!("input")), vec![]);
        doc.mutate()
            .set_attribute_by_name(id, "type", "file")
            .unwrap();
        doc.get_node_mut(id)
            .unwrap()
            .element_data_mut()
            .unwrap()
            .special_data = SpecialElementData::FileInput(
            vec![std::path::PathBuf::from("/private/files/name.txt")].into(),
        );
        assert_eq!(doc.form_value(id).unwrap(), "C:\\fakepath\\name.txt");
        doc.mutate().set_form_files(id, vec![]).unwrap();
        assert_eq!(doc.form_value(id).unwrap(), "");
        assert!(
            doc.get_node(id)
                .unwrap()
                .element_data()
                .unwrap()
                .file_data()
                .unwrap()
                .is_empty()
        );
    }
}
