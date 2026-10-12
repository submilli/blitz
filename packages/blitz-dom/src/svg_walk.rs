//! Visiting everything a parsed SVG document keeps.

/// What [`walk`] visits.
pub(crate) enum Visit<'a> {
    /// A document: the parsed one, or one nested as an image.
    Tree(&'a usvg::Tree),
    /// A node of a document.
    Node(&'a usvg::Node),
}

/// Visit every document `tree` holds and every node it keeps: those it renders,
/// and those of its clip paths, masks, patterns, `feImage` primitives,
/// flattened text and nested documents.
pub(crate) fn walk<'a>(tree: &'a usvg::Tree, mut visit: impl FnMut(Visit<'a>)) {
    let mut trees = vec![tree];
    let mut groups: Vec<&usvg::Group> = Vec::new();
    while let Some(tree) = trees.pop() {
        visit(Visit::Tree(tree));
        groups.push(tree.root());
        groups.extend(tree.clip_paths().iter().map(|clip| clip.root()));
        groups.extend(tree.masks().iter().map(|mask| mask.root()));
        groups.extend(tree.patterns().iter().map(|pattern| pattern.root()));
        for primitive in tree.filters().iter().flat_map(|filter| filter.primitives()) {
            if let usvg::filter::Kind::Image(image) = primitive.kind() {
                groups.push(image.root());
            }
        }
        while let Some(group) = groups.pop() {
            for node in group.children() {
                visit(Visit::Node(node));
                match node {
                    usvg::Node::Group(child) => groups.push(child),
                    usvg::Node::Text(text) => groups.push(text.flattened()),
                    usvg::Node::Image(image) => {
                        if let usvg::ImageKind::SVG(nested) = image.kind() {
                            trees.push(nested);
                        }
                    }
                    usvg::Node::Path(_) => {}
                }
            }
        }
    }
}
