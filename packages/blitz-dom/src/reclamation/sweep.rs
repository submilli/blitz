use std::collections::HashSet;

use crate::dom_events::EventTargetId;
use crate::{BaseDocument, NodeId};

impl BaseDocument {
    /// Release detached native nodes not reachable from engine or embedder roots.
    ///
    /// The embedder must supply every node reachable from its wrappers, parser,
    /// and suspended continuations at an exclusive checkpoint. The graph is
    /// recomputed here; a previously obtained snapshot cannot authorize removal.
    /// This does not dispatch events, process messages, or run layout. Affected
    /// derived caches are scrubbed before slots are freed and marked for rebuild.
    /// Returned generation IDs can be used to prune embedder caches.
    pub fn reclaim_detached_nodes(&mut self, embedder_roots: &[NodeId]) -> Vec<NodeId> {
        let graph = self.node_reachability();
        let live = graph.retained_nodes(embedder_roots);
        let dead: Vec<_> = graph
            .nodes
            .into_iter()
            .filter(|id| !live.contains(id))
            .collect();
        if dead.is_empty() {
            return dead;
        }
        super::caches::prune(self, &live);
        self.prune_reclaimed_indexes(&live);
        for &id in &dead {
            self.event_listeners.remove_target(EventTargetId::Node(id));
            self.nodes.remove(id);
        }
        dead
    }

    fn prune_reclaimed_indexes(&mut self, live: &HashSet<NodeId>) {
        self.nodes_to_id.retain(|_, ids| {
            ids.retain(|id| live.contains(id));
            !ids.is_empty()
        });
        self.controls_to_form
            .retain(|control, form| live.contains(control) && live.contains(form));
        self.changed_nodes.retain(|id| live.contains(id));
        self.shadow_hosts.retain(|id| live.contains(id));
        self.scrollbar_activity.retain(|id, _| live.contains(id));
        self.snapshots
            .retain(|node, _| live.contains(&NodeId::from_u64(node.id() as u64)));
        self.animations
            .sets
            .write()
            .retain(|key, _| live.contains(&NodeId::from_u64(key.node.id() as u64)));
        let guard = self.guard.read();
        let previous_sheets = self.nodes_to_stylesheet.len();
        self.nodes_to_stylesheet.retain(|id, sheet| {
            if live.contains(id) {
                return true;
            }
            self.stylist.remove_stylesheet(sheet.clone(), &guard);
            false
        });
        if previous_sheets != self.nodes_to_stylesheet.len() {
            self.stylesheet_generation += 1;
            self.stylist
                .force_stylesheet_origins_dirty(style::stylesheets::OriginSet::all());
        }
        for &id in live {
            if let Some(styles) = &mut self.nodes[id].shadow_styles {
                let before = styles.sheets.len();
                styles.sheets.retain(|id, _| live.contains(id));
                styles.dirty |= before != styles.sheets.len();
            }
        }
    }
}
