# Regress bounded fork

Based on regress 0.12.0 from the Cargo registry. The cached `.crate` archive
has SHA-256 `32eef8b209c3c1c15dbad02c1f30f9539f00dc7253e0cbcdaae442a50a09d7c1`.
Every copied source file was compared byte-for-byte with that archive before
modification. Upstream MIT and Apache-2.0 licenses are retained.

This package is private to the Blitz fork. Production uses only `Pattern` and
its caller-owned `Budget`; the upstream API is explicitly under `legacy` for
compatibility/differential testing. It must not be used for page patterns.

The bounded path disables optimization/search-prefix analysis, admits source and
emission storage, meters parser/class-set work and backtracking, bounds nesting,
and returns typed resource exhaustion separately from syntax or mismatch.
Admission is cumulative across nested work and repeated control/token matches.
Default operation limits are 4,000,000 work units and 32 MiB of admitted storage;
source and value caps are 4,096 and 1,048,576 UTF-16 code units. These allowances
include transient allocations and do not refund freed storage.

Local semantic fixes cover Unicode-v class-set grammar, string membership/order,
case closure before set algebra, lookbehind strings, duplicate named references,
and platform-independent numeric bounds. UTF-16 input preserves lone surrogates.
The fork retains upstream licenses, Unicode tables, tests and a visibly separated
legacy API. Sources were formatted after modification; consult the Git diff and
SUB-1095 evidence for the reviewed changes and executed checks.
