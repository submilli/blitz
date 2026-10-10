#![allow(clippy::module_inception)]

mod attributes;
mod children;
pub use children::Children;
#[cfg(feature = "custom-widget")]
mod custom_widget;
mod edit;
mod element;
mod node;
pub(crate) mod scrollbar;
mod stylo_data;
#[cfg(feature = "svg")]
mod svg;
mod text;

pub use attributes::{Attribute, Attributes};
#[cfg(feature = "custom-widget")]
pub use custom_widget::{
    ComputedStyles, CustomWidgetData, CustomWidgetStatus, ProxyRenderContext, Widget,
};
pub use element::{
    CanvasData, DocumentData, ElementData, ImageData, ImageResourceData, LayoutData,
    ListItemLayout, ListItemLayoutPosition, Marker, RasterImageData, SpecialElementData,
    SpecialElementType, Status,
};
pub(crate) use element::{FormControlState, ScriptState};
pub use node::*;
pub use scrollbar::{ScrollbarColor, ScrollbarRef, ScrollbarWidth};
pub use stylo_data::ComputedStyleRef;
#[cfg(all(test, feature = "svg"))]
pub(crate) use svg::MAX_SVG_SOURCE_BYTES;
#[cfg(feature = "svg")]
pub use svg::{SvgImageData, SvgIntrinsicDimensions};
#[cfg(feature = "svg")]
pub(crate) use svg::{inflate_svgz, is_svgz};
pub use text::{GeneratedTextInputEvent, TextBrush, TextInputData, TextLayout};
