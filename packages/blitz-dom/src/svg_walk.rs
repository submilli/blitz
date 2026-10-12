//! Visiting everything a parsed SVG document keeps.

/// What [`walk`] visits.
pub(crate) enum Visit<'a> {
    /// A document: the parsed one, or one nested as an image.
    Tree(&'a usvg::Tree),
    /// A node of a document.
    Node(&'a usvg::Node),
}

/// Visit every document `tree` holds and every node it keeps, in document
/// order (each node before its children, a nested document where its image
/// is): those it renders, then those of its clip paths, masks, patterns and
/// `feImage` primitives, with flattened text and nested documents in place.
/// The walk keeps its own stack, as pages choose how deep trees nest.
pub(crate) fn walk<'a>(tree: &'a usvg::Tree, mut visit: impl FnMut(Visit<'a>)) {
    let mut stack: Vec<std::slice::Iter<'a, usvg::Node>> = Vec::new();
    enter(tree, &mut visit, &mut stack);
    while let Some(nodes) = stack.last_mut() {
        let Some(node) = nodes.next() else {
            stack.pop();
            continue;
        };
        visit(Visit::Node(node));
        match node {
            usvg::Node::Group(group) => stack.push(group.children().iter()),
            usvg::Node::Text(text) => stack.push(text.flattened().children().iter()),
            usvg::Node::Image(image) => {
                if let usvg::ImageKind::SVG(nested) = image.kind() {
                    enter(nested, &mut visit, &mut stack);
                }
            }
            usvg::Node::Path(_) => {}
        }
    }
}

/// Visit `tree` and stack its groups to walk, the rendered root on top.
fn enter<'a>(
    tree: &'a usvg::Tree,
    visit: &mut impl FnMut(Visit<'a>),
    stack: &mut Vec<std::slice::Iter<'a, usvg::Node>>,
) {
    visit(Visit::Tree(tree));
    let mut groups = vec![tree.root()];
    groups.extend(tree.clip_paths().iter().map(|clip| clip.root()));
    groups.extend(tree.masks().iter().map(|mask| mask.root()));
    groups.extend(tree.patterns().iter().map(|pattern| pattern.root()));
    for primitive in tree.filters().iter().flat_map(|filter| filter.primitives()) {
        if let usvg::filter::Kind::Image(image) = primitive.kind() {
            groups.push(image.root());
        }
    }
    stack.extend(
        groups
            .into_iter()
            .rev()
            .map(|group| group.children().iter()),
    );
}
