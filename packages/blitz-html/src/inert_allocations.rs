//! Release recovery nodes that the tree builder detached from an inert result.
use blitz_dom::{DocumentMutator, NodeId};
use std::collections::HashSet;

pub(crate) fn discard_detached_allocations(
    m: &mut DocumentMutator<'_>,
    document: NodeId,
    allocated: &[NodeId],
) {
    // Parentless template contents are owned through their element. Ordinary
    // parent checks would wrongly discard them together with recovery garbage.
    let mut owned = HashSet::new();
    let mut work = vec![document, m.doc.template_document(document)];
    while let Some(id) = work.pop() {
        if !owned.insert(id) {
            continue;
        }
        let Some(node) = m.doc.get_node(id) else {
            continue;
        };
        work.extend(node.children.iter().copied());
        if let Some(element) = node.element_data() {
            work.extend(element.template_contents);
            work.extend(element.shadow_root);
        }
    }
    for &id in allocated {
        if !owned.contains(&id) && m.doc.get_node(id).is_some() {
            m.remove_and_drop_node(id);
        }
    }
}
