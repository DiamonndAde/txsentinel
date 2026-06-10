# TxSentinel — Architecture Document

**Advanced Infrastructure Challenge: Smart Transaction Stack**
Superteam Earn Hackathon | June 2026

---

## Overview

TxSentinel is a Rust-native smart transaction infrastructure that combines real-time Solana slot streaming (Yellowstone/Geyser gRPC with RPC fallback), AI-driven priority fee and tip decisions (DeepSeek), and atomic Jito bundle submission — all visualised in a live terminal dashboard (Ratatui TUI). The system tracks every transaction through its full lifecycle (Submitted → Processed → Confirmed → Finalized) and maintains a counterfactual ledger that proves the AI agent beats a naive p50 baseline in real time.

---

## System Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                    TxSentinel Process                           │
│                                                                 │
│  ┌──────────────┐   slot/TPS    ┌────────────────────────────┐ │
│  │ Yellowstone  │──────────────▶│  SlotMonitor               │ │
│  │ gRPC Stream  │               │  (broadcast channel)       │ │
│  │ (SolInfra)   │               └────────────┬───────────────┘ │
│  └──────────────┘                            │ SlotState       │
│                                              ▼                 │
│  ┌──────────────┐  tip floor   ┌────────────────────────────┐ │
│  │ Jito Block   │◀────────────▶│  TipOracle                 │ │
│  │ Engine API   │              │  p25/p50/p75/p95/p99 + EMA │ │
│  └──────────────┘              └────────────┬───────────────┘ │
│                                             │ TipPercentiles  │
│  ┌──────────────┐  reasoning   ┌────────────▼───────────────┐ │
│  │ DeepSeek API │◀────────────▶│  AiAgent                   │ │
│  │ (V3 / R1)    │              │  decide_tip() / retry()    │ │
│  └──────────────┘              └────────────┬───────────────┘ │
│                                             │ AgentDecision   │
│                                ┌────────────▼───────────────┐ │
│                                │  BundleBuilder             │ │
│                                │  main tx + tip tx          │ │
│                                └────────────┬───────────────┘ │
│                                             │ Bundle          │
│  ┌──────────────┐  bundle_id   ┌────────────▼───────────────┐ │
│  │ Jito Block   │◀────────────▶│  JitoSubmitter             │ │
│  │ Engine       │  status poll │  sendBundle / getStatus    │ │
│  └──────────────┘              └────────────┬───────────────┘ │
│                                             │                 │
│                                ┌────────────▼───────────────┐ │
│                                │  LifecycleTracker          │ │
│                                │  in-memory state machine   │ │
│                                └────────────┬───────────────┘ │
│                                             │                 │
│                                ┌────────────▼───────────────┐ │
│                                │  LifecycleLog (SQLite)     │ │
│                                │  persists all BundleEntry  │ │
│                                └────────────┬───────────────┘ │
│                                             │                 │
│                                ┌────────────▼───────────────┐ │
│                                │  Ratatui TUI               │ │
│                                │  6-panel live dashboard    │ │
│                                └────────────────────────────┘ │
└─────────────────────────────────────────────────────────────────┘
```

---

## Module Breakdown

### `src/config.rs`
Loads all configuration from `.env` via `dotenvy`. Fields: `rpc_url`, `grpc_endpoint`, `grpc_x_token`, `jito_url`, `deepseek_api_key`, `deepseek_model`, `keypair_path`, `is_devnet`, `network`. Setting `NETWORK=devnet` activates the RPC fallback path automatically.

### `src/rpc.rs` — Minimal JSON-RPC Client
Direct `reqwest` HTTP calls to the Solana RPC endpoint. Implemented without `solana-client` to avoid the heavy dependency tree and its crypto version conflicts (see Dependency Architecture below).

Key methods:
- `get_latest_blockhash()` — fetches at `confirmed` commitment
- `get_slot()` — current confirmed slot
- `get_transaction_status(sig)` — polls signature confirmation status
- `get_balance(pubkey)` — wallet balance in lamports
- `send_transaction(tx)` — base64-encodes a signed `Transaction` and submits via `sendTransaction` JSON-RPC (devnet fallback)

### `src/slot_monitor.rs` — Yellowstone gRPC Streaming
Connects to Yellowstone/Dragon's Mouth gRPC endpoint using `yellowstone-grpc-client v13.1`. Subscribes to slot updates at `confirmed` commitment level. Updates `SlotState` (current slot, estimated TPS) in a shared `Arc<Mutex<SlotState>>`. Connection attempts are wrapped in a 15-second timeout; on failure or stream end it auto-reconnects with 3-second backoff.

**RPC fallback**: if the gRPC stream has not yet delivered a slot (slot == 0), a background task polls `getSlot` over RPC every ~1 second and derives TPS from the slot delta (`slots/sec × ~2,500 avg transactions per slot`). The moment the gRPC stream comes alive, it takes priority. This guarantees live slot/TPS telemetry even when the gRPC provider is degraded.

### `src/bundle/builder.rs` — Jito Bundle Construction
Builds a 2-transaction Jito bundle:
1. **Main transaction**: Self-transfer of 1 lamport + `SetComputeUnitPrice` (AI-decided fee) + `SetComputeUnitLimit` (5,000 CUs — enough headroom for the two ComputeBudget instructions plus the transfer)
2. **Tip transaction**: Transfer to a Jito tip account (chosen from `getTipAccounts`)

Also builds fault-injection bundles via `build_with_stale_blockhash()` which deliberately signs with an invalid blockhash, causing `BlockhashNotFound` on submission — used to test the AI retry path.

### `src/bundle/submitter.rs` — Jito Bundle Submission
Sends bundles to the Jito Block Engine via the `sendBundle` JSON-RPC method. Polls status via `getBundleStatuses`. Returns structured `BundleStatus` with `landed_slot` and `err` fields.

### `src/bundle/tip_oracle.rs` — Live Tip Market Feed
Fetches `https://bundles.jito.wtf/api/v1/bundles/tip_floor` every 10 seconds (this is Jito's dedicated bundles API host — distinct from the block engine submission host). The API returns values denominated in SOL; the oracle converts to lamports (× 10⁹) and exposes p25/p50/p75/p95/p99 percentiles plus the EMA of landed p50 tips. On HTTP errors or parse failures it degrades gracefully to the last cached values (or realistic defaults on first fetch). The `baseline_tip()` method always returns p50 — this is the "naive" baseline used in the counterfactual ledger.

### `src/lifecycle/types.rs` — Core Data Types
```rust
enum CommitmentStage {
    Submitted,
    Processed,
    Confirmed,
    Finalized,
    Failed(FailureKind),
}

enum FailureKind {
    ExpiredBlockhash,
    FeeTooLow,
    ComputeExceeded,
    BundleFailure,
    LeaderSkipped,
    Unknown,
}

struct BundleEntry {
    signature: String,
    stage: CommitmentStage,
    tip_lamports: u64,
    submitted_slot: u64,
    processed_slot: Option<u64>,
    confirmed_slot: Option<u64>,
    finalized_slot: Option<u64>,
    submitted_at: DateTime<Utc>,
    processed_at: Option<DateTime<Utc>>,
    confirmed_at: Option<DateTime<Utc>>,
    finalized_at: Option<DateTime<Utc>>,
    // timing deltas (computed)
    submitted_to_processed_ms: Option<u64>,
    processed_to_confirmed_ms: Option<u64>,
    confirmed_to_finalized_ms: Option<u64>,
    // AI metadata
    ai_reasoning: Option<String>,
    ai_tip_decision: Option<u64>,
    baseline_tip: Option<u64>,       // p50 counterfactual
    injected_fault: Option<String>,
}
```

### `src/lifecycle/tracker.rs` — State Machine
In-memory `HashMap<String, BundleEntry>` keyed by signature. Methods advance the stage and record precise timestamps at each transition. Syncs to SQLite on every state change.

### `src/lifecycle/log.rs` — SQLite Persistence
Uses `rusqlite` with a bundled SQLite build. Schema has one table: `bundle_entries`. Provides `upsert()` and `all_entries()`. The database file is `txsentinel.db` in the working directory.

### `src/failure/classifier.rs` — Failure Classification
Pattern-matches RPC/Jito error strings to `FailureKind` variants. Patterns are ordered for specificity: compute-budget errors are checked before blockhash errors because Solana RPC error responses contain `"replacementBlockhash"` as a standard JSON field, which would false-trigger a naive substring match on `"blockhash"`. Enables structured reporting in the TUI and structured context for the AI retry agent.

### `src/failure/injector.rs` — Fault Injection
When the `[f]` key is pressed, a deliberately invalid blockhash (a fixed all-`0x01` byte pattern that cannot exist on the ledger) is queued behind a `Mutex`. On the next `[s]` submission, `consume()` pops it and the bundle builder signs with it, guaranteeing an immediate `BlockhashNotFound` failure regardless of timing. The AI retry agent then diagnoses the failure, fetches a fresh blockhash, and resubmits.

### `src/ai/agent.rs` — DeepSeek Decision Engine
Calls the DeepSeek API at `https://api.deepseek.com/chat/completions`. The model is configurable via `DEEPSEEK_MODEL`: `deepseek-chat` (V3, 3–8s latency — default for live demos) or `deepseek-reasoner` (R1 with explicit chain-of-thought, slower). API calls have a 45-second hard timeout. For `deepseek-chat`, the prompt instructs the model to show its step-by-step reasoning (TPS band → percentile target → interpolation math) before the JSON decision; for R1, the `reasoning_content` field is used directly.

**`decide_tip()`**: Given tip percentiles, current TPS, slots until next Jito leader, leader window size, current slot, recent confirmed latency delta, and landing rate — the agent returns a JSON decision:
```json
{
  "tip_lamports": 15000,
  "percentile_used": "p75",
  "summary": "Moderate congestion, using p75 for reliable landing",
  "reasoning": "..."
}
```

**`decide_retry()`**: Given the failure kind, attempt number, previous tip, error slot, and full context — returns:
```json
{
  "should_retry": true,
  "wait_slots": 2,
  "new_tip_lamports": 18000,
  "failure_diagnosis": "BlockhashExpired — blockhash exceeded 150-slot window",
  "summary": "Retry immediately with fresh blockhash, bump tip to p80",
  "reasoning": "..."
}
```

The model's reasoning (chain-of-thought for R1, pre-JSON explanation for V3) is captured and displayed live in the AI panel, and persisted to SQLite alongside the decision.

### `src/ai/tools.rs` — Context Serialization
Converts telemetry data into JSON "tool result" format for the AI system prompt — tip percentiles, leader window, failure context, network health. This structured context is what enables grounded, non-hallucinated AI decisions.

### `src/tui/` — Ratatui Dashboard

Six panels rendered every 100ms:

| Panel | Content |
|-------|---------|
| Title bar | Network (devnet/mainnet), keybindings |
| SLOT STREAM | Current slot, estimated TPS, load level, submission count |
| TIP MARKET | Live p25/p50/p75/p95/p99 in lamports (color-coded by urgency) |
| LEADER WINDOW | Slots until next Jito leader (mainnet) or RPC fallback indicator (devnet) |
| ACTIVE BUNDLES | The 5 most recent bundles with stage, AI tip, baseline, latency |
| AI AGENT REASONING | Scrollable (j/k keys) reasoning buffer, cleared per submission; failure diagnosis + retry reasoning append within the same submission |
| LIFECYCLE LOG | Historical entries with full timing breakdown |
| Status bar | Current operation status |

Keybindings: `[s]` submit bundle, `[f]` inject fault, `[j]/[k]` scroll the AI panel, `[q]` quit. Diagnostic logs are written to `txsentinel.log` (not stderr, which would corrupt the alternate-screen TUI).

---

## Transaction Lifecycle

```
[s] pressed
    │
    ├─▶ AiAgent.decide_tip()  ──── DeepSeek API call
    │         │
    │         ▼ AgentDecision { tip_lamports, percentile_used, reasoning }
    │
    ├─▶ BundleBuilder.build_self_transfer()
    │         │  or build_with_stale_blockhash() if fault injected
    │         ▼ Bundle { main_tx, tip_tx }
    │
    ├─▶ LifecycleTracker.register()  ──── SUBMITTED state, SQLite write
    │
    ├─▶ JitoSubmitter.send_bundle()  ──── POST /api/v1/bundles
    │         │
    │         ▼ bundle_id (UUID)
    │
    └─▶ poll_bundle_status() loop (30 × 2s)
              │
              ├─ "processed"  ──▶ tracker.advance_processed()  → Δsubmit→processed
              ├─ "confirmed"  ──▶ tracker.advance_confirmed()  → Δprocessed→confirmed
              ├─ "finalized"  ──▶ tracker.advance_finalized()  → Δconfirmed→finalized
              └─ error / timeout ──▶ tracker.mark_failed()
```

On devnet, the Jito submission step is replaced by `rpc.send_transaction()` and the polling uses `rpc.get_transaction_status()` (40 polls × 800ms) instead of the Jito bundle status API. All other steps — AI decision, bundle construction, lifecycle tracking, SQLite persistence, TUI display — are identical.

---

## Counterfactual Ledger

Every `BundleEntry` stores two tip values:
- `ai_tip_decision`: the lamport amount chosen by the AI agent
- `baseline_tip`: always `p50` from the tip oracle (naive strategy)

Over a session, the LIFECYCLE LOG panel shows both columns. For bundles that land successfully, comparing `ai_tip_decision` vs `baseline_tip` shows:
- When AI tips higher than p50: faster landing in high-congestion periods
- When AI tips lower than p50: cost savings in low-congestion periods

This is the counterfactual proof that the AI agent adds measurable value over a fixed strategy.

---

## Dependency Architecture

Solana's Rust SDK ecosystem has deep version conflicts around cryptography crates:

| Crate | Version | Why |
|-------|---------|-----|
| `solana-sdk` | `2.1` | Uses `curve25519-dalek 4.x` which is compatible with `zeroize ^1.7` |
| `yellowstone-grpc-client` | `13.1` | No `solana-sdk` dependency — clean isolation |
| `yellowstone-grpc-proto` | `12.4` | Matches client 13.1 |
| `reqwest` | `0.12` + `native-tls` | Avoids `rustls 0.23` which also requires `zeroize ^1.7` via `ring` |

Deliberately excluded: `solana-client`, `solana-transaction-status`, `solana-streamer`, `solana-perf` — all of these transitively pull in `curve25519-dalek 3.x` which requires `zeroize <1.4`, creating an irresolvable conflict with `rustls 0.23`.

---

## External Dependencies (APIs & Services)

| Service | Purpose | Endpoint |
|---------|---------|---------|
| SolInfra gRPC | Yellowstone slot streaming | `fra.grpc.solinfra.dev:443` |
| Solana devnet RPC | Transaction submission + status + slot fallback | `https://api.devnet.solana.com` |
| Jito Bundles API | Live tip floor percentiles | `https://bundles.jito.wtf` |
| Jito Block Engine | Bundle submission (mainnet mode) | `https://mainnet.block-engine.jito.wtf` |
| DeepSeek API | AI tip/retry decisions | `https://api.deepseek.com` |

---

## Security & Safety

- Keypair loaded once from `~/.config/solana/id.json` on startup, never held across await points
- All RPC and API calls use HTTPS/TLS
- gRPC connection uses native TLS roots
- SQLite database stored locally; no external data exfiltration
- API keys loaded from `.env` file, not hardcoded
- Fault injection is opt-in (requires explicit `[f]` keypress) and only affects the immediate next submission

---

## File Structure

```
txsentinel/
├── Cargo.toml
├── src/
│   ├── main.rs              # tokio runtime, TUI event loop, submit_bundle
│   ├── config.rs            # env var loading
│   ├── rpc.rs               # minimal Solana JSON-RPC client
│   ├── slot_monitor.rs      # Yellowstone gRPC slot streaming
│   ├── ai/
│   │   ├── mod.rs
│   │   ├── agent.rs         # DeepSeek decide_tip / decide_retry
│   │   └── tools.rs         # telemetry → AI context serialization
│   ├── bundle/
│   │   ├── mod.rs
│   │   ├── builder.rs       # Jito bundle construction
│   │   ├── submitter.rs     # Jito sendBundle / getBundleStatuses
│   │   └── tip_oracle.rs    # /api/v1/bundles/tip_floor polling
│   ├── failure/
│   │   ├── mod.rs
│   │   ├── classifier.rs    # error string → FailureKind
│   │   └── injector.rs      # stale blockhash fault injection
│   ├── lifecycle/
│   │   ├── mod.rs
│   │   ├── types.rs         # BundleEntry, CommitmentStage, FailureKind
│   │   ├── log.rs           # SQLite persistence
│   │   └── tracker.rs       # in-memory state machine + log sync
│   └── tui/
│       ├── mod.rs
│       ├── app.rs           # App state shared with all background tasks
│       └── ui.rs            # Ratatui draw functions (6 panels)
├── txsentinel.db            # SQLite database (created at runtime)
└── txsentinel.log           # diagnostic log file (created at runtime)
```
