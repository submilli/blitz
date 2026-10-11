//! Strongly connected components of a reference graph.

/// Strongly connected components of `edges` by Tarjan's algorithm, iterative
/// so that page-made chains cannot exhaust the stack. Returns each vertex's
/// component, and the components in Tarjan's order: each after every
/// component it reaches.
pub(super) fn components(edges: &[Vec<usize>]) -> (Vec<usize>, Vec<Vec<usize>>) {
    const UNSEEN: usize = usize::MAX;
    let count = edges.len();
    let mut index = vec![UNSEEN; count];
    let mut low = vec![0; count];
    let mut on_stack = vec![false; count];
    let mut stack = Vec::new();
    let mut component = vec![UNSEEN; count];
    let mut order: Vec<Vec<usize>> = Vec::new();
    let mut next = 0;
    for start in 0..count {
        if index[start] != UNSEEN {
            continue;
        }
        let mut calls = vec![(start, 0)];
        index[start] = next;
        low[start] = next;
        next += 1;
        stack.push(start);
        on_stack[start] = true;
        while let Some((vertex, edge)) = calls.last_mut() {
            let vertex = *vertex;
            if let Some(&target) = edges[vertex].get(*edge) {
                *edge += 1;
                if index[target] == UNSEEN {
                    index[target] = next;
                    low[target] = next;
                    next += 1;
                    stack.push(target);
                    on_stack[target] = true;
                    calls.push((target, 0));
                } else if on_stack[target] {
                    low[vertex] = low[vertex].min(index[target]);
                }
                continue;
            }
            calls.pop();
            if let Some((parent, _)) = calls.last() {
                low[*parent] = low[*parent].min(low[vertex]);
            }
            if low[vertex] == index[vertex] {
                let mut members = Vec::new();
                loop {
                    let member = stack.pop().expect("a root's component is on the stack");
                    on_stack[member] = false;
                    component[member] = order.len();
                    members.push(member);
                    if member == vertex {
                        break;
                    }
                }
                order.push(members);
            }
        }
    }
    (component, order)
}

#[cfg(test)]
mod tests {
    use super::components;

    #[test]
    fn components_follow_what_they_reach() {
        // 0 → 1 ⇄ 2 → 3, and 4 alone with a loop.
        let edges = vec![vec![1], vec![2], vec![1, 3], vec![], vec![4]];
        let (component, order) = components(&edges);
        assert_eq!(component[1], component[2]);
        assert_ne!(component[0], component[1]);
        assert!(component[3] < component[1] && component[1] < component[0]);
        assert_eq!(order.iter().map(Vec::len).sum::<usize>(), 5);
        assert_eq!(order[component[4]], vec![4]);
    }

    #[test]
    fn long_chains_do_not_recurse() {
        let edges: Vec<Vec<usize>> = (0..100_000)
            .map(|vertex| vec![vertex + 1])
            .chain([vec![]])
            .collect();
        let (component, order) = components(&edges);
        assert_eq!(order.len(), 100_001);
        assert!(component[100_000] < component[0]);
    }
}
