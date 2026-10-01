//! Image-button resource identity and CSSOM dimensions.
use crate::node::{ImageData, SpecialElementData};
use crate::{BaseDocument, DocumentMutator, ElementData, NodeId};
use markup5ever::{QualName, local_name, ns};
use style::values::specified::box_::{DisplayInside, DisplayOutside};

impl ElementData {
    pub(crate) fn is_image_input(&self) -> bool {
        self.name.ns == ns!(html)
            && self.name.local == local_name!("input")
            && self
                .attr(local_name!("type"))
                .is_some_and(|v| v.eq_ignore_ascii_case("image"))
    }
}

impl DocumentMutator<'_> {
    pub(crate) fn image_input_attribute_changed(&mut self, id: NodeId, name: &QualName) {
        if name.ns == ns!() && matches!(name.local.as_ref(), "src" | "type") {
            self.update_image_input(id, name.local == local_name!("src"));
        }
    }

    pub(crate) fn update_image_input(&mut self, id: NodeId, source_changed: bool) {
        let Some(node) = self.doc.get_node(id) else {
            return;
        };
        let Some(element) = node
            .element_data()
            .filter(|e| e.name.ns == ns!(html) && e.name.local == local_name!("input"))
        else {
            return;
        };
        let source = element
            .is_image_input()
            .then(|| element.attr(local_name!("src")))
            .flatten()
            .filter(|s| !s.is_empty())
            .map(|s| {
                self.doc
                    .resolve_url(s)
                    .map_or_else(|| s.to_string(), |u| u.to_string())
            });
        let is_image = element.is_image_input();
        let has_source = element
            .attr(local_name!("src"))
            .is_some_and(|s| !s.is_empty());
        let previous = element.form_state.image_input_source.clone();
        if previous != source {
            self.doc.failed_image_inputs.remove(&id);
            if let Some(old_url) = previous.as_deref() {
                self.doc.remove_image_waiter(id, old_url);
            }
            let node = &mut self.doc.nodes[id];
            let element = node.element_data_mut().expect("image input is an element");
            element.form_state.image_input_source = source.clone();
            if !has_source {
                element.form_state.image_input_image = None;
            }
            if (!is_image || !has_source)
                && matches!(element.special_data, SpecialElementData::Image(_))
            {
                element.special_data = SpecialElementData::None;
                node.clear_layout_cache();
                node.insert_damage(crate::layout::damage::ALL_DAMAGE);
            }
        }
        if is_image && has_source {
            let element = self.doc.nodes[id]
                .element_data_mut()
                .expect("image input is an element");
            if let Some(image) = &element.form_state.image_input_image {
                element.special_data = SpecialElementData::Image(image.clone());
            }
        }
        // Image buttons load even while detached; decoded images survive removal.
        if let Some(source) = source {
            if self.doc.resolve_url(&source).is_some() {
                self.load_image(id);
            } else if source_changed
                || self.doc.nodes[id]
                    .element_data()
                    .is_none_or(|e| e.form_state.image_input_image.is_none())
            {
                self.doc.failed_image_inputs.insert(id, source);
            }
        }
    }
}

impl BaseDocument {
    pub(crate) fn fail_image_input(&mut self, id: NodeId, url: &str) {
        let Some(node) = self.nodes.get_mut(id) else {
            return;
        };
        let Some(element) = node.element_data_mut().filter(|e| {
            e.is_image_input() && e.form_state.image_input_source.as_deref() == Some(url)
        }) else {
            return;
        };
        element.form_state.image_input_image = None;
        element.special_data = SpecialElementData::None;
        node.clear_layout_cache();
        node.insert_damage(crate::layout::damage::ALL_DAMAGE);
    }

    /// Image-button dimensions after resolving layout. With no decoded image,
    /// or no CSS box, HTML uses the dimension attributes (then intrinsic size).
    pub fn image_input_dimensions(&self, id: NodeId) -> (u32, u32) {
        let Some(node) = self.get_node(id) else {
            return (0, 0);
        };
        let Some(element) = node.element_data().filter(|e| e.is_image_input()) else {
            return (0, 0);
        };
        let intrinsic = match element.form_state.image_input_image.as_deref() {
            Some(ImageData::Raster(image)) => Some((image.width, image.height)),
            #[cfg(feature = "svg")]
            Some(ImageData::Svg(image)) => Some((
                image.tree.size().width() as u32,
                image.tree.size().height() as u32,
            )),
            _ => None,
        };
        if intrinsic.is_some() && self.image_input_has_box(id) {
            let layout = node.unrounded_layout();
            return (
                layout.content_box_width().round().max(0.0) as u32,
                layout.content_box_height().round().max(0.0) as u32,
            );
        }
        let (width, height) = intrinsic.unwrap_or_default();
        let dimension = |name, fallback| {
            element
                .attr(name)
                .and_then(|v| style::servo::attr::parse_unsigned_integer(v.chars()).ok())
                .filter(|&v| v <= i32::MAX as u32)
                .unwrap_or(fallback)
        };
        (
            dimension(local_name!("width"), width),
            dimension(local_name!("height"), height),
        )
    }

    fn image_input_has_box(&self, id: NodeId) -> bool {
        let Some(node) = self.get_node(id).filter(|n| n.flags.is_in_document()) else {
            return false;
        };
        if node.primary_styles().is_none() {
            return false;
        }
        let target = id;
        let mut current = Some(id);
        while let Some(id) = current {
            let node = &self.nodes[id];
            if node.primary_styles().is_some_and(|s| {
                s.clone_display().outside() == DisplayOutside::None
                    && (node.id == target || s.clone_display().inside() != DisplayInside::Contents)
            }) {
                return false;
            }
            current = node.parent;
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::DocumentConfig;

    #[test]
    fn changing_sources_keeps_only_current_waiters_even_without_completions() {
        // The default provider drops requests, as a saturated host queue may do.
        let mut doc = BaseDocument::new(DocumentConfig {
            base_url: Some("https://example.test/".into()),
            ..Default::default()
        });
        let input = doc
            .mutate()
            .create_element(crate::qual_name!("input", html), vec![]);
        doc.mutate()
            .set_attribute_by_name(input, "type", "image")
            .unwrap();
        for index in 0..2048 {
            doc.mutate()
                .set_attribute_by_name(input, "src", format!("{index}.svg"))
                .unwrap();
            assert_eq!(doc.pending_images.len(), 1);
            assert_eq!(
                doc.pending_images
                    .values()
                    .map(|v| v.waiters.len())
                    .sum::<usize>(),
                1
            );
        }
        for _ in 0..2048 {
            doc.mutate()
                .set_attribute_by_name(input, "src", "2047.svg")
                .unwrap();
        }
        assert_eq!(
            doc.pending_images
                .values()
                .map(|v| v.waiters.len())
                .sum::<usize>(),
            1
        );
        doc.mutate().remove_attribute_by_name(input, "src");
        assert!(doc.pending_images.is_empty());
        for index in 0..2048 {
            doc.mutate()
                .set_attribute_by_name(input, "src", format!("http://[{index}"))
                .unwrap();
            assert_eq!(doc.failed_image_inputs.len(), 1);
            assert!(doc.pending_images.is_empty());
        }
        doc.handle_messages();
        assert!(doc.failed_image_inputs.is_empty());
        doc.mutate()
            .set_attribute_by_name(input, "src", "http://[")
            .unwrap();
        assert_eq!(doc.failed_image_inputs.len(), 1);
        doc.mutate().remove_and_drop_node(input);
        assert!(doc.failed_image_inputs.is_empty());
    }
}
