# Vendored Stylo

This directory starts from the published `stylo` 0.21.0 crate (crates.io
checksum `631d138475c566e37168f8394380e8ec89a6089dc806c7a76d355d73e9006553`).
It is kept in the Blitz fork because custom-property substitution and its
dependency traversal must reject page-created recursion before it can overflow
the browser guest's fixed 8 MiB stack.

Apart from this notice and the included MPL 2.0 license text, the fork changes
only `custom_properties.rs` and `properties/cascade.rs` from that release.
