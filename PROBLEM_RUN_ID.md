# Run Identity, Public Starts, and Overlap Coordination

This note captures the original run identity problem, why the old behavior was surprising for public
operations, which invariants must not be weakened, the options that were considered, and the
implemented resolution.

The authoritative design contract remains `docs/design.md`.

## The Immediate Symptom

This command:

```sh
mfm run start --op portfolio_snapshot --config portfolio.toml
```

currently derives the same run identity every time the certified spec and store trust scope are the
same. If the first run completed, a later command attaches to that completed run instead of creating
a fresh portfolio snapshot.

That is surprising for a public operation named `portfolio_snapshot`. A user expects "start" to
perform a new read of current external state, not to return an old completed snapshot for the same
config.

The same problem appears in future workflows:

- `portfolio_rebalance` may run every hour from cron. Each tick should be a separate run even though
  the config is the same.
- `portfolio_snapshot_warn` may read portfolio state and send a Telegram warning if a threshold is
  crossed. It is repeatable by schedule or manual start, even though it has a notification side
  effect.
- A user may manually run the same public op twice and expect two observations, not a permanent
  singleton.

## Original Identity Model

Before the fix, normal launch identity was content-addressed from:

```text
certified_spec_hash
trust_scope_id
invocation_key_digest, but only when a caller supplied one
```

The derived `run_id` is recorded in `RunAdmitted` and then used by attach, resume, replay, status,
and public-output authority checks.

This model is strong for durable authority:

- one run stream has one `RunAdmitted`;
- `run_id` is derived from recorded identity material;
- replay and public-output rendering can fail closed if identity material does not match;
- duplicate launchers for the same identity attach instead of creating ambiguous streams.

The problem is not the existence of `run_id` validation. The problem is that the default identity
for public starts is too close to "canonical run for this config" and too far from "this concrete
execution occurrence".

## Process-Fungible Execution Constraint

`docs/design.md` says the semantic unit is the certified run, not the process that happens to drive
it. It also says leases, waiters, execution claims, notifications, and observation cursors are
operational liveness mechanisms.

That means these things can coordinate work, reduce duplicate effort, or wake blocked invokers. They
must not decide semantic authority for:

- append events;
- replay;
- resume;
- public-output rendering;
- side-effect legality;
- terminal run state.

This matters because it rules out a tempting shortcut: "if the execution lane is free, reuse the
same run id for a new run." That would make mutable liveness state part of semantic identity.

Resource lanes have a similar shape. A FIFO waiter is not lane authority. The committed
`ResourceLaneClaimed` event is authority. For run identity, the committed `RunAdmitted` event is the
corresponding authority boundary.

## Why Reusing The Same `run_id` Is Not Enough

If a completed stream could be reused for another execution under the same `run_id`, the stream
would become ambiguous:

```text
run_id R
  RunAdmitted
  ...
  RunCompleted
  RunAdmitted again?
  ...
  RunCompleted again?
```

Then status, replay, resume, and public output would need another hidden dimension to answer "which
execution of R?" That hidden dimension is an occurrence key by another name.

So a fresh execution needs a fresh semantic identity. The question is how that identity should be
chosen and how much of it should be public.

## Terms

These names are not final. They describe the concepts involved.

### Certified Spec Hash

The hash of the certified executable workflow. This answers:

```text
What deterministic workflow was certified?
```

It should not be polluted with wall-clock time, random nonces, scheduler ticks, or process-local
state. Operation planning should remain deterministic.

### Invocation Key / Occurrence Key

The identity of one intended execution occurrence. This answers:

```text
Why is this concrete run different from another run of the same certified workflow?
```

Examples:

```text
manual:<opaque nonce>
client-request:<idempotency key>
schedule:<schedule_id>:tick:<due_at>
```

The raw key should not be persisted if it might contain caller material. Persisting only a
domain-separated digest is enough for run identity.

### Overlap Key

The key for coordinating concurrently active occurrences of the same base work. This answers:

```text
Is there already an active occurrence for this op/config/series?
```

This is operational coordination. It can prevent duplicated live work, but it should not replace
run identity.

### Resource Lane Key

The key for fencing an external side effect, such as a wallet/signer lane. This answers:

```text
Which external resource must not be mutated concurrently?
```

This belongs inside an already admitted run and is governed by side-effect ledger semantics.

## Important Distinctions

These concerns are related but should not be collapsed:

```text
run identity          -> which append-only run stream exists?
start overlap         -> should this new occurrence be started while another is active?
execution claim       -> which process may currently drive this run_id?
resource lane         -> which side-effect resource is fenced inside the run?
side-effect recovery  -> how do we avoid duplicating external mutations?
```

The current confusion comes from trying to make `run_id` answer too much, then considering whether
lanes should answer the missing pieces.

## Examples

### Manual Snapshot

Desired behavior:

```sh
mfm run start --op portfolio_snapshot --config portfolio.toml
```

Later:

```sh
mfm run start --op portfolio_snapshot --config portfolio.toml
```

These should normally be two different portfolio observations.

If the two commands happen at the same time, we may want only one live observation and one
`already_active` or `skipped` result. That is an overlap policy question, not a reason to reuse a
completed run forever.

### Retry-Safe API Start

A client that needs idempotent retries should provide the occurrence identity:

```text
invocation_key = client-request-123
```

Retrying with the same key attaches to the same run. Using a different key creates a different run.

### Scheduled Rebalance

An hourly rebalance should get one occurrence per scheduled tick:

```text
invocation_key = schedule:rebalance-main:tick:2026-07-07T14:00:00Z
```

Inside that run, wallet side effects still use resource lanes, side-effect idempotency, receipts,
and recovery evidence.

### Snapshot Warning

A warning workflow that sends Telegram is not read-only; the notification is a side effect.

That does not change launch semantics. It still has one invocation per manual start or schedule
tick. The Telegram side effect should include occurrence/tick/rule/recipient material in its own
idempotency and recovery evidence so retrying the same occurrence does not double-send.

## Possible Solutions

### Option 1: Keep Current Behavior And Rename `invocation_key`

Rename `invocation_key` to `run_key` or `invocation_key`, but keep it optional.

Identity remains:

```text
certified_spec_hash
trust_scope_id
invocation_key_digest, but only when a caller supplies one
```

Behavior:

- no key means "canonical run for this config";
- same key means attach;
- different key means new run.

Pros:

- smallest implementation;
- no store/runtime semantic change;
- makes the current escape hatch less confusing.

Cons:

- preserves the bad default for public ops;
- users still have to know to provide a key for every fresh snapshot or scheduled tick;
- `run start` still behaves like a cache lookup when no key is supplied.

This is likely too small.

### Option 2: Public Starts Are Always Invocation-Addressed

Make public starts invocation-addressed and make an invocation digest required in run identity
material.

Identity becomes:

```text
certified_spec_hash
trust_scope_id
invocation_key_digest
```

Behavior:

- if caller supplies `--invocation-key`, same key attaches and different key creates a new run;
- if caller omits `--invocation-key`, app mints a fresh opaque key before deriving `run_id`;
- `attached` means "same invocation", not "same config existed before".

Pros:

- matches public op intuition: a start is an execution occurrence;
- side effects and read-only workflows use the same launch model;
- no entry-point launch modes;
- strict `run_id` validation remains intact;
- historical reads remain explicit by `run_id`.

Cons:

- two simultaneous manual starts without a supplied key create two runs;
- clients that need retry safety must provide an invocation key;
- without overlap coordination, duplicated live work is possible.

This solves stale completed snapshots but does not solve simultaneous duplicate starts.

### Option 3: Invocation Identity Plus Start-Overlap Coordination

Use Option 2 for semantic identity, and add an operational overlap guard for public starts.

Flow:

```text
1. parse and plan public op config
2. certify or otherwise derive the base certified_spec_hash
3. compute overlap_key = trust_scope_id + certified_spec_hash
4. check/acquire an overlap lane for that base key
5. if busy and policy is skip_if_active, return already_active/skipped
6. if not busy, mint or accept invocation_key
7. derive run_id from certified_spec_hash + trust_scope_id + invocation_key_digest
8. append RunAdmitted and drive the run under the normal execution claim
```

Default behavior could be:

```text
manual public start:
  overlap_key = trust_scope_id + certified_spec_hash
  overlap_policy = skip_if_active
  invocation_key = caller key or app-minted nonce
```

Then two simultaneous commands:

```sh
mfm run start --op portfolio_snapshot --config portfolio.toml
mfm run start --op portfolio_snapshot --config portfolio.toml
```

would produce one admitted run and one `already_active` or `skipped` response. A later command after
the first run completes creates a fresh run.

Pros:

- matches user intuition for manual starts;
- avoids duplicate live snapshots by default;
- avoids attaching to old completed runs;
- keeps run identity semantic and lanes operational;
- uses a lane-like mechanism for the thing lanes are good at: active overlap coordination.

Cons:

- introduces another internal coordination scope/policy;
- requires a new admission-lane class or equivalent coordination path;
- must be carefully documented as liveness coordination, not semantic authority;
- "skipped" starts need a public output shape that does not pretend a run was created.

This is the strongest candidate if we want good default behavior for both stale completed runs and
simultaneous duplicate starts.

### Option 4: Entry-Point Launch Modes

Add a public entry-point launch mode:

```text
content_addressed
invocation_addressed
```

Some ops would behave like canonical singleton specs. Others would behave like repeatable
occurrences.

Pros:

- explicit;
- supports rare singleton workflows.

Cons:

- adds a public descriptor concept;
- requires users and authors to understand another axis;
- risks misclassification;
- does not by itself solve simultaneous duplicate starts.

This is probably too much for the current problem.

### Option 5: Use Read-Only vs Side-Effecting As The Split

Make `NoSideEffects` workflows fresh by default and side-effecting workflows content-addressed by
default.

Pros:

- uses an existing certified property.

Cons:

- wrong axis;
- `portfolio_rebalance` is side-effecting but repeatable by schedule;
- `portfolio_snapshot_warn` includes notification side effects but is also repeatable;
- side-effect safety belongs to side-effect ledgers, resource lanes, idempotency, and recovery, not
  to launch freshness.

This should be rejected.

### Option 6: Use Execution FIFO/Lanes Instead Of Run Identity

Remove or weaken top-level run identity validation and let lane state decide whether a new execution
can happen.

Pros:

- feels simpler at first;
- mirrors the intuition that lanes serialize work.

Cons:

- violates the Process-Fungible Execution boundary;
- makes mutable liveness rows semantic authority;
- makes replay/status/public-output ambiguous;
- still needs a hidden occurrence dimension to distinguish multiple executions of the same config;
- risks turning stale or incorrectly reaped operational state into semantic behavior.

This should be rejected.

## Current Best Candidate

The current best candidate is Option 3:

```text
public starts are invocation-addressed
plus an operational overlap guard for active duplicate starts
```

This keeps the model split clean:

```text
invocation_key_digest -> semantic run identity
overlap_key           -> active-start coordination
execution claim       -> active driver for one run_id
resource lane         -> side-effect fencing inside the run
```

It also matches the desired behavior:

- same command after completion creates a fresh snapshot;
- same command at the same time does not necessarily duplicate live work;
- scheduled ticks get deterministic invocation keys;
- retries can attach to the same invocation by supplying the same key;
- side effects remain protected by the side-effect machinery, not by launch identity.

## Implemented Resolution

The implementation follows the option-3 shape and removes the old ambiguity:

```text
run identity          = certified_spec_hash + trust_scope_id + invocation_key_digest
execution claim lane  = certified_spec_hash + trust_scope_id
resource lane         = explicit side-effect/resource key from certified state intent
```

Public entry-point starts accept an optional `invocation_key`. If the caller supplies it, retries
target the same append-only run stream. If the caller omits it, the app mints a fresh opaque
invocation key before deriving `run_id`. The raw key is never persisted; only a domain-separated
digest enters `RunIdentityMaterialV1`.

`RunIdentityMaterialV1` always contains `invocation_key_digest`. `run_id` remains strictly derived
from recorded identity material, and `RunAdmitted` remains the semantic authority for attach, resume,
replay, status, and public-output reads.

Public start admission now atomically pairs `RunAdmitted` with execution-claim acquisition for the
base work identity. If another run currently holds that execution lane, the contender does not append
any run event and the response is:

```text
outcome = already_active
active_run_id = <holder run id>
run = absent
```

That keeps the public surface small:

- users see only `invocation_key` when they need retry-stable starts;
- execution claim scope is internal;
- resource lanes remain side-effect/resource authority;
- side-effect idempotency remains state/adapter logic.

The central rule remains unchanged: operational lanes coordinate process activity, but they do not
replace durable run identity or certified side-effect authority.
