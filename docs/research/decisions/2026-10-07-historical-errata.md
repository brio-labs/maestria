# Historical retrieval-research errata — 2026-10-07

**Disposition:** documentary correction only. No experiment was run or replayed, no model or route was promoted, and no publication or GitHub change was made.

## Provenance and affected text

This erratum corrects two interpretations introduced by [PR #428](https://github.com/brio-labs/maestria/pull/428), not its historical tables or results. The affected current text is [`docs/RESEARCH.md` §2.1.0a](../../RESEARCH.md), [`§2.1.0b`](../../RESEARCH.md), and the sample-size clarification in [`§2.1.1`](../../RESEARCH.md). The historical dense-lane disposition is retained in [`§2.1.0c`](../../RESEARCH.md). The original PR diff remains the provenance for the earlier wording; the current source sections and correction records are linked in `claims.jsonl`.

## BGE-M3 sparse-head shape

The earlier text treated a `[1, 1024]` `sparse_linear.pt` tensor as a stub because it expected a `[250002, 1024]` projection and inferred that trained sparse weights were absent. That inference is contradicted by the official FlagEmbedding mechanism: at [pinned source revision `6eefbac0e0c185205fe210b999a4cbe55c97054e`](https://github.com/FlagOpen/FlagEmbedding/blob/6eefbac0e0c185205fe210b999a4cbe55c97054e/research/BGE_M3/modeling.py#L78-L123), the implementation defines `Linear(hidden_size, 1)`, applies ReLU to one scalar per token, and scatters/max-reduces token weights by `input_ids` into vocabulary IDs. With hidden size 1024, `[1, 1024]` is the expected linear weight shape; the vocabulary dimension arises in the subsequent token-ID aggregation. Shape and file size alone do not establish that trained weights are absent or random.

The same source constructs a new linear head and loads saved pooler weights only when both `colbert_linear.pt` and `sparse_linear.pt` exist. This describes loader behavior; it does not establish which weights were present in the historical checkpoint. No weights were downloaded. The original report's 391 backbone keys, 3.5 KB file size, and `[1, 1024]` shape are retained as reported historical observations, not rechecked facts. The previously reported community ONNX omissions/collapsed sparse output and CPU costs remain as reported in §2.1.0a; this correction did not reproduce or retest them.

## Seeded 5,000-passage comparison

The 200-query, 5,000-passage, seed-42 comparison retained all judged relevant passages and added random fillers. Its table is an exploratory comparison conditional on that qrels-inclusive subcorpus, sampled negatives, candidate models, and encoding conventions. Removing or changing hard negatives may change or invert model order. The results do not establish full-corpus ranking, current-launcher quality, or state-of-the-art performance. The original table and report are preserved unchanged; neither was regenerated, reranked, or replayed for this erratum. The reported sample inflation and full-corpus BM25 reference are historical context, not new measurements or a like-for-like comparison.

The 31 repetitions per case/route in the #428 record are repeated machine timings, not 31 independent retrieval needs. There were only two independent final task cases per class; that is a narrow basis for generalizing quality, and the 147-chunk timing scope does not establish performance for all user files.

## Historical disposition and current route

- **#425 → #426 → #428:** #425 was closed without merge. #426 and then #428 were merged; the terminal sparse result remained negative and no sparse class was promoted. The optimization and the historical timing/quality observations remain attached to their original scope.
- **#429 → #445:** both are historical dense results under their dated, changed budgets and evaluation scopes. Their former per-class promotions do not qualify or promote the current route.
- **#511:** merged evidence for a negative late-interaction result only. The full late-interaction engine was kept in a separate archived, unmerged implementation and was not integrated by #511. This reference is not evidence that archived bytes are currently available.
- **#515:** remains an open, unmerged candidate, not an integrated telemetry capability.

The current route remains **Shadow**; mandatory lexical quality is retained. Historical promotion records do not override the current route identity or the unpassed current serving qualification.

## Bounded unresolved reference and evidence limits

The full-SHA mismatch associated with #551 remains unresolved and is not presumed to be a typo. The private original was not inspected, hashed, downloaded, or replayed. This erratum makes no inference that the referenced evidence is available or absent; its identifier is only a bounded pointer. No private query values, paths, capture bytes, or capture hashes are copied into either ledger or this note.

The claim ledger uses explicit statuses `proposed`, `reported_not_reproduced`, `exploratory_supported`, `contradicted`, `inconclusive`, and `superseded`; each record states assertion, scope, protocol, evidence, limits, and reviewer status. The literature ledger records primary source identity/version/date and limits on transfer; model-card marketing or a source's own broad benchmark claims are not used as defaults or launcher qualification. Parent completed the primary-reference and documentary/linkage smoke for the cited sources, JSONL records, and local references; independent review found no introduced documentary defect. This was bounded to documentary/source scope: historical experiments and measurements were not replayed, archived-engine availability was not verified, the private #551 original was not inspected, and no model or serving qualification was performed. No historical table, report, or frozen-trial hash was overwritten.