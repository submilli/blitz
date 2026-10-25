//! Passive stylesheet objects for connected trees without a browsing context.
use super::*;
use style::{media_queries::MediaList, stylesheets::Stylesheet};

impl BaseDocument {
    /// Preserve readable CSSOM rules without imports, fonts or cascade attachment.
    /// Active insertion reparses through normal stylesheet processing.
    pub(crate) fn process_inert_style_element(&mut self, owner: NodeId) {
        if !self.style_element_accepts_css(owner) {
            self.clear_owner_stylesheet(owner);
            return;
        }
        let css = self.nodes[owner].text_content();
        let css = html_escape::decode_html_entities(&css);
        let url = self.style_base_url(owner);
        let loader = DetachedStylesheetLoader {
            url_data: url.clone(),
        };
        let sheet = Stylesheet::from_str(
            crate::css_limits::bounded(&css),
            url,
            Origin::Author,
            ServoArc::new(self.guard.wrap(MediaList::empty())),
            self.guard.clone(),
            Some(&loader),
            None,
            QuirksMode::NoQuirks,
            AllowImportRules::Yes,
        );
        if let Some(element) = self.nodes[owner].element_data_mut() {
            element.stylesheet_origin_clean = true;
            element.special_data = crate::node::SpecialElementData::Stylesheet(DocumentStyleSheet(
                ServoArc::new(sheet),
            ));
        }
        self.update_owner_stylesheet_media(owner);
    }
}
#[cfg(test)]
mod tests;
