# xml5ever 0.39.0

Imported from the crates.io xml5ever 0.39.0 package with its MIT/Apache licenses.
All source is unchanged except `src/tree_builder/mod.rs`: report an error when
EOF arrives in Main phase with an unclosed root. Blitz's bounded XML adapter
projects reported errors as inert parsererror documents. Regression coverage:
`blitz-html/tests/document_metadata.rs::unclosed_xml_reports_an_error_document`.
The dependency remains inside the pinned Blitz fork; no browser-side XML parser
or token-balancing workaround is introduced.
