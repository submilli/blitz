# Blitz CPU paint adapter

This is the anyrender_vello_cpu 0.17.0 registry source, preserving the
upstream licenses. `UPSTREAM.json` records the checksum, exact upstream
revision and original copied file hashes. The licenses were fetched from
that upstream revision because the registry package omits them.

SUB-1590 changes only `src/scene.rs`: a source rectangle passed through
`kurbo::Shape::as_rect` selects Vello's rectangle stroke primitive. General
paths continue through `stroke_path`. This retains source identity across
inline SVG, SVG images, Canvas sources and temporary SVG filter scenes,
without converting their geometry or duplicating the adapter's image cache,
paint conversion, glyph rendering or layer behavior.
