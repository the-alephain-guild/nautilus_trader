# ATR-normalized margin strategy

Trades short-horizon binary outcome markets on the proposition that, as settlement
approaches, the distance between the current reference level and the settlement baseline
grows large relative to what the remaining volatility can traverse. The distance is
normalized by a directional ATR and compared against configured thresholds.

## Rules

With `TP` the settlement baseline, `CP` the current reference level, and the ATR measured
over completed bars of the same feed:

| Rule | Condition | Action | Reading |
|---|---|---|---|
| thick lead | `CP > TP` and `(CP−TP) / down_atr >= 1.05` | buy up | a routine decline cannot erase the lead |
| thin lead | `CP > TP` and `(CP−TP) / down_atr <= 0.90` | buy down | a routine decline erases it |
| near gap | `CP < TP` and `(TP−CP) / up_atr <= 0.80` | buy up | a routine advance closes the gap |
| far gap | `CP < TP` and `(TP−CP) / up_atr >= 1.04` | buy down | an advance cannot close it |

Ratios between `0.90` and `1.05`, or between `0.80` and `1.04`, produce no order. That gap
is deliberate: it is the band where the outcome is genuinely undecided.

A lead is measured against the **downside** ATR and a deficit against the **upside** ATR,
because a lead is only threatened by a decline and a deficit only by an advance. A
conventional ATR is direction-agnostic and cannot express this, so the two directions are
accumulated separately as `mean(high − open)` and `mean(open − low)`.

## Why the reference feed must be the settlement feed

These markets resolve against a specific published data stream, and the market description
states the resolution source explicitly, including that it is not any spot market. The
strategy therefore reads `TP`, `CP` and the ATR from that same stream.

Substituting a spot feed is not merely imprecise. A *constant* offset does cancel, since
both `TP` and `CP` carry it and the rules only use `CP − TP`. What does not cancel is the
offset's **drift** between the instant the baseline is taken and the instant the current
level is read — that enters the ratio undiminished, and the no-trade band is only 15% wide.

## Time-scale matching

A one-minute ATR compared against a two-minute remaining horizon understates the reachable
range. With `scale_atr_by_remaining` the ATR is multiplied by the square root of the
remaining bar count, which is the matching correction under a random walk. Disabling it
reproduces a fixed one-minute band.

## Entry price bound

At an entry price of `p`, a win returns `1 − p` and a loss costs `p`, so the break-even win
rate **equals** `p`. An entry at 0.95 must therefore be right 95% of the time merely to
break even, and the payoff ratio degrades further as `p` approaches 1.
`max_entry_price` is the control on that asymmetry; entries outside
`[min_entry_price, max_entry_price]` are refused and counted.

## Leg pairing

Each interval is a distinct instrument pair, so legs are not configured. They are paired at
runtime from the venue's event identifier and outcome label, and markets arriving without an
event identifier are skipped rather than traded on one leg — a vote could otherwise be
filled on the wrong side. A down vote **buys the down token** rather than selling the up
token, because under the conditional token framework a naked sale would require splitting
collateral first.

## Baselines that cannot be established

The baseline is the reference level at the market's activation instant. A market whose
activation instant is not covered by an observation within tolerance is marked unavailable
and never traded: trading it would require inferring the level the outcome resolves against.
This is expected for whatever market is already open when the strategy starts.

## Declines are counted, not silent

A strategy that quietly does nothing is indistinguishable from one whose feed has stopped.
Every path that declines to trade increments a named counter — `no_atr`, `no_baseline`,
`no_leg`, `no_quote`, `price_bounds`, `undecided`, `rule_filtered`, `unpaired` — and the
totals are logged on stop alongside the submitted count.

## Running the paper configuration

```bash
cargo run -p nautilus-polymarket --features examples --example polymarket-atr-margin-sandbox
```

Live market data drives a simulated matching engine: no orders reach the venue and no funds
are at risk. `queue_position` and `liquidity_consumption` are enabled so a resting order's
fill stays contingent on the flow ahead of it — the assumption a replay cannot check.

The default configuration restricts trading to the thick-lead rule. In the exploratory
study that preceded this implementation it was the only rule with a positive point
estimate; the other three were negative or statistically indistinguishable from zero.

## Tests

```bash
cargo test -p nautilus-polymarket --features examples --example polymarket-atr-margin-sandbox
```

The decision kernel is kept free of engine types so the rules can be exercised directly:
a rule reachable only through a running engine tends to be verified only where an outer
guard has already decided the outcome.

`verdict_domains_are_exhaustive_and_disjoint` enumerates the ratio axis on both sides and
asserts each ratio receives the verdict the thresholds define. Asserting only that every
rule is *reachable* would pass even if two domains overlapped or left a hole.

The suite was mutation-checked before being relied on. Swapping the upside and downside ATR,
weakening the thick-lead comparison from `>=` to `>`, and removing the square-root scaling
each turn it red; all pass again once reverted.

## Two defects only a live run exposed

Both compiled and passed the unit suite; neither is visible without starting the node.

**The reference subscription never reached the adapter.** A custom `DataType` carries no
venue, so the engine cannot infer which client should receive it. Subscribing with no
client id registers a handler on the message bus and stops there — the adapter is never
told, and the strategy receives nothing while appearing healthy. Passing the client id
explicitly fixes it. Worth noting that the two log lines that would have shown this are
at `debug` level, so at `info` the failure is entirely silent.

**Instrument re-publication reset the legs.** The adapter re-publishes definitions
periodically. The original registration path rebuilt the leg each time, discarding quotes
already received. Registration is now idempotent per instrument.

A third symptom turned out not to be a defect: a `1013` websocket close on the market
stream is the venue asking for a retry, and the client reconnects and restores its
subscriptions on its own.

## What the short verification runs changed

Four defects and one trade-off, none of which the unit suite could reach.

**The venue's activation field is not the interval's open.** It carries the instant the
*market* was created, which for a recurring series precedes the interval it trades by
weeks. Every baseline lookup therefore landed outside any observation and every market was
refused. The open is now derived as `expiration - interval_secs`. Before the fix all four
tracked markets reported an unavailable baseline; after it, only the one already open when
the strategy started — which is correct, since its open cannot be observed.

**A market refused for want of a baseline left no record at all.** Both the decision path
and the settlement path returned early before journalling, so a run that refused
everything produced an empty file, indistinguishable from a run whose feed never started.
Settlements are now written whether or not a baseline existed, carrying a `skip_reason`.

**Counters lived only in memory.** A run ended by a signal never reaches its stop handler,
so the counters vanished exactly when the run needed explaining. A `heartbeat` record now
snapshots them into the journal every minute; it doubles as proof the journal is live.

**Decisions fired at the early edge of the window rather than at the target.** The
eligibility test admitted `target ± tolerance`, so the first observation to enter the band
triggered it: with a 75-second target the first evaluations landed at 128 seconds. The
upper bound is now the target itself and the tolerance only extends backwards, which puts
the first evaluation at 74.9 seconds.

**The trade-off:** in one verification window the thick-lead rule fired 32 times while the
market sat at 0.98, and every one was refused by the 0.95 entry bound. That is the bound
working as intended — a 0.98 entry needs a 98% win rate to break even — but it means a
meaningful share of signals will not convert, and the order rate will run below the
trigger rate.

Note that one market can produce many decision records: a refusal leaves it open to
re-evaluation, and one verification window was evaluated 58 times across its decision
window. Records carry `evaluation_index` so a consumer can collapse them; the settlement
records, one per market, are what statistics should be computed from.
