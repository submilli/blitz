//! Cancel undeliverable preparation notifications without changing script state.
use crate::BaseDocument;

impl BaseDocument {
    /// Discard queued connected-script notifications when their document retires.
    /// Unlike taking scripts for preparation, cancellation must not mark them as
    /// started. Independently held native leases continue to retain their nodes.
    pub fn discard_connected_scripts(&mut self) {
        self.connected_scripts.clear();
    }
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
}
