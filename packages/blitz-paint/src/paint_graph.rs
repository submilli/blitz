//! Frame-local recording separates CSS effect ownership from box visitation.
//! Geometry and overflow scopes come from the original paint traversal; shared
//! effects follow flat ancestry, so hoisted boxes can join their inline owner.
use crate::{inline_opacity::NodeScene, layers::LayerManager};
use anyrender::{
    Filter, Glyph, NormalizedCoord, Paint, PaintRef, PaintScene, RenderContext, Scene,
    recording::RenderCommand,
};
use blitz_dom::{BaseDocument, NodeId};
use kurbo::{Affine, Rect, Shape, Stroke, Vec2};
use peniko::{BlendMode, Color, Fill, FontData, StyleRef};
use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    rc::Rc,
    sync::Arc,
};
use style::properties::generated::longhands::position::computed_value::T as Position;

mod ownership;
use ownership::{ScopeIndex, WorkBudget, is_flex_or_grid_item};

// Metadata admission is cumulative, independent of the final renderer's layer
// admission. Refusing a whole recording avoids exposing content when a mask's
// closing commands would otherwise fall beyond the budget.
const RECORDING_WORK_LIMIT: usize = 2_000_000;
const RECORDING_SCOPE_LIMIT: usize = 1025;

struct Scope {
    command: RenderCommand,
    node: NodeId,
    effect: bool,
    postlude: bool,
    overflow: bool,
}
struct Drawing {
    node: NodeId,
    scopes: Vec<usize>,
    command: RenderCommand,
}
#[derive(Default)]
struct Recording {
    scopes: Vec<Scope>,
    active: Vec<usize>,
    drawings: Vec<Drawing>,
    work: usize,
    exhausted: bool,
}
#[derive(Clone)]
pub(crate) struct GraphScene {
    recording: Rc<RefCell<Recording>>,
    node: NodeId,
    effect_next: bool,
    postlude_next: bool,
    overflow_next: bool,
}
impl GraphScene {
    pub(crate) fn new(node: NodeId) -> Self {
        Self {
            recording: Rc::default(),
            node,
            effect_next: false,
            postlude_next: false,
            overflow_next: false,
        }
    }
    pub(crate) fn at_node(&self, node: NodeId) -> Self {
        Self {
            node,
            effect_next: false,
            postlude_next: false,
            overflow_next: false,
            recording: self.recording.clone(),
        }
    }
    /// Mark just the next pushed layer as an effect of this flat-tree node.
    pub(crate) fn effect(&mut self) {
        self.effect_next = true;
    }
    pub(crate) fn overflow(&mut self) {
        self.overflow_next = true;
    }
    pub(crate) fn postlude(&mut self) {
        self.postlude_next = true;
    }
    pub(crate) fn local(&mut self) {
        self.effect_next = false;
        self.overflow_next = false;
        self.postlude_next = false;
    }
    fn record(&mut self, scene: Scene) {
        let mut recording = self.recording.borrow_mut();
        for command in scene.commands {
            if recording.exhausted {
                return;
            }
            let cost = recording.active.len().saturating_add(1);
            recording.work = recording.work.saturating_add(cost);
            if recording.work > RECORDING_WORK_LIMIT {
                recording.exhausted = true;
                return;
            }
            match command {
                RenderCommand::PushLayer(_) | RenderCommand::PushClipLayer(_) => {
                    let id = recording.scopes.len();
                    if id >= RECORDING_SCOPE_LIMIT {
                        recording.exhausted = true;
                        return;
                    }
                    recording.scopes.push(Scope {
                        command,
                        node: self.node,
                        effect: self.effect_next,
                        postlude: self.postlude_next,
                        overflow: self.overflow_next,
                    });
                    self.effect_next = false;
                    self.postlude_next = false;
                    self.overflow_next = false;
                    recording.active.push(id);
                }
                RenderCommand::PopLayer => {
                    recording.active.pop();
                }
                _ => {
                    let scopes = recording.active.clone();
                    recording.drawings.push(Drawing {
                        node: self.node,
                        scopes,
                        command,
                    });
                }
            }
        }
    }
    pub(crate) fn paint(
        self,
        scene: &mut impl PaintScene,
        doc: &BaseDocument,
        layers: &LayerManager,
        clip: Rect,
    ) {
        let mut recording = std::mem::take(&mut *self.recording.borrow_mut());
        // Text can be recorded by an ancestor inline layout, and positioned
        // descendants can be hoisted. Suppress by flat ownership as well as
        // render traversal so neither can bypass protected control masking.
        let hidden = crate::inline_opacity::hidden_inline_descendants(doc);
        recording
            .drawings
            .retain(|drawing| !hidden.contains(&drawing.node));
        if recording.exhausted {
            return;
        }
        if let Some(graph) = Graph::new(doc, recording, clip) {
            graph.paint(scene, layers);
        }
    }
}
impl NodeScene for GraphScene {
    type Scene = Self;
    fn for_node(&mut self, id: NodeId) -> &mut Self {
        self.node = id;
        self
    }
}
impl RenderContext for GraphScene {}
impl PaintScene for GraphScene {
    fn reset(&mut self) {
        *self.recording.borrow_mut() = Recording::default();
    }
    fn push_layer(
        &mut self,
        blend: impl Into<BlendMode>,
        alpha: f32,
        transform: Affine,
        clip: &impl Shape,
        filter: Option<Arc<Filter>>,
        backdrop_filter: Option<Arc<Filter>>,
    ) {
        let mut scene = Scene::new();
        scene.push_layer(blend, alpha, transform, clip, filter, backdrop_filter);
        self.record(scene);
    }
    fn push_clip_layer(&mut self, transform: Affine, clip: &impl Shape) {
        let mut scene = Scene::new();
        scene.push_clip_layer(transform, clip);
        self.record(scene);
    }
    fn pop_layer(&mut self) {
        let mut scene = Scene::new();
        scene.pop_layer();
        self.record(scene);
    }
    fn stroke<'a>(
        &mut self,
        style: &Stroke,
        transform: Affine,
        brush: impl Into<PaintRef<'a>>,
        brush_transform: Option<Affine>,
        shape: &impl Shape,
    ) {
        let mut scene = Scene::new();
        scene.stroke(style, transform, brush, brush_transform, shape);
        self.record(scene);
    }
    fn fill<'a>(
        &mut self,
        style: Fill,
        transform: Affine,
        brush: impl Into<PaintRef<'a>>,
        brush_transform: Option<Affine>,
        shape: &impl Shape,
    ) {
        let mut scene = Scene::new();
        scene.fill(style, transform, brush, brush_transform, shape);
        self.record(scene);
    }
    fn draw_glyphs<'a, 's: 'a>(
        &'s mut self,
        font: &'a FontData,
        font_size: f32,
        hint: bool,
        normalized_coords: &'a [NormalizedCoord],
        embolden: Vec2,
        style: impl Into<StyleRef<'a>>,
        brush: impl Into<PaintRef<'a>>,
        brush_alpha: f32,
        transform: Affine,
        glyph_transform: Option<Affine>,
        glyphs: impl Iterator<Item = Glyph> + Clone,
    ) {
        let brush = match brush.into() {
            Paint::Solid(color) => Paint::Solid(color),
            Paint::Gradient(gradient) => Paint::Gradient(gradient.clone()),
            Paint::Image(image) => Paint::Image(image.to_owned()),
            Paint::Resource(resource) => Paint::Resource(resource),
            Paint::Custom(_) => Paint::Solid(Color::TRANSPARENT),
        };
        let command = RenderCommand::GlyphRun(anyrender::recording::GlyphRunCommand {
            font_data: font.clone(),
            font_size,
            hint,
            normalized_coords: normalized_coords.to_vec(),
            embolden,
            style: style.into().to_owned(),
            brush,
            brush_alpha,
            transform,
            glyph_transform,
            glyphs: glyphs.collect(),
        });
        self.record(Scene {
            tolerance: 0.1,
            commands: vec![command],
        });
    }
    fn draw_box_shadow(
        &mut self,
        transform: Affine,
        rect: Rect,
        brush: Color,
        radius: f64,
        std_dev: f64,
    ) {
        let mut scene = Scene::new();
        scene.draw_box_shadow(transform, rect, brush, radius, std_dev);
        self.record(scene);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Key {
    Effect(usize),
    Inline(NodeId),
    Context(NodeId),
    Local(usize),
}
struct Group {
    command: Option<RenderCommand>,
    children: Vec<Entry>,
    key: Option<Key>,
    node: NodeId,
    postlude: bool,
}
enum Entry {
    Group(usize),
    Draw(Box<RenderCommand>),
}
struct Graph {
    groups: Vec<Group>,
}
impl Graph {
    fn new(doc: &BaseDocument, recording: Recording, clip: Rect) -> Option<Self> {
        let mut graph = Self {
            groups: vec![Group {
                command: None,
                children: Vec::new(),
                key: None,
                node: doc.root_node().id,
                postlude: false,
            }],
        };
        let index = ScopeIndex::new(doc, &recording.scopes, clip);
        let mut lookup = HashMap::new();
        let mut budget = WorkBudget(recording.work);
        for mut drawing in recording.drawings {
            let path = index.path(&mut drawing, &mut budget)?;
            graph.append_path(&index, &mut lookup, path, drawing.command, &mut budget)?;
        }
        graph.sort_inline_contexts(doc);
        graph.sort_postludes();
        Some(graph)
    }

    fn append_path(
        &mut self,
        index: &ScopeIndex<'_>,
        lookup: &mut HashMap<(usize, Key), usize>,
        path: Vec<Key>,
        command: RenderCommand,
        budget: &mut WorkBudget,
    ) -> Option<()> {
        let mut parent = 0;
        for key in path {
            budget.spend(1)?;
            let child = *lookup.entry((parent, key)).or_insert_with(|| {
                let child = self.groups.len();
                self.groups.push(index.group(key));
                self.groups[parent].children.push(Entry::Group(child));
                child
            });
            parent = child;
        }
        self.groups[parent]
            .children
            .push(Entry::Draw(Box::new(command)));
        Some(())
    }

    fn sort_postludes(&mut self) {
        // Mask composition closes the whole effect, including hoisted boxes
        // discovered later in the original paint traversal.
        for id in 0..self.groups.len() {
            let mut children = std::mem::take(&mut self.groups[id].children);
            children.sort_by_key(
                |entry| matches!(entry, Entry::Group(child) if self.groups[*child].postlude),
            );
            self.groups[id].children = children;
        }
    }

    fn sort_inline_contexts(&mut self, doc: &BaseDocument) {
        let mut preorder = HashMap::new();
        let mut work = vec![doc.root_node().id];
        while let Some(id) = work.pop() {
            preorder.insert(id, preorder.len());
            let node = &doc.as_ref().tree()[id];
            work.extend(node.after());
            work.extend(node.flat_children().iter().rev().copied());
            work.extend(node.before());
        }
        for parent in 0..self.groups.len() {
            let has_inline = self.groups[parent]
                .children
                .iter()
                .any(|entry| match entry {
                    Entry::Group(id) => match self.groups[*id].key {
                        Some(Key::Inline(_)) => true,
                        Some(Key::Context(node)) => doc.as_ref().tree()[node]
                            .primary_styles()
                            .is_some_and(|style| {
                                style.clone_display().is_inline_flow()
                                    && style.get_effects().opacity < 1.0
                            }),
                        _ => false,
                    },
                    _ => false,
                });
            if !has_inline && !matches!(self.groups[parent].key, Some(Key::Inline(_))) {
                continue;
            }
            let mut children = std::mem::take(&mut self.groups[parent].children);
            children.sort_by_key(|entry| match entry {
                Entry::Group(id)
                    if matches!(self.groups[*id].key, Some(Key::Inline(_) | Key::Context(_))) =>
                {
                    let node = &doc.as_ref().tree()[self.groups[*id].node];
                    let z = if is_flex_or_grid_item(doc, node.id)
                        || node
                            .primary_styles()
                            .is_some_and(|s| s.clone_position() != Position::Static)
                    {
                        node.z_index()
                    } else {
                        0
                    };
                    (
                        if z < 0 { 0 } else { 2 },
                        z,
                        preorder.get(&node.id).copied().unwrap_or(usize::MAX),
                    )
                }
                _ => (1, 0, 0),
            });
            self.groups[parent].children = children;
        }
    }
    fn paint(mut self, scene: &mut impl PaintScene, layers: &LayerManager) {
        // A refused mask footer must never leave already drawn source exposed.
        // Admit the complete graph before emitting any command to the backend.
        if !layers.can_admit(
            self.groups
                .iter()
                .filter(|group| group.command.is_some())
                .count(),
        ) {
            return;
        }
        enum Work {
            Enter(usize),
            Draw(Box<RenderCommand>),
            Exit(bool),
        }
        let mut work = vec![Work::Enter(0)];
        while let Some(action) = work.pop() {
            match action {
                Work::Draw(command) => scene.append_scene(
                    Scene {
                        tolerance: 0.1,
                        commands: vec![*command],
                    },
                    Affine::IDENTITY,
                ),
                Work::Exit(pushed) => layers.maybe_pop_layer(scene, pushed),
                Work::Enter(id) => {
                    let group = &mut self.groups[id];
                    let pushed = match group.command.take() {
                        Some(RenderCommand::PushLayer(layer)) => {
                            if layer.alpha == 0.0 {
                                continue;
                            }
                            let pushed = layers.maybe_push_blend_layer(
                                scene,
                                true,
                                layer.blend,
                                layer.alpha,
                                layer.transform,
                                &layer.clip,
                                layer.filter,
                                layer.backdrop_filter,
                            );
                            if !pushed {
                                continue;
                            }
                            pushed
                        }
                        Some(RenderCommand::PushClipLayer(layer)) => {
                            let pushed = layers.maybe_push_layer(
                                scene,
                                true,
                                1.0,
                                layer.transform,
                                &layer.clip,
                                None,
                                None,
                            );
                            if !pushed {
                                continue;
                            }
                            pushed
                        }
                        _ => false,
                    };
                    work.push(Work::Exit(pushed));
                    work.extend(std::mem::take(&mut group.children).into_iter().rev().map(
                        |entry| match entry {
                            Entry::Group(id) => Work::Enter(id),
                            Entry::Draw(command) => Work::Draw(command),
                        },
                    ));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use blitz_dom::DocumentConfig;

    fn masked_drawing(scene: &mut GraphScene) {
        let clip = Rect::new(0.0, 0.0, 10.0, 10.0);
        scene.effect();
        scene.push_layer(
            peniko::Mix::Normal,
            0.999,
            Affine::IDENTITY,
            &clip,
            None,
            None,
        );
        scene.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            Color::from_rgb8(255, 0, 0),
            None,
            &clip,
        );
        // The budget may be reached before this transparent mask is recorded.
        // No prefix of the drawing may then escape to the backend.
        scene.postlude();
        scene.push_layer(
            peniko::BlendMode::new(peniko::Mix::Normal, peniko::Compose::DestIn),
            1.0,
            Affine::IDENTITY,
            &clip,
            None,
            None,
        );
        scene.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            Color::TRANSPARENT,
            None,
            &clip,
        );
        scene.pop_layer();
        scene.pop_layer();
    }

    #[test]
    fn exhausted_recording_discards_masked_content_before_replay() {
        let doc = BaseDocument::new(DocumentConfig::default());
        let mut scene = GraphScene::new(doc.root_node().id);
        scene.recording.borrow_mut().work = RECORDING_WORK_LIMIT - 4;
        masked_drawing(&mut scene);
        assert!(scene.recording.borrow().exhausted);
        let mut output = Scene::new();
        scene.paint(
            &mut output,
            &doc,
            &LayerManager::default(),
            Rect::new(0.0, 0.0, 10.0, 10.0),
        );
        assert!(output.commands.is_empty());
    }

    #[test]
    fn scope_admission_discards_the_whole_masked_frame() {
        let doc = BaseDocument::new(DocumentConfig::default());
        let mut scene = GraphScene::new(doc.root_node().id);
        let clip = Rect::new(0.0, 0.0, 10.0, 10.0);
        for _ in 0..RECORDING_SCOPE_LIMIT {
            scene.push_clip_layer(Affine::IDENTITY, &clip);
            scene.pop_layer();
        }
        masked_drawing(&mut scene);
        assert!(scene.recording.borrow().exhausted);
        let mut output = Scene::new();
        scene.paint(&mut output, &doc, &LayerManager::default(), clip);
        assert!(output.commands.is_empty());
    }
    #[test]
    fn replay_layer_admission_cannot_expose_a_mask_prefix() {
        let doc = BaseDocument::new(DocumentConfig::default());
        let node = doc.root_node().id;
        let clip = Rect::new(0.0, 0.0, 10.0, 10.0);
        let mut source = Scene::new();
        source.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            Color::from_rgb8(255, 0, 0),
            None,
            &clip,
        );
        let mut graph = Graph {
            groups: vec![Group {
                command: None,
                key: None,
                children: vec![Entry::Draw(Box::new(source.commands.remove(0)))],
                node,
                postlude: false,
            }],
        };
        for _ in 0..=RECORDING_SCOPE_LIMIT {
            let mut layer = Scene::new();
            layer.push_clip_layer(Affine::IDENTITY, &clip);
            let id = graph.groups.len();
            graph.groups.push(Group {
                command: layer.commands.pop(),
                key: None,
                children: Vec::new(),
                node,
                postlude: false,
            });
            graph.groups[0].children.push(Entry::Group(id));
        }
        let mut output = Scene::new();
        graph.paint(&mut output, &LayerManager::default());
        assert!(output.commands.is_empty());
    }
}
