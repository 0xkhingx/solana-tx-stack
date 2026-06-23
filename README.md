# Solana TX Stack

Monorepo scaffold for a smart Solana transaction stack with:

- a Rust transaction engine
- a TypeScript AI agent for autonomous retry decisions
- a shared lifecycle log pipeline
- fault injection hooks for expired blockhash and low-tip scenarios

## Project Overview

The repository is split into two layers:

- `rust-engine/` owns chain access, bundle assembly, lifecycle state tracking, and log emission.
- `ts-agent/` watches failed bundle logs, asks Claude for a retry decision, and writes decision files that the Rust side can poll.

The stack is designed around live data:

- blockhashes are fetched at `confirmed` commitment
- tips are derived from the live Jito tip accounts
- retry decisions come from the AI agent, not hardcoded control flow

## Setup

### Rust Engine

1. Install Rust and Cargo.
2. Create `rust-engine/.env` from the root `.env.example`.
3. Set your Solana and Yellowstone credentials.
4. Build the workspace:

```bash
cd rust-engine
cargo build
```

### TypeScript Agent

1. Install Node.js 18+.
2. Create `ts-agent/.env` if needed, or reuse the root `.env.example`.
3. Install dependencies:

```bash
cd ts-agent
npm install
```

## Run the Full Stack

1. Start the Rust engine first so it can produce `logs/lifecycle.jsonl` and `logs/tips.json`.
2. Start the TypeScript agent so it can watch the log and decision directories.
3. Point both components at the same repo root.

Example flow:

```bash
cd rust-engine
cargo run
```

In a second terminal:

```bash
cd ts-agent
npm run dev
```

## Fault Injection

The Rust `fault-injector` crate provides deliberate failure modes for testing the retry pipeline:

- `ExpiredBlockhashFault` fetches a blockhash, waits 80 slots, then submits so the hash is guaranteed to expire.
- `LowTipFault` submits with a 1 lamport tip to provoke bundle rejection.

These are wired for reproducible tests of logging, agent decisions, and retry handling.

## AI Decision Flow

The agent watches `logs/lifecycle.jsonl` for new failed bundles.

When it sees a failure with `retry_count === 0`, it:

1. loads recent tip percentile data from `logs/tips.json`
2. reads the last 3 bundle outcomes for context
3. sends a strict JSON-only prompt to Claude
4. writes the resulting decision to `decisions/{bundle_id}.json`

The Rust side can poll the decision queue and decide whether to re-submit with a refreshed blockhash and new tip.

## Questions

Q1: What does the delta between `processed_at` and `confirmed_at` tell you about network health?

The delta between processed_at and confirmed_at reflects how long the cluster took to reach supermajority stake-weighted agreement on the block containing the bundle. On a healthy network this is typically 400–800ms, representing two or three voting rounds. A large delta — anything over 2 seconds — signals elevated validator vote latency, possible fork resolution overhead, or stake concentration causing slower agreement. This delta is operationally useful: if it is growing across successive submissions, it is a signal to hold new bundles rather than submit into an unhealthy slot window.

Q2: Why should you never use finalized commitment when fetching a blockhash for a time-sensitive transaction?

Finalized commitment lags the confirmed tip of the chain by 31 or more slots, which is roughly 13 seconds on mainnet. A blockhash fetched at finalized is already well into its expiry window — Solana blockhashes expire after approximately 150 slots, around 60 seconds. Using finalized for a time-sensitive bundle submission wastes a significant portion of that window before the transaction is even constructed. The correct commitment for blockhash fetching is confirmed, which reflects the most recent slot the cluster has reached quorum on without the finalization lag.

Q3: What happens to your bundle if the Jito leader skips their slot?

If the Jito leader skips their slot, the bundle is dropped. Jito bundles are routed to the current leader's block engine instance. If that leader does not produce a block — whether due to being offline, delinquent, or simply skipping — the bundle never enters a block and is silently discarded. The stack must detect this by watching the slot stream: if slots advance without a block from the expected leader, that is the signal to resubmit to the next Jito-enabled leader window. This is one of the reasons stream-based slot observation is mandatory — RPC polling alone is too slow to detect leader skips in time to act.
