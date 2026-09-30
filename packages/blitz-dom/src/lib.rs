//! The core DOM abstraction in Blitz
//!
//! This crate implements a flexible headless DOM ([`BaseDocument`]), which is designed to emebedded in and "driven" by external code. Most users will want
//! to use a wrapper:
//!
//!  - [`HtmlDocument`](https://docs.rs/blitz-html/latest/blitz_html/struct.HtmlDocument.html) from the [blitz-html](https://docs.rs/blitz-html) crate.
//!    Allows you to parse HTML (or XHTML) into a Blitz [`BaseDocument`], and can be combined with a markdown-to-html converter like [comrak](https://docs.rs/comrak)
//!    or [pulldown-cmark](https://docs.rs/pulldown-cmark) to render/process markdown.
//!  - [`DioxusDocument`](https://docs.rs/dioxus-native/latest/dioxus_native/struct.DioxusDocument.html) from the [dioxus-native](https://docs.rs/dioxus-native) crate.
//!    Combines a [`BaseDocument`] with a Dioxus `VirtualDom` to enable dynamic rendering and event handling.
//!
//! It includes: A DOM tree respresentation, CSS parsing and resolution, layout and event handling. Additional functionality is available in
//! separate crates, including html parsing ([blitz-html](https://docs.rs/blitz-html)), networking ([blitz-net](https://docs.rs/blitz-html)),
//! rendering ([blitz-paint](https://docs.rs/blitz-paint)) and windowing ([blitz-shell](https://docs.rs/blitz-shell)).
//!
//! Most of the functionality in this crates is provided through the  struct.
//!
//! `blitz-dom` has a native Rust API that is designed for higher-level abstractions to be built on top (although it can also be used directly).
//!
//! The goal behind this crate is that any implementor can interact with the DOM and render it out using any renderer
//! they want.
//!

#![allow(clippy::collapsible_if)]

// TODO: Document features
// ## Feature flags
//  - `default`: Enables the features listed below.
//  - `tracing`: Enables tracing support.

pub const DEFAULT_CSS: &str = include_str!("../assets/default.css");
pub const BULLET_FONT: &[u8] = include_bytes!("../assets/moz-bullet-font.otf");

pub mod custom_elements;
/// The DOM implementation.
///
/// This is the primary entry point for this crate.
mod document;
pub mod dom_api;
pub mod dom_events;
pub mod mutations;

/// The nodes themsleves, and their data.
pub mod node;
pub mod shadow;

mod clock;
mod config;
/// CSSOM stylesheet access (`document.styleSheets`, `CSSStyleSheet`, `CSSRule`)
mod cssom;
mod debug;
mod dom_string;
mod events;
mod font_metrics;
mod form;
mod html;
/// Loading of `<iframe>` elements into sub-documents.
mod iframe;
/// Integration of taffy and the DOM.
mod layout;
mod mutator;
mod query_selector;
mod resolve;
/// Computation of resolved CSS property values (`getComputedStyle()`)
mod resolved_style;
/// Scrolling of nodes and the viewport, and scroll animations.
mod scrolling;
mod selection;
mod serialize;
pub use dom_string::DomString;
/// Implementations that interact with servo's style engine
mod stylo;
mod stylo_device;
mod stylo_to_cursor_icon;
mod stylo_to_kurbo;
mod stylo_to_parley;
pub mod traversal;
/// Versioned storage for the nodes of the DOM tree.
mod tree;

mod url;

pub use cssom::{CssRuleInfo, CssomError, CssomSheet};
pub use resolved_style::{
    css_property_is_supported, parse_transform_matrix, resolved_style_property_names,
};
pub use stylo_to_kurbo::resolve_2d_transform;

pub mod net;
pub mod util;

#[cfg(feature = "accessibility")]
mod accessibility;

pub use crate::layout::paint_tree::{HoistedPaintChild, StackingContext, hoisted_child_position};
pub use crate::layout::replaced::IntrinsicSizes;
#[cfg(feature = "custom-widget")]
pub use crate::node::Widget;

pub use blitz_traits::node_id::NodeId;
// Re-export taffy: it is part of blitz-dom's public API (e.g. `Node::style`,
// `Node::final_layout`)
#[cfg(feature = "system-clock")]
pub use clock::SystemClock;
pub use clock::{Clock, FrozenClock};
pub use config::{DocumentConfig, StyleThreading};
pub use document::{BaseDocument, DocGuard, DocGuardMut, Document, PlainDocument};
pub use markup5ever::{
    LocalName, Namespace, NamespaceStaticSet, Prefix, PrefixStaticSet, QualName, local_name,
    namespace_prefix, namespace_url, ns,
};
pub use mutator::DocumentMutator;
pub use node::{Attribute, DocumentData, ElementData, Node, NodeData, TextNodeData};
/// The text engine, for embedders that shape text with the document's fonts.
pub use parley;
pub use parley::FontContext;
pub use query_selector::MAX_SELECTOR_NESTING;
pub use scrolling::{ScrollBehavior, ScrollLogicalPosition};
pub use tree::NodeTree;

/// Convert a Blitz [`NodeId`] into a [`taffy::NodeId`] (which wraps a `u64`).
#[inline]
pub fn taffy_node_id(id: NodeId) -> taffy::NodeId {
    taffy::NodeId::from(id.as_u64())
}

/// Convert a [`taffy::NodeId`] produced by [`taffy_node_id`] back into a Blitz [`NodeId`].
#[inline]
pub fn dom_node_id(id: taffy::NodeId) -> NodeId {
    NodeId::from_u64(u64::from(id))
}
pub use style::Atom;
pub use style::invalidation::element::restyle_hints::RestyleHint;
pub use style::media_queries::MediaType;
pub use style::stylist::RegisterCustomPropertyResult;
pub type SelectorList = selectors::SelectorList<style::selector_parser::SelectorImpl>;
pub use events::{EventDriver, EventHandler, NoopEventHandler};
pub use html::{DummyHtmlParserProvider, HtmlParserProvider};
pub use util::{Point, decode_font_bytes};

/// The role a supplied font plays for CSS generic families.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FontRole {
    /// `sans-serif`, and the fallback for `system-ui`, `cursive`, `fantasy`
    /// and unknown families.
    SansSerif,
    /// `serif`
    Serif,
    /// `monospace`
    Monospace,
}

/// Build a font context from supplied font files only (no system font
/// discovery), mapping CSS generic families to them by role. The bullet font
/// used for list markers is always registered.
///
/// This lets an embedder control exactly which fonts a document can use,
/// which is needed for deterministic rendering and for platforms without
/// system fonts.
pub fn build_font_ctx(fonts: &[(FontRole, &[u8])]) -> FontContext {
    use parley::fontique::{Blob, Collection, CollectionOptions, GenericFamily, SourceCache};
    use std::sync::Arc;

    let mut ctx = FontContext {
        source_cache: SourceCache::new_shared(),
        collection: Collection::new(CollectionOptions {
            shared: false,
            system_fonts: false,
        }),
    };
    ctx.collection
        .register_fonts(Blob::new(Arc::new(BULLET_FONT) as _), None);
    for &(role, data) in fonts {
        let decoded = decode_font_bytes(data).into_owned();
        let families: Vec<_> = ctx
            .collection
            .register_fonts(Blob::new(Arc::new(decoded) as _), None)
            .iter()
            .map(|(id, _)| *id)
            .collect();
        let generics: &[GenericFamily] = match role {
            FontRole::SansSerif => &[
                GenericFamily::SansSerif,
                GenericFamily::SystemUi,
                GenericFamily::UiSansSerif,
                GenericFamily::Cursive,
                GenericFamily::Fantasy,
                GenericFamily::UiRounded,
            ],
            FontRole::Serif => &[GenericFamily::Serif, GenericFamily::UiSerif],
            FontRole::Monospace => &[GenericFamily::Monospace, GenericFamily::UiMonospace],
        };
        for &generic in generics {
            ctx.collection
                .append_generic_families(generic, families.iter().copied());
        }
    }
    ctx
}

/// Convenience builder for the one-font case: produces a [`FontContext`] with
/// system-font discovery disabled and the supplied font registered as the
/// fallback for every generic family. The standard setup for WASM, where
/// browsers don't expose system fonts. WOFF/WOFF2 inputs are decoded
/// automatically.
pub fn build_single_font_ctx(font_data: &[u8]) -> FontContext {
    use parley::fontique::{Blob, Collection, CollectionOptions, GenericFamily, SourceCache};
    use std::sync::Arc;

    let mut ctx = FontContext {
        source_cache: SourceCache::new_shared(),
        collection: Collection::new(CollectionOptions {
            shared: false,
            system_fonts: false,
        }),
    };
    let decoded = decode_font_bytes(font_data).into_owned();
    let registered = ctx
        .collection
        .register_fonts(Blob::new(Arc::new(decoded) as _), None);
    let family_ids: Vec<_> = registered.iter().map(|(id, _)| *id).collect();
    for generic in [
        GenericFamily::SansSerif,
        GenericFamily::Serif,
        GenericFamily::Monospace,
        GenericFamily::SystemUi,
    ] {
        ctx.collection
            .append_generic_families(generic, family_ids.iter().copied());
    }
    ctx
}

/// Fonts for text inside SVG images, which usvg lays out itself.
#[cfg(feature = "svg")]
pub type SvgFontDb = std::sync::Arc<usvg::fontdb::Database>;

/// Build the SVG font database from the same supplied fonts as
/// [`build_font_ctx`], so text in SVG images uses only those fonts too. Pass
/// it as [`DocumentConfig::svg_fonts`]. Generic families with no font of
/// their role fall back to the first supplied font.
#[cfg(feature = "svg")]
pub fn build_svg_font_db(fonts: &[(FontRole, &[u8])]) -> SvgFontDb {
    let mut db = usvg::fontdb::Database::new();
    let mut families: Vec<(FontRole, String)> = Vec::new();
    for &(role, data) in fonts {
        let before = db.len();
        db.load_font_data(decode_font_bytes(data).into_owned());
        let family = db
            .faces()
            .skip(before)
            .find_map(|face| face.families.first().map(|(name, _)| name.clone()));
        if let Some(family) = family
            && !families.iter().any(|(r, _)| *r == role)
        {
            families.push((role, family));
        }
    }
    let family_for = |role: FontRole| {
        families
            .iter()
            .find(|(r, _)| *r == role)
            .or_else(|| families.iter().find(|(r, _)| *r == FontRole::SansSerif))
            .or(families.first())
            .map(|(_, family)| family.clone())
    };
    if let Some(family) = family_for(FontRole::SansSerif) {
        db.set_sans_serif_family(family.clone());
        db.set_cursive_family(family.clone());
        db.set_fantasy_family(family);
    }
    if let Some(family) = family_for(FontRole::Serif) {
        db.set_serif_family(family);
    }
    if let Some(family) = family_for(FontRole::Monospace) {
        db.set_monospace_family(family);
    }
    std::sync::Arc::new(db)
}

#[cfg(test)]
mod font_ctx_tests {
    use super::{BULLET_FONT, FontRole, build_font_ctx};
    use parley::fontique::GenericFamily;

    #[test]
    fn roles_map_to_css_generic_families() {
        let mut ctx = build_font_ctx(&[(FontRole::Monospace, BULLET_FONT)]);
        let has =
            |ctx: &mut crate::FontContext, g| ctx.collection.generic_families(g).next().is_some();
        assert!(has(&mut ctx, GenericFamily::Monospace));
        assert!(has(&mut ctx, GenericFamily::UiMonospace));
        assert!(
            !has(&mut ctx, GenericFamily::Serif),
            "no serif font was supplied"
        );

        let mut ctx = build_font_ctx(&[(FontRole::SansSerif, BULLET_FONT)]);
        for g in [
            GenericFamily::SansSerif,
            GenericFamily::SystemUi,
            GenericFamily::Cursive,
        ] {
            assert!(has(&mut ctx, g), "{g:?} falls back to the sans-serif font");
        }
    }

    #[test]
    fn no_system_fonts_are_discovered() {
        let mut ctx = build_font_ctx(&[]);
        // Only the bundled bullet font is registered.
        assert_eq!(ctx.collection.family_names().count(), 1);
    }
}
