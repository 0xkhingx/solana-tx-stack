# Solana TX Stack Architecture

## 1. System Overview

`solana-tx-stack` is a smart transaction stack for Solana that combines live network observation, transaction assembly, bundle lifecycle tracking, and AI-driven retry decisions.

The system is designed to do one job end to end:

1. observe slots and tip-account state from Yellowstone gRPC
2. derive live tip guidance from current Jito tip-account balances
3. build and submit Solana bundles through the transaction engine
4. track bundle lifecycle transitions as they move through the network
5. write a durable lifecycle log for auditability
6. let a TypeScript agent inspect failures and ask Claude whether to retry
7. feed retry decisions back into the Rust engine through a file-based queue

The stack intentionally avoids hardcoded retry policy and hardcoded tip values. All operational choices are driven by live chain state or by the AI agent's JSON decision output.

## 2. Component Diagram

The main components and their responsibilities are:

- Yellowstone stream subscriber: receives slot updates and tip-account updates from Yellowstone-compatible gRPC
- Transaction engine: constructs transactions and bundles, fetches confirmed blockhashes, tracks lifecycle transitions
- Fault injector: creates deliberate failure inputs for testing the retry path
- Lifecycle logger: persists normalized bundle lifecycle events to JSONL
- TypeScript AI agent: watches failures, asks Claude for a retry decision, writes the decision file
- Jito block engine: receives bundles and determines whether they land, fail, or get dropped

ASCII data flow:

```text
                +----------------------------------+
                | Yellowstone gRPC Endpoint        |
                |  - slot updates                  |
                |  - tip account updates           |
                +------------------+---------------+
                                   |
                                   v
                    +--------------+--------------+
                    | Yellowstone Stream Subscriber|
                    |  - bounded mpsc channel      |
                    |  - broadcast channel         |
                    |  - tips.json writer           |
                    +--------------+--------------+
                                   |
                                   v
                    +--------------+--------------+
                    | Transaction Engine           |
                    |  - build bundle              |
                    |  - fetch confirmed blockhash |
                    |  - lifecycle state machine   |
                    +--------------+--------------+
                                   |
                                   v
                    +--------------+--------------+
                    | Jito Block Engine            |
                    |  - bundle submission         |
                    |  - landing / failure result  |
                    +--------------+--------------+
                                   |
                     lifecycle events / failures
                                   v
                    +--------------+--------------+
                    | Lifecycle Logger             |
                    |  - logs/lifecycle.jsonl      |
                    +--------------+--------------+
                                   |
                                   v
                    +--------------+--------------+
                    | TypeScript AI Agent          |
                    |  - watches failures          |
                    |  - calls Claude              |
                    |  - writes decisions/*.json   |
                    +--------------+--------------+
                                   |
                                   v
                    +--------------+--------------+
                    | Retry Queue Handshake        |
                    |  - retry-queue.json          |
                    +--------------+--------------+
                                   |
                                   v
                    +--------------+--------------+
                    | Transaction Engine           |
                    |  - resubmit bundle           |
                    +----------------------------------+
```

The fault injector plugs into the transaction engine path by supplying intentionally bad inputs, such as an expired blockhash or a 1 lamport tip, so the lifecycle logger and agent can be exercised deterministically.

## 3. Data Flow

The nominal path from slot observation to bundle landing is:

1. The Yellowstone subscriber observes a new slot update.
2. It also receives updates for the five Jito tip accounts and writes the latest tip balances to `logs/tips.json`.
3. The transaction engine consumes the slot and tip signals.
4. The engine fetches a fresh blockhash at `confirmed` commitment.
5. The engine constructs a bundle from versioned transactions and a tip instruction.
6. The bundle is submitted to the Jito block engine.
7. The engine records a lifecycle transition with timestamp and slot metadata.
8. If the bundle is processed, confirmed, and finalized, those transitions are appended to the lifecycle log.
9. If the bundle fails, the failure reason is persisted in the same lifecycle record.
10. The TypeScript agent notices the failure entry and decides whether to retry.
11. If Claude returns `retry`, the agent writes a retry request to `retry-queue.json`.
12. The Rust engine polls the queue, resubmits, and logs the retry attempt.

In practice, the lifecycle log is the source of truth for what happened to each bundle. The queue file is only a control-plane handoff for retries.

## 4. Commitment Level Strategy

The stack uses `confirmed` for blockhash and slot-sensitive RPC calls because time-sensitive Solana transactions need recent chain state, not maximal finality.

Why `confirmed`:

- it is recent enough to avoid stale blockhashes
- it reflects chain progress without waiting for full finality
- it is the right balance for bundle submission latency

Why not `finalized` for blockhash fetching:

- finalized lags behind current leader progress
- by the time a finalized blockhash is returned, it may already be too old for a time-sensitive bundle
- using finalized would increase expiry risk and reduce the usefulness of the retry system

Where finalized still matters:

- final state tracking can record when a bundle is finalized
- lifecycle logging can preserve the finalized timestamp for audit purposes
- the system still wants to know when a successful bundle becomes irreversible

In short: `confirmed` is the operational commitment level for fetching and submitting; `finalized` is an outcome state, not a fetch policy.

## 5. Tip Calculation

Tip selection is driven by live Jito tip-account data, not a fixed constant.

The stream subscriber watches the five canonical Jito tip accounts:

- `96gYZGLnJYVFmbjzopPSU6QiEV5fGqZNyN9nmNhvrZU5`
- `HFqU5x63VTqvQss8hp11i4wVV8bD44PvwucfZ2bU7gRe`
- `Cw8CFyM9FkoMi7K7Crf6HNQqf4uEMzpKw6QNghXLvLkY`
- `ADaUMid9yfUytqMBgopwjb2DTLSokTSzL1zcaozgVFze`
- `DfXygSm4jCyNCybVYYK6DwvWqjKee8pbDmJGcLWNDXjh`

As those accounts update, the engine writes balances to `logs/tips.json`. The tip reader in the TypeScript agent reads that file and derives percentile statistics such as:

- p50
- p75
- p90

The engine then uses the live percentile data to select a dynamic tip amount. The practical effect is that tips adapt to current network conditions instead of assuming a static fee market.

The 75th percentile is a useful default because it aims above the median without always chasing the highest observed value.

## 6. AI Agent Decision Loop

The AI decision loop is deliberately simple and file-based.

1. The lifecycle logger appends a failed bundle entry to `logs/lifecycle.jsonl`.
2. The TypeScript agent watches the log file for new failures.
3. When it finds a record with `failure_reason != null` and `retry_count === 0`, it constructs a structured prompt.
4. The prompt includes:
   - the failure reason
   - the tip amount used
   - the slot at submission
   - recent tip percentile data from `logs/tips.json`
   - the last three bundle outcomes from the lifecycle log
5. The agent sends the prompt to Claude Sonnet.
6. Claude must reply with JSON only.
7. The agent parses the response strictly.
8. If parsing fails, the agent defaults to `abort`.
9. The agent writes the decision to `decisions/{bundle_id}.json`.
10. The retry trigger watches the decisions directory and forwards retry requests into `retry-queue.json`.

The important constraint is that Claude does not directly execute any action. It only emits a structured decision, and the Rust engine remains the execution authority.

## 7. Failure Handling

Each failure type maps to a different retry posture.

### ExpiredBlockhash

This means the bundle was built on a blockhash that aged out before submission or execution.

Expected behavior:

- the failure is recorded in the lifecycle log
- the agent sees the expiry reason and usually prefers retry
- if the agent chooses retry, it should request a refreshed blockhash

Why it happens:

- the blockhash was too old by the time the bundle reached the network
- slot progress outran the submission window

### FeeTooLow

This means the tip or fee was insufficient for the current market conditions or bundle priority.

Expected behavior:

- the failure is logged with the tip used
- the agent uses the current tip percentiles as context
- it may retry with a higher `new_tip_lamports`, or abort if the market is too volatile

Why it happens:

- the bundle was underpriced relative to competing flow
- current tip pressure exceeded the chosen amount

### BundleDropped

This means the bundle did not make it through the block engine path.

Expected behavior:

- the failure is logged as a drop rather than a protocol-level rejection
- the agent evaluates whether a retry with a fresh blockhash and adjusted tip is reasonable
- if the leader skipped the slot or the bundle missed the intended execution window, retry may still be sensible

Why it happens:

- the leader skipped the slot
- the bundle missed its inclusion window
- network conditions or internal queueing prevented landing

## 8. Retry Flow

The retry path is a file handshake between TypeScript and Rust.

Flow:

1. The agent decides `retry`.
2. It writes `decisions/{bundle_id}.json`.
3. The retry trigger reads that file.
4. If `action === "retry"`, it writes `retry-queue.json` at the repo root.
5. The queue entry contains:
   - `bundle_id`
   - `new_tip_lamports`
   - `refresh_blockhash`
6. The Rust engine polls `retry-queue.json` every 200ms.
7. When found, the engine reads the file.
8. The engine deletes the queue file so the request is single-use.
9. The engine logs the retry attempt with a `[RETRY]` prefix.
10. The engine resubmits the bundle using the requested tip and blockhash policy.

ASCII handshake:

```text
Lifecycle failure
      |
      v
logs/lifecycle.jsonl
      |
      v
TypeScript agent -> Claude -> decision JSON
      |
      v
decisions/{bundle_id}.json
      |
      v
retry-trigger.ts
      |
      v
retry-queue.json
      |
      v
Rust engine poll loop
      |
      v
resubmit(bundle_id, new_tip_lamports, refresh_blockhash)
```

This handshake keeps the system decoupled. The agent can make policy decisions without directly invoking Rust internals, and the engine can remain focused on transaction execution.

## 9. Infrastructure Decisions

### Why Rust for the engine

Rust is a good fit for the transaction engine because it needs:

- low-latency RPC and stream processing
- explicit backpressure handling
- strong typing around bundle state and failure reasons
- predictable memory usage
- safe concurrency for live stream ingestion and lifecycle emission

The engine is close to the chain and should be optimized for correctness and operational tightness.

### Why TypeScript for the agent

TypeScript is a good fit for the AI agent because it needs:

- fast iteration on prompt formatting
- straightforward filesystem watchers
- easy SDK integration for Claude
- simple JSON parsing and decision serialization

The agent is policy-heavy, not latency-critical. It benefits more from rapid iteration than from systems-level performance.

## 10. Known Limitations and Tradeoffs

- The retry flow is file-based rather than an in-memory message bus, which is simple and observable but not the lowest-latency design.
- The architecture assumes the Rust engine and TypeScript agent share the same repo root and filesystem semantics.
- The tip percentile model is intentionally simple. It is good enough for dynamic guidance, but it does not model full fee-market microstructure.
- The agent can only make decisions from the log context it is given. If the log history is sparse, its judgment will be less informed.
- The stack depends on Yellowstone and Jito interface compatibility, which can change across releases.
- On Windows, the Rust dependency tree may still require extra toolchain configuration for some Solana/OpenSSL-related crates.
- The current retry queue design is single-entry and file-serialized. It is easy to reason about, but it is not intended for high-volume concurrent retry traffic.

Overall, the design favors operational clarity over maximal throughput. That is the right tradeoff for a smart transaction stack where correctness, observability, and controlled retry behavior matter more than raw message volume.
