//! HTML actual disabledness, evaluated against current ancestry.
use crate::{Node, ns};
use markup5ever::local_name;
use style_dom::ElementState;

impl Node {
    /// HTML disabledness is distinct from the presence of a disabled attribute.
    /// <https://html.spec.whatwg.org/multipage/semantics-other.html#disabled-elements>
    pub fn is_disabled(&self) -> bool {
        self.disabled_with_fieldset(|| self.disabled_by_fieldset())
    }

    pub(crate) fn is_disabled_with_ancestors(&self, disabled: bool) -> bool {
        self.disabled_with_fieldset(|| disabled)
    }

    fn disabled_with_fieldset(&self, fieldset: impl FnOnce() -> bool) -> bool {
        let Some(element) = self.element_data().filter(|e| e.can_be_disabled()) else {
            return false;
        };
        if element.has_attr(local_name!("disabled")) {
            return true;
        }
        match &*element.name.local {
            "optgroup" => false,
            "option" => self.parent.is_some_and(|parent| {
                let parent = self.with(parent);
                parent.is_html_tag("optgroup")
                    && parent
                        .element_data()
                        .is_some_and(|e| e.has_attr(local_name!("disabled")))
            }),
            _ => fieldset(),
        }
    }

    /// Project ancestry-dependent state without caching it across tree mutations.
    pub(crate) fn effective_element_state(&self) -> ElementState {
        let mut state = *self.element_state();
        if self.element_data().is_some_and(|e| e.can_be_disabled()) {
            let disabled = self.is_disabled();
            state.set(ElementState::DISABLED, disabled);
            state.set(ElementState::ENABLED, !disabled);
        }
        state
    }

    fn disabled_by_fieldset(&self) -> bool {
        let mut child = self;
        while let Some(parent) = child.parent {
            let parent = self.with(parent);
            if parent.fieldset_disables(child) {
                return true;
            }
            child = parent;
        }
        false
    }

    pub(crate) fn fieldset_disables(&self, child: &Node) -> bool {
        self.is_html_tag("fieldset")
            && self
                .element_data()
                .is_some_and(|e| e.has_attr(local_name!("disabled")))
            && (!child.is_html_tag("legend")
                || self.children.first_legend(self.tree()) != Some(child.id))
    }

    fn is_html_tag(&self, name: &str) -> bool {
        self.element_data()
            .is_some_and(|e| e.name.ns == ns!(html) && &*e.name.local == name)
    }
}
