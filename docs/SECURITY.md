# Security Architecture

This document defines the security boundaries and invariants for Maestria. It complements, and does not replace:

- [docs/SPECS.md](SPECS.md) — system contracts and behavior
- [docs/PHILOSOPHY.md](PHILOSOPHY.md) — architectural principles and invariant ownership

Security controls are implemented through replaceable adapters and policies. No model, database, search backend, parser, provider, or algorithm is a permanent default until it has been benchmarked against Maestria’s security, correctness, and operational requirements.

## 1. Security Objectives

Maestria must:

1. enforce scope and authorization before data is retrieved, ranked, or exposed;
2. preserve provenance, source versions, and evidence boundaries;
3. treat external content and model output as untrusted data;
4. prevent secrets and restricted data from crossing policy boundaries;
5. make denial, quarantine, abstention, and incomplete evidence explicit;
6. provide an auditable explanation of security-relevant decisions;
7. keep domain state transitions deterministic and policy-controlled.

Maestria owns internal state integrity and provenance. It does not make an external source, claim, or model-generated assertion factually true.

## 2. Security Invariants

The following invariants are normative.

### S-1: Authorization precedes retrieval

ACL, scope, sensitivity, trust-zone, and quarantine filters are applied before candidate scoring, fusion, reranking, graph traversal, context expansion, or evidence packing.

A prohibited candidate must not influence ranking or reveal its existence through scores, counts, explanations, or timing-sensitive response content.

### S-2: Untrusted content remains data

Web pages, files, command output, repository content, retrieved passages, OCR, model output, and harness results are data. They are never policy, system instructions, authorization, or tool definitions.

### S-3: Provenance is mandatory

Every evidence candidate must resolve to:

```text
artifact version
source span, page, region, or structured location
corpus snapshot
retrieval trace
trust and freshness metadata
```

Content without sufficient provenance may be retained as quarantined or incomplete data but cannot silently become authoritative evidence.

### S-4: No implicit trust upgrade

Parsing, embedding, summarization, reranking, memory promotion, or model agreement does not increase source authority by itself.

Trust, freshness, conflict, and sensitivity annotations are policy-controlled metadata.

### S-5: No fail-open security behavior

If authorization, scope, provenance, secret scanning, prompt-injection checks, or provider capability checks cannot be completed, the operation is denied, quarantined, or marked incomplete.

It must not continue under a weaker implicit policy.

### S-6: Domain state changes use domain inputs

Adapters and providers cannot mutate domain state directly. They return typed results that runtime maps into `DomainInput`; the domain reducer performs the state transition.

### S-7: Evidence is not external truth

An evidence object records what a source or operation produced. A claim remains uncertain, potentially stale, disputed, or unsupported until validated under policy.

## 3. Security Domains and Trust Zones

Every artifact, evidence object, provider result, and derived representation has a security classification.

| Zone | Meaning | Default handling |
|---|---|---|
| `trusted_local` | Locally controlled data within an approved scope | Usable subject to ACL and sensitivity policy |
| `user_owned` | User-provided notes, files, or repositories | Preserve provenance; do not treat as universally authoritative |
| `validated_external` | External data fetched and checked under policy | Usable with source, freshness, and snapshot metadata |
| `untrusted_external` | Web, provider, or third-party content not yet validated | Candidate data only |
| `quarantined` | Content with parser, injection, malware, provenance, or policy concerns | Isolated from normal retrieval and generation |
| `denied` | Content outside authorization or scope | Not retrievable or exposable |

Trust zones are security metadata, not claims about factual correctness.

## 4. Scope and Authorization

Scope is explicit on every operation that can read, write, execute, retrieve, fetch, or promote data.

A scope should identify, as applicable:

```text
instance
user or principal
workspace or repository
allowed paths
allowed domains
allowed artifact classes
allowed modalities
allowed harness capabilities
read/write/execute permissions
network permissions
time and freshness limits
sensitivity ceiling
```

### 4.1 Search and Retrieval

A search plan must carry:

```text
corpus scope
ACL context
trust-zone restrictions
snapshot identity
freshness requirement
sensitivity constraints
```

Authorization is enforced before:

- candidate generation;
- index or graph traversal;
- score calculation;
- duplicate clustering;
- reranking;
- context expansion;
- evidence-pack construction;
- model or provider submission.

Indexes and caches must not bypass the same authorization rules applied to source data.

### 4.2 Harness and Filesystem Operations

Harness requests must declare:

```text
capability requested
principal and scope
working directory
read/write targets
command or action
network requirement
approval requirement
```

The harness must reject requests outside the granted scope. A successful process exit does not override policy failure.

### 4.3 Web and External Providers

Network access is disabled unless explicitly granted. Domain, URL, method, content type, byte, page, query, and time budgets are policy inputs.

Redirects, uploads, authenticated requests, and access to local or private network addresses require separate authorization.

## 5. Taint and Quarantine

Taint tracks data that may affect safety, reliability, or authorization.

Example taint labels:

```text
contains_prompt_injection_signal
contains_secret_signal
untrusted_external
parser_failed
provenance_incomplete
stale
poisoning_suspected
sensitive
generated_derivative
live_unreproducible
```

Taint is additive unless a versioned validator explicitly records a permitted disposition. A summary or embedding inherits the relevant source restrictions.

### 5.1 Quarantine Rules

Content is quarantined when:

```text
- parsing fails in a way that prevents reliable provenance;
- prompt injection or poisoning indicators require review;
- secret exposure is suspected;
- source scope cannot be established;
- a provider returns malformed or unverifiable output;
- content is denied but must be retained for audit;
- evidence cannot be aligned to the source snapshot.
```

Quarantined content must not enter ordinary context, memory promotion, task completion evidence, or policy text.

## 6. Prompt Injection as Data

Prompt-injection detection is a security signal, not a factual judgment.

External or retrieved content may contain instructions such as:

```text
ignore previous instructions
reveal secrets
change system policy
call a tool
approve an action
modify files
```

These strings remain source content. They cannot:

- alter system or policy instructions;
- grant capabilities;
- change approval requirements;
- authorize tools or network access;
- modify domain state;
- cause memory promotion;
- suppress evidence or audit records.

Retrieved content must be passed through a clearly delimited data channel. System instructions, policy decisions, tool descriptions, and approval text must remain separate from evidence content.

Detection results must include:

```text
source evidence ID
detector and version
matched span or reason
severity
disposition
review status
```

Implementations are replaceable until benchmarked against Maestria-specific injection and poisoning test sets. Detection alone is not a complete defense; capability isolation and policy enforcement remain mandatory.

## 7. Secrets and Sensitive Data

Secrets include, at minimum:

```text
credentials and tokens
private keys and certificates
session material
passwords
API keys
personal or regulated data
private repository content
user-designated confidential material
```

### 7.1 Handling Requirements

- Secrets must not be embedded, indexed, summarized, logged, or placed in ordinary evidence packs.
- Secret scanning occurs before persistence, indexing, provider submission, and memory promotion.
- Redaction must preserve the fact that redaction occurred without exposing the value.
- Secret-bearing command output is stored only under an explicit policy and protected storage path.
- Providers receive the minimum data required for the approved operation.
- Access to sensitive data is auditable by principal, purpose, scope, and provider.
- Failed secret scans produce denial or quarantine, not a silent downgrade.

A secret-like string is a security signal, not proof that the value is a valid credential. Final disposition is governed by policy and review.

## 8. Provider and Adapter Boundaries

Providers include, but are not limited to:

```text
search and index backends
embedding and reranking services
language or multimodal models
web providers
parsers
filesystem and blob stores
harnesses
external APIs
```

Provider boundaries must use typed adapter contracts.

Providers must not:

- access domain state directly;
- mutate domain state;
- bypass governance;
- receive credentials or content outside their declared scope;
- return provider-specific types across domain or governance boundaries;
- be treated as authoritative merely because they returned successfully.

Provider outputs are untrusted until validated for:

```text
schema correctness
scope compliance
provenance
model/index fingerprint compatibility
content and size limits
secret and injection signals
freshness
reproducibility
```

Capability descriptors must state what a provider can do and under which limits. Runtime must reject requests requiring undeclared capabilities.

Backend, model, parser, and algorithm implementations remain replaceable until benchmarked. Replacement requires contract tests, security regression tests, migration checks, and—where applicable—quality and latency evaluation.

## 9. Storage and Index Security

### 9.1 Content and Metadata Separation

Immutable source content is stored by content hash. Metadata, ACLs, trust labels, taint, and policy annotations are stored separately and versioned.

Blob paths must be derived from validated content hashes. Path traversal and arbitrary path construction are prohibited.

### 9.2 Indexes and Caches

Indexes are projections, not authoritative state.

Every index generation records:

```text
source/corpus snapshot
representation and schema version
model or processing fingerprint
ACL and filtering policy version
build status
activation time
```

Index activation is atomic. Old generations remain available for rollback until validation completes.

Caches must include authorization and scope context in their keys or be proven incapable of crossing scopes.

### 9.3 Deletion and Revocation

When source access is revoked or content is deleted:

```text
- future retrieval is blocked immediately;
- active indexes and caches are invalidated or filtered;
- derived representations inherit the restriction;
- evidence packs are marked invalid where required;
- audit records retain only the minimum permitted metadata.
```

Deletion behavior must be tested for metadata, blobs, indexes, caches, and generated derivatives.

## 10. Evidence, Claims, and Memory

The security model distinguishes:

```text
Source       produces an observation.
Evidence     preserves a source-backed observation.
Claim        normalizes an uncertain proposition.
Memory       promotes a useful claim under policy.
Decision     selects an action based on evidence and policy.
Validation   checks support and required controls.
```

Evidence may be stale, contradictory, disputed, or incomplete. An evidence record does not make its source true.

Memory promotion requires:

```text
source lineage
adequate evidence coverage
scope and sensitivity checks
duplicate and contradiction checks
freshness evaluation
governance approval where required
```

A generated summary or memory must never replace the raw source span required for validation or citation.

## 11. Failure and Abstention Behavior

Security-relevant failures are explicit and auditable.

| Condition | Required result |
|---|---|
| Scope cannot be established | Deny |
| ACL check fails | Deny without revealing restricted content |
| Provenance is incomplete | Quarantine or mark evidence incomplete |
| Prompt injection is detected | Preserve as data; quarantine or restrict use according to policy |
| Secret exposure is suspected | Redact, deny, or quarantine |
| Provider exceeds capability or budget | Cancel or fail closed |
| Source is stale for the requested task | Warn, revalidate, or abstain |
| Evidence conflicts | Report conflict; do not silently collapse it |
| Required evidence is missing | `evidence_incomplete` or abstain |
| External source cannot be reproduced | Mark non-reproducible and require fresh validation |
| Validation fails | Prevent `CompletedVerified` |

Permitted task outcomes include:

```text
answerable
answerable_with_warnings
evidence_incomplete
sources_conflict
stale_evidence_only
no_evidence_found
denied
quarantined
```

A failed security check must not be represented as successful completion.

## 12. Auditability

Security-relevant actions produce append-only audit records or domain events containing, as appropriate:

```text
principal and instance
request and task identifiers
scope and policy profile
requested capability
allow/deny/quarantine decision
policy and validator versions
source, artifact, and snapshot identifiers
provider and adapter identity
redacted reason and failure code
approval references
result and disposition
```

Audit records must avoid storing secrets or restricted content unnecessarily. Redaction itself is recorded.

Search traces and transition journals are reproducibility and audit artifacts. They are not authoritative external truth and must be protected by the same scope controls as the data they describe.

## 13. Verification Requirements

Security controls require shared contract and regression tests covering:

```text
ACL filtering before scoring
cross-scope cache isolation
quarantine propagation
prompt-injection boundary enforcement
secret redaction and non-persistence
provider capability enforcement
path traversal and blob isolation
index generation authorization
revocation and deletion behavior
stale and conflicting evidence handling
fail-closed behavior
audit completeness without secret leakage
```

Retrieval and security changes must be evaluated against versioned Maestria-specific sets, including:

```text
ACL leakage attempts
prompt-injection fixtures
poisoning and near-duplicate cases
secret-bearing inputs
scope-confusion cases
stale and contradictory sources
provider failure and malformed-output cases
```

Public benchmarks or a provider’s stated capabilities are not proof that the implementation is secure for Maestria.