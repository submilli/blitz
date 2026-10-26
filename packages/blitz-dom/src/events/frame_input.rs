//! Native subdocument forwarding follows the projected content viewport.
use crate::BaseDocument;
use blitz_traits::events::{DomEvent, DomEventData, UiEvent};
use kurbo::Point;

pub(super) enum FrameInput {
    Unpositioned,
    Content(Point),
    Outside,
}

impl FrameInput {
    pub(super) fn locate(doc: &BaseDocument, event: &DomEvent) -> Self {
        let Some(child) = doc.get_node(event.target).and_then(|node| node.subdoc()) else {
            return Self::Unpositioned;
        };
        let pointer = match &event.data {
            DomEventData::PointerMove(pointer)
            | DomEventData::PointerDown(pointer)
            | DomEventData::PointerUp(pointer)
            | DomEventData::PointerCancel(pointer)
            | DomEventData::PointerEnter(pointer)
            | DomEventData::PointerLeave(pointer)
            | DomEventData::PointerOver(pointer)
            | DomEventData::PointerOut(pointer)
            | DomEventData::MouseMove(pointer)
            | DomEventData::MouseDown(pointer)
            | DomEventData::MouseUp(pointer)
            | DomEventData::MouseEnter(pointer)
            | DomEventData::MouseLeave(pointer)
            | DomEventData::MouseOver(pointer)
            | DomEventData::MouseOut(pointer)
            | DomEventData::TouchStart(pointer)
            | DomEventData::TouchMove(pointer)
            | DomEventData::TouchEnd(pointer)
            | DomEventData::TouchCancel(pointer)
            | DomEventData::Click(pointer)
            | DomEventData::ContextMenu(pointer)
            | DomEventData::DoubleClick(pointer) => pointer,
            _ => return Self::Unpositioned,
        };
        let Some(projection) = doc
            .content_box_to_viewport(event.target)
            .filter(|projection| {
                projection.determinant().is_finite()
                    && projection.determinant().abs() >= f64::EPSILON
            })
        else {
            return Self::Outside;
        };
        let point = projection.inverse()
            * Point::new(
                f64::from(pointer.coords.client_x),
                f64::from(pointer.coords.client_y),
            );
        let child = child.inner();
        let viewport = child.viewport();
        let width = f64::from(viewport.window_size.0) / viewport.scale_f64();
        let height = f64::from(viewport.window_size.1) / viewport.scale_f64();
        if point.x >= 0.0 && point.y >= 0.0 && point.x < width && point.y < height {
            Self::Content(point)
        } else {
            Self::Outside
        }
    }

    pub(super) fn project(&self, event: &mut UiEvent, scroll: crate::util::Point<f64>) {
        let Self::Content(point) = self else { return };
        let pointer = match event {
            UiEvent::PointerMove(pointer)
            | UiEvent::PointerDown(pointer)
            | UiEvent::PointerUp(pointer)
            | UiEvent::PointerCancel(pointer) => pointer,
            _ => return,
        };
        pointer.coords.client_x = point.x as f32;
        pointer.coords.client_y = point.y as f32;
        pointer.coords.page_x = (point.x + scroll.x) as f32;
        pointer.coords.page_y = (point.y + scroll.y) as f32;
    }
}
