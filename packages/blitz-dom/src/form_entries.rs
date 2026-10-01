//! Successful-control selection shared by navigation and FormData bindings.
use crate::{BaseDocument, NodeId, ns};
use blitz_traits::net::{Entry, EntryValue, FormData};
use markup5ever::local_name;
use std::collections::HashMap;

impl BaseDocument {
    /// Construct current entries in tree order, including associated external
    /// controls. Construction retains at most 16,384 entries and 16 MiB of
    /// metadata/text; file bytes remain shared. Legacy native paths are never
    /// authority for a binding-side read. Failure returns no partial list.
    /// https://html.spec.whatwg.org/multipage/form-control-infrastructure.html#constructing-the-entry-list
    pub fn form_entry_list(
        &self,
        form: NodeId,
        submitter: Option<NodeId>,
    ) -> Result<FormData, blitz_traits::net::FormDataLimit> {
        let controls: std::collections::HashSet<_> = self.form_controls(form).into_iter().collect();
        let mut contexts = HashMap::<NodeId, (bool, bool)>::new();
        let mut entries = Entries::default();
        for id in crate::traversal::TreeTraverser::new_with_root(self, self.tree_root(form)) {
            if entries.failed {
                break;
            }
            let node = &self.nodes[id];
            let (mut disabled, mut datalist) = node
                .parent
                .and_then(|p| contexts.get(&p).copied())
                .unwrap_or_default();
            if let Some(parent) = node.parent.and_then(|p| self.get_node(p)) {
                disabled |= parent.fieldset_disables(node);
            }
            let element = node.element_data().filter(|e| e.name.ns == ns!(html));
            datalist |= element.is_some_and(|e| &*e.name.local == "datalist");
            contexts.insert(id, (disabled, datalist));
            if controls.contains(&id) && !datalist && !node.is_disabled_with_ancestors(disabled) {
                self.append_control_entries(id, submitter, &mut entries);
            }
        }
        if entries.failed {
            Err(blitz_traits::net::FormDataLimit)
        } else {
            Ok(entries.data)
        }
    }

    fn append_control_entries(&self, id: NodeId, submitter: Option<NodeId>, entries: &mut Entries) {
        let Some(e) = self.nodes[id].element_data() else {
            return;
        };
        let tag = &*e.name.local;
        if !matches!(tag, "input" | "select" | "textarea" | "button") {
            return;
        }
        let kind = e
            .attr(local_name!("type"))
            .unwrap_or("")
            .to_ascii_lowercase();
        let name = e.attr(local_name!("name")).unwrap_or("");
        if tag == "input" && kind == "image" {
            if submitter == Some(id) {
                let prefix = if name.is_empty() {
                    String::new()
                } else {
                    format!("{name}.")
                };
                push(
                    entries,
                    &format!("{prefix}x"),
                    EntryValue::String(e.form_state.image_coordinates.0.to_string()),
                );
                push(
                    entries,
                    &format!("{prefix}y"),
                    EntryValue::String(e.form_state.image_coordinates.1.to_string()),
                );
            }
            return;
        }
        if name.is_empty() {
            return;
        }
        match (tag, kind.as_str()) {
            ("button", _) | ("input", "submit" | "reset" | "button") => {
                if submitter == Some(id) && e.is_submit_button() {
                    push(
                        entries,
                        name,
                        e.attr(local_name!("value")).unwrap_or("").into(),
                    );
                }
            }
            ("input", "checkbox" | "radio") => {
                if self.checkedness(id) {
                    push(
                        entries,
                        name,
                        e.attr(local_name!("value")).unwrap_or("on").into(),
                    );
                }
            }
            ("input", "file") => self.append_file_entries(id, name, entries),
            ("input", "hidden") if name.eq_ignore_ascii_case("_charset_") => {
                push(entries, name, "UTF-8".into());
            }
            ("select", _) => {
                for option in self.selected_enabled_options(id) {
                    if entries.failed {
                        break;
                    }
                    push(
                        entries,
                        name,
                        EntryValue::String(self.form_value(option).unwrap_or_default()),
                    );
                }
            }
            _ => push(
                entries,
                name,
                EntryValue::String(self.form_value(id).unwrap_or_default()),
            ),
        }
    }

    fn append_file_entries(&self, id: NodeId, name: &str, entries: &mut Entries) {
        if let Some(e) = self.nodes[id]
            .element_data()
            .filter(|e| !e.form_state.files.is_empty())
        {
            for file in e.form_state.files.iter() {
                if entries.failed {
                    break;
                }
                push(entries, name, EntryValue::FileContents(file.clone()));
            }
            return;
        }
        #[cfg(feature = "file-input")]
        if let Some(files) = self.nodes[id]
            .element_data()
            .and_then(|e| e.file_data())
            .filter(|f| !f.is_empty())
        {
            for path in files.iter() {
                if entries.failed {
                    break;
                }
                push(entries, name, EntryValue::File(path.clone()));
            }
            return;
        }
        #[cfg(not(feature = "file-input"))]
        let _ = id;
        push(entries, name, EntryValue::EmptyFile);
    }
}

#[derive(Default)]
struct Entries {
    data: FormData,
    bytes: usize,
    failed: bool,
}

fn push(entries: &mut Entries, name: &str, value: EntryValue) {
    let value_size = match &value {
        EntryValue::String(s) => s.len(),
        EntryValue::File(p) => p.as_os_str().len(),
        EntryValue::FileContents(f) => f.name.len().saturating_add(f.content_type.len()),
        EntryValue::EmptyFile => 0,
    };
    let size = name.len().saturating_add(value_size);
    if entries.failed
        || entries.data.len() >= 16_384
        || size > (16usize * 1024 * 1024).saturating_sub(entries.bytes)
    {
        entries.failed = true;
        return;
    }
    entries.bytes += size;
    entries.data.0.push(Entry {
        name: name.to_owned(),
        value,
    });
}
