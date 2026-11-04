# Native account capture

`LiveNode.account_capture_handle(client_id)` obtains a read-only transport from the actual execution
registry before the node starts. The first supported owner is `SandboxExecutionClient`. This API does
not start an account, submit an order, authorize execution, or certify an external venue account.

## Owner and lifetime

The registry must contain exactly the requested execution client. The adapter supplies its own probe;
unsupported clients return no probe. The handle records the actual client, account, venue, account type,
client incarnation, API/model revision, configuration JSON and its SHA-256 digest. Currency definitions
include their code, precision, type and ISO identifier instead of inferring identity from an uppercased
symbol. `runtime_version` is the compiled crate version, not a wheel digest. A consuming host must bind
its actual runtime artifact separately.

The API and model revisions describe protocol semantics. A runtime artifact receipt should bind the
actual wheel digest to its source commit, Python ABI, target platform and build features. The consuming
host must verify the installed artifact before admitting a supported model. Crate and distribution
version strings alone cannot establish that this API is present in the installed runtime.

The native probe contains weak, thread-affine references. It is neither `Send` nor `Sync`. Only request
and response values cross the bounded runtime queue. The Python handle sends through this queue and
can be awaited while `run_async` owns the node; it never sends an `Rc<RefCell<_>>` to a Tokio worker.
Disconnect, stop, reset, replacement and native-owner destruction retire the old probe. A new client
has a new incarnation. Source metadata remains historical metadata after retirement, not a fresh read.

A handle must be obtained while the node is idle. Calling `start()` alone does not drive the request
receiver, so an observation times out rather than reading the native state on the caller's thread.
Both `run()` and hosted `run_async()` service the same owner branch. A stop request takes precedence,
and lifecycle is checked again after the synchronous copy and before returning it.

## Snapshot boundary

Capture instrumentation is enabled only when a handle is requested. The owner counts completed
synchronous dispatches, including secondary dispatches and startup drains. Counters and sequence
allocation use checked arithmetic; overflow invalidates capture instead of reusing a watermark.

At the owner branch, each FIFO admission frontier is the applied count plus its receiver depth. The
owner requires every admitted frontier to have been applied and no mutation callback to remain active.
It then copies native state without awaiting, and rereads the frontiers. Work admitted before or during
the copy invalidates the result. Since this branch does not consume ingress while copying, an admitted
message cannot disappear between the two samples. A message admitted after the copy is future work,
not evidence that the earlier snapshot was incomplete. The result carries the node instance, client
incarnation, monotonic capture sequence, source time and explicit frontier vector.

The first implementation supports sandbox nodes with reconciliation disabled. Incomplete report tasks,
external message-bus ingress and opaque stream processors are rejected. Ordinary source channels,
system events and commands, timer callbacks, execution, data and maintenance mutations are tracked.
A temporarily empty queue on its own, or an empty order-status report, is not capture evidence.

## Actual account state

The owner reads the current initialized account. Missing current state returns unavailable even if
`starting_balances` still exists. Frozen accounts and duplicate starting currency definitions are not
supported. Starting balances describe configuration provenance only; they never substitute for current
cash after fills and commissions.

An initialized cash account can have no current balance entries. The capture preserves that empty
array, and a zero balance in another currency does not manufacture a missing base-currency row.
Missing current account state still returns unavailable. A consumer can establish zero equity only
after checking the full capture boundary, the no-borrowing model and its supported currency mappings.

Enumeration includes every current currency, every open account position and every nonterminal account
order. It does not filter by a strategy or configured instrument list. All unassigned orders remain
potential pending intents, because a different venue can still use the sole client's default route.
The adapter cross-checks every matching engine's resting and
conditional order identities. Unapplied native updates, unacknowledged fills and unresolved native
work are unavailable; already applied acknowledgement bookkeeping does not require another market-data
tick merely to be pruned. Conflicting entity borrows return unavailable, not a panic or empty collection.

Amounts are exported from native fixed-point integers as exact decimal text. Floating-point position
averages are not converted into purported exact prices. `no_borrowing_model` is true only for the actual
cash account with borrowing disabled. A margin account observation does not establish debt completeness
or a mark-to-market valuation. This result is not a general NAV calculation or an inventory-complete
flag; the consuming protocol must validate its model, mapping, size, valuation and capability rules.

## Python transport

Obtain the handle from an already configured sandbox node before starting it:

```python
import asyncio
import json

capture = node.account_capture_handle(client_id)
stop = node.handle()

async def observe():
    run = asyncio.ensure_future(node.run_async())
    try:
        while not node.is_running:
            await asyncio.sleep(0.001)
        return json.loads(await capture.capture_json(timeout_ms=1000))
    finally:
        stop.stop()
        await run

result = asyncio.run(observe())
```

A successful response has `status="captured"` and a `capture` object. A failure has
`status="unavailable"` and a closed reason. Queue pressure, timeout, cancellation, an unsupported
runtime, missing account, borrow conflict and retirement never become zero balances or empty state.
Requests are bounded to 32 queued values and at most 60 seconds. Cancelled or timed-out callers cannot
make a late reply valid. Collection is read-only; a separate authorization protocol must revalidate its
own authority before acting on an observation.
