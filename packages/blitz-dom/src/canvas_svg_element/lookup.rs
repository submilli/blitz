//! What a reference's IDs and `url()`s resolve to.

use super::MAX_ATTRIBUTE_LEN;
use crate::canvas_svg_filter::CanvasSvgLimit;
use crate::{BaseDocument, NodeId};
use std::collections::HashSet;

/// Stylesheets whose URLs a local `url(#id)` may have resolved against.
const MAX_STYLESHEET_URLS: usize = 256;

impl BaseDocument {
    /// The first element with ID `id` in `owner`'s tree, as
    /// [`Self::find_svg_reference`] finds it, through the document's ID index
    /// (which leaves shadow trees to the scan).
    pub(super) fn svg_element_by_id(
        &self,
        owner: NodeId,
        id: &str,
    ) -> Result<Option<NodeId>, CanvasSvgLimit> {
        const MAX_CANDIDATES: usize = 16;
        let tree = self.tree_scope(owner)?;
        if self.nodes[tree].is_shadow_root() {
            return self.find_svg_reference(owner, id);
        }
        let Some(candidates) = self.nodes_to_id.get(id) else {
            return Ok(None);
        };
        if candidates.len() > MAX_CANDIDATES {
            return self.find_svg_reference(owner, id);
        }
        let mut first: Option<Vec<NodeId>> = None;
        for &candidate in candidates {
            if self.tree_scope(candidate)? != tree {
                continue;
            }
            let chain = self.ancestry(candidate)?;
            let earlier = match &first {
                Some(first) => self.precedes(&chain, first)?,
                None => true,
            };
            if earlier {
                first = Some(chain);
            }
        }
        Ok(first.and_then(|chain| chain.last().copied()))
    }

    /// The root of `node`'s tree: its shadow root, else its document.
    fn tree_scope(&self, node: NodeId) -> Result<NodeId, CanvasSvgLimit> {
        let top = self.ancestry(node)?[0];
        Ok(if self.nodes[top].is_shadow_root() {
            top
        } else {
            self.node_document(node)
        })
    }

    /// `node` and its ancestors, the topmost first.
    fn ancestry(&self, node: NodeId) -> Result<Vec<NodeId>, CanvasSvgLimit> {
        const MAX_ANCESTORS: usize = 4096;
        let mut chain = vec![node];
        let mut current = node;
        while let Some(parent) = self.nodes[current].parent {
            if chain.len() >= MAX_ANCESTORS {
                return Err(CanvasSvgLimit);
            }
            chain.push(parent);
            current = parent;
        }
        chain.reverse();
        Ok(chain)
    }

    /// Whether the node ending ancestry `a` comes before the one ending `b`
    /// in tree order; both start at the same root.
    fn precedes(&self, a: &[NodeId], b: &[NodeId]) -> Result<bool, CanvasSvgLimit> {
        const MAX_SIBLINGS: usize = 1 << 16;
        let shared = a.iter().zip(b).take_while(|(a, b)| a == b).count();
        let (Some(&a_branch), Some(&b_branch)) = (a.get(shared), b.get(shared)) else {
            // One is the other's ancestor, which comes first.
            return Ok(a.len() < b.len());
        };
        let Some(&parent) = shared.checked_sub(1).and_then(|index| a.get(index)) else {
            return Err(CanvasSvgLimit);
        };
        for &child in self.nodes[parent].children.iter().take(MAX_SIBLINGS) {
            if child == a_branch {
                return Ok(true);
            }
            if child == b_branch {
                return Ok(false);
            }
        }
        Err(CanvasSvgLimit)
    }
}
/// The URLs, without fragments, a local `url(#id)` may be resolved against:
/// the document's, its base, the presentation attributes' and the document's
/// stylesheets'. Longer ones than an attribute the snapshot reads are left
/// out, so each lookup hashes a bounded key.
pub(super) fn local_urls(doc: &BaseDocument, root: NodeId) -> HashSet<String> {
    let mut urls = vec![
        doc.document_url(root),
        doc.document_base_url(root),
        doc.nodes[root].svg_presentation_base(),
    ];
    let guard = doc.guard.read();
    let sheets = doc.nodes_to_stylesheet.values().take(MAX_STYLESHEET_URLS);
    for sheet in sheets {
        if let Some(contents) = sheet.0.contents.try_read_with(&guard) {
            urls.push((*contents.url_data.0).clone());
        }
    }
    urls.iter()
        .map(|url| &url[..url::Position::AfterQuery])
        .filter(|url| url.len() <= MAX_ATTRIBUTE_LEN)
        .map(str::to_owned)
        .collect()
}
