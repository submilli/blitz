# Blitz SVG parser dependency

This is the usvg 0.48.1 registry source, retaining its upstream licenses and
copyright notices. `UPSTREAM.json` records the registry checksum, upstream
commit and SHA-256 of every original copied file. The snapshot is held in the
Blitz engine fork so its revision and downstream `usvg` patch share the Blitz
pin; no separate fork or ambient parser implementation is loaded.

SUB-1588 retains the original object bounding box on each parsed filter when
primitiveUnits is objectBoundingBox. usvg resolves most lengths but leaves
lighting positions in primitive coordinates and uses its own displacement
scale convention. Renderers need that context even after usvg clones a filter
and gives it a generated ID. The public read-only getter changes neither XML
nor CSS selection, conversion order, IDs, inputs or primitive results.

Local changes are in `src/tree/filter.rs` (field/getter and regressions) and
`src/parser/filter.rs` (initialization for URL and CSS-function filters).
