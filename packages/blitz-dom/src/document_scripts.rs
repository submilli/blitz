//! Own and cancel preparation candidates across reentrant post-connection steps.
use crate::{BaseDocument, NodeId, NodeLease};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScriptType {
    Classic,
    Module,
    ImportMap,
}

impl BaseDocument {
    /// Snapshot post-connection steps before calling script. A later element
    /// stays eligible for preparation if an earlier callback reconnects it.
    pub fn take_connected_script_candidates(&mut self) -> Vec<NodeLease> {
        std::mem::take(&mut self.connected_scripts)
            .into_iter()
            .map(|id| self.lease_node(id))
            .collect()
    }

    /// Empty classic scripts and unrecognized types stay unstarted. The module
    /// loader accepts an empty inline module as a graph root. A recognized
    /// classic nomodule script is started but skipped.
    pub fn start_script_preparation(&mut self, node: NodeId, parser: bool) -> Option<ScriptType> {
        let element = self.nodes.get_mut(node)?.element_data_mut()?;
        if element.script_state.already_started
            || (!parser && element.script_state.parser_inserted)
            || element.name.ns != crate::ns!(html)
            || element.name.local.as_ref() != "script"
        {
            return None;
        }
        // A parser that returns before preparation relinquishes this candidate.
        if parser {
            element.script_state.parser_inserted = false;
        }
        let native = self.get_node(node)?;
        let element = native.element_data()?;
        let has_source = element.has_attr(crate::local_name!("src")) || native.children.iter().any(|&child| {
            matches!(&self.nodes[child].data, crate::node::NodeData::Text(text) if !text.content.is_empty())
        });
        let kind = script_type(element)?;
        if !self.is_connected(node) {
            return None;
        }
        if !has_source {
            if kind != ScriptType::Module || element.script_state.empty_module_reported {
                return None;
            }
            self.nodes[node]
                .element_data_mut()?
                .script_state
                .empty_module_reported = true;
            return Some(kind);
        }
        let suppressed =
            kind == ScriptType::Classic && element.has_attr(crate::local_name!("nomodule"));
        let flags = &mut self.nodes[node].element_data_mut()?.script_state;
        flags.already_started = true;
        flags.parser_inserted = parser;
        (!suppressed).then_some(kind)
    }

    /// Inline script source consists of direct Text children in tree order.
    pub fn script_source(&self, node: NodeId) -> String {
        self.get_node(node)
            .map(|node| node.child_text_content().as_str_lossy().to_owned())
            .unwrap_or_default()
    }

    /// Discard queued connected-script notifications when their document retires.
    /// Unlike taking scripts for preparation, cancellation must not mark them as
    /// started. Independently held native leases continue to retain their nodes.
    pub fn discard_connected_scripts(&mut self) {
        self.connected_scripts.clear();
    }
}

fn script_type(element: &crate::node::ElementData) -> Option<ScriptType> {
    if let Some(kind) = element.attr(crate::local_name!("type")) {
        let kind = kind.trim().to_ascii_lowercase();
        return match kind.as_str() {
            "module" => Some(ScriptType::Module),
            "importmap" => Some(ScriptType::ImportMap),
            "" => Some(ScriptType::Classic),
            _ => javascript_mime(&kind).then_some(ScriptType::Classic),
        };
    }
    let language = element
        .attr(crate::local_name!("language"))
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    (language.is_empty()
        || language.starts_with("javascript")
        || matches!(language.as_str(), "jscript" | "ecmascript"))
    .then_some(ScriptType::Classic)
}

/// Match a lower-case JavaScript MIME essence.
pub fn javascript_mime(kind: &str) -> bool {
    matches!(
        kind.split(';').next().unwrap_or("").trim(),
        "text/javascript"
            | "application/javascript"
            | "application/ecmascript"
            | "application/x-ecmascript"
            | "application/x-javascript"
            | "text/ecmascript"
            | "text/javascript1.0"
            | "text/javascript1.1"
            | "text/javascript1.2"
            | "text/javascript1.3"
            | "text/javascript1.4"
            | "text/javascript1.5"
            | "text/jscript"
            | "text/livescript"
            | "text/x-ecmascript"
            | "text/x-javascript"
    )
}

#[cfg(test)]
mod tests {
    use crate::{BaseDocument, QualName, ns};

    #[test]
    fn cancellation_preserves_unstarted_scripts_and_independent_leases() {
        let mut document = BaseDocument::new(Default::default());
        let root = document.root_node().id;
        let scope = document
            .mutate()
            .try_create_element(QualName::new(None, ns!(html), "div".into()), Vec::new())
            .unwrap();
        let script = document
            .mutate()
            .try_create_element(QualName::new(None, ns!(html), "script".into()), Vec::new())
            .unwrap();
        document.mutate().append_children(root, &[scope]);
        document.mutate().append_children(scope, &[script]);
        assert!(document.node_reachability().roots.contains(&script));
        let pin = document.lease_node(script);
        document.discard_connected_scripts();
        assert!(document.node_reachability().roots.contains(&script));
        assert!(
            !document
                .get_node(script)
                .unwrap()
                .element_data()
                .unwrap()
                .script_state
                .already_started
        );
        drop(pin);
        assert!(!document.node_reachability().roots.contains(&script));
        assert!(document.take_connected_scripts().is_empty());
        document.mutate().remove_child(scope, script).unwrap();
        document.mutate().append_children(scope, &[script]);
        assert_eq!(document.take_connected_scripts(), [script]);
        assert!(
            document
                .get_node(script)
                .unwrap()
                .element_data()
                .unwrap()
                .script_state
                .already_started
        );
    }
    #[test]
    fn empty_module_handle_does_not_claim_later_source() {
        use super::ScriptType;
        let mut document = BaseDocument::new(Default::default());
        let root = document.root_node().id;
        let mut tree = document.mutate();
        let script = tree.create_element(QualName::new(None, ns!(html), "script".into()), vec![]);
        tree.set_attribute(script, QualName::new(None, ns!(), "type".into()), "module");
        tree.append_children(root, &[script]);
        drop(tree);

        assert_eq!(
            document.start_script_preparation(script, true),
            Some(ScriptType::Module)
        );
        assert_eq!(document.start_script_preparation(script, false), None);
        let text = document.mutate().create_text_node("export {}");
        document.mutate().append_children(script, &[text]);
        assert_eq!(
            document.start_script_preparation(script, false),
            Some(ScriptType::Module)
        );
        assert_eq!(document.start_script_preparation(script, false), None);
    }

    #[test]
    fn connection_candidates_start_individually_and_import_preserves_script_history() {
        use crate::foreign_import::ImportMode;
        let mut source = BaseDocument::new(Default::default());
        let root = source.root_node().id;
        let mut tree = source.mutate();
        let host = tree.create_element(QualName::new(None, ns!(html), "div".into()), vec![]);
        let first = tree.create_element(QualName::new(None, ns!(html), "script".into()), vec![]);
        let next = tree.create_element(QualName::new(None, ns!(html), "script".into()), vec![]);
        for script in [first, next] {
            let text = tree.create_text_node("1");
            tree.append_children(script, &[text]);
        }
        tree.append_children(host, &[first, next]);
        tree.append_children(root, &[host]);
        drop(tree);

        let candidates = source.take_connected_script_candidates();
        assert_eq!(
            candidates.iter().map(|pin| pin.id()).collect::<Vec<_>>(),
            [first, next]
        );
        assert!(source.start_script_preparation(first, false).is_some());
        source.mutate().remove_node(next);
        assert!(!source.start_script_preparation(next, false).is_some());
        let mut target = BaseDocument::new(Default::default());
        let dest = target.root_node().id;
        for (old, mode, should_run) in [
            (first, ImportMode::Copy { deep: true }, false),
            (next, ImportMode::Copy { deep: true }, true),
            (next, ImportMode::Transfer, true),
        ] {
            let new = target
                .mutate()
                .try_import_foreign(&source, old, dest, mode)
                .unwrap()
                .root();
            target.mutate().append_children(dest, &[new]);
            assert_eq!(
                target.start_script_preparation(new, false).is_some(),
                should_run
            );
            assert!(!target.start_script_preparation(new, false).is_some());
        }
        source.mutate().append_children(host, &[next]);
        assert!(source.start_script_preparation(next, false).is_some());
        assert!(!source.start_script_preparation(next, false).is_some());
    }
    #[test]
    fn importing_an_unprepared_parser_script_releases_only_the_copys_parser_ownership() {
        use crate::foreign_import::ImportMode;
        let mut source = BaseDocument::new(Default::default());
        let parent = source.root_node().id;
        let mut tree = source.mutate();
        let script = tree.create_element(QualName::new(None, ns!(html), "script".into()), vec![]);
        tree.mark_script(script, true, false);
        let text = tree.create_text_node("1");
        tree.append_children(script, &[text]);
        tree.append_children(parent, &[script]);
        drop(tree);

        let mut target = BaseDocument::new(Default::default());
        let root = target.root_node().id;
        let copy = target
            .mutate()
            .try_import_foreign(&source, script, root, ImportMode::Copy { deep: true })
            .unwrap()
            .root();
        target.mutate().append_children(root, &[copy]);
        assert!(target.start_script_preparation(copy, false).is_some());
        assert!(target.start_script_preparation(copy, false).is_none());
        assert!(source.start_script_preparation(script, false).is_none());
        assert!(source.start_script_preparation(script, true).is_some());
    }
}
