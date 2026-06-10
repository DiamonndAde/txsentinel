# TxSentinel

**Advanced Infrastructure Challenge — Smart Transaction Stack**
Superteam Earn Hackathon | $5,000 Prize Pool

A Rust-native smart transaction infrastructure that streams live Solana slot data (Yellowstone/Geyser gRPC), uses an AI agent (DeepSeek) to make real-time priority fee and retry decisions, submits Jito bundles with dynamic tips, and tracks every transaction through its full lifecycle — all displayed in a live Ratatui terminal dashboard.

**🎬 Demo video:** [Watch on Google Drive](https://drive.google.com/file/d/1OVcxEEbWPeMjMlUdaxXuDP56ur0GoDY0/view?usp=sharing)
**📐 Architecture document:** [View on Notion](https://app.notion.com/p/ARCHITECTURE-37b500afcefb80fbb201df5e1641249e?source=copy_link) (also in this repo: [ARCHITECTURE.md](./ARCHITECTURE.md))

---

## Demo

The TUI shows six live panels simultaneously:

```
⚡ TxSentinel — Smart Solana Transaction Stack  [devnet]  [q] quit  [s] submit bundle  [f] inject fault
┌─────────────────────────┐ ┌────────────────────────┐ ┌────────────────────────┐
│  SLOT STREAM            │ │  TIP MARKET (lamports) │ │  LEADER WINDOW         │
│  Slot       312,847,203 │ │  p25        1,000 L    │ │  Mode    RPC Fallback  │
│  TPS               3240 │ │  p50        5,000 L    │ │  Jito    mainnet-only  │
│  Load            HIGH   │ │  p75       12,000 L    │ │                        │
│  Bundles              7 │ │  p95       35,000 L    │ │  ▶ READY (devnet)      │
│                         │ │  p99      100,000 L    │ │                        │
└─────────────────────────┘ └────────────────────────┘ └────────────────────────┘
┌─── ACTIVE BUNDLES ─────────────────────────────────────────────────────────────┐
│ #  Signature    Stage        Tip (L)  Slot        Latency   AI Tip  Baseline  │
│ #1 a4f2b8c1...  FINALIZED ✓  12000    312847190   1240ms    12000   5000      │
│ #2 7d9e1234...  CONFIRMED    12000    312847198   820ms     12000   5000      │
└────────────────────────────────────────────────────────────────────────────────┘
┌─── AI AGENT REASONING (DeepSeek)    ───────────────────────────────────────────┐
│  TPS at 3240 indicates high congestion on this slot epoch                      │
│  p50 baseline of 5000L has only 58% landing rate in last 5 bundles            │
│  Recommending p75 (12000L) to target top-third of fee market                  │
│  Leader window: 3 slots until Jito — submit now for current leader             │
│  -> Fee: 12000L  [p75]  (baseline: 5000L)                                     │
└────────────────────────────────────────────────────────────────────────────────┘
```

---

## Technical Questions

### 1. What does the delta between `processed_at` and `confirmed_at` tell you about network health?

The processed→confirmed delta is the time from when a transaction first appears in a processed block to when that block accumulates 66%+ supermajority stake votes (the `confirmed` commitment level). This delta is the most sensitive real-time health signal available without running a validator.

**Low delta (< 400ms)**: The network is healthy. Validators are voting quickly, stake is distributed and online, and vote latency is low. In practice this is one or two slots.

**High delta (> 2s)**: Indicates vote propagation problems. Could be a large validator offline (reducing available stake), network partitioning, or a fork being resolved. This is a leading indicator of degraded finality — confirmed blocks may be taking longer than usual to cross the 2/3 supermajority threshold.

**Very high delta (> 10s)**: The network may be experiencing a slow-down or outage. Transactions that appear processed may be on a fork that gets orphaned, causing them to never reach confirmed or finalized status.

In TxSentinel, this delta is computed per bundle and fed back to the AI agent as the `avg_delta` context variable. A rising delta causes the AI agent to increase its tip recommendation, since higher congestion correlates with validator resource pressure, and a higher tip increases the probability that validators prioritize your transaction over competing ones.

---

### 2. Why should you never use finalized commitment when fetching a blockhash for time-sensitive transactions?

A Solana blockhash is valid for a window of **150 slots** (~60-90 seconds). The commitment levels describe how far behind the current tip of the chain a given block is:

| Commitment | Lag | Slots "spent" from window |
|------------|-----|--------------------------|
| `processed` | 0 slots | 0 slots used |
| `confirmed` | ~2 slots | ~2 slots used |
| `finalized` | ~32 slots | ~32 slots used |

When you call `getLatestBlockhash` with `finalized` commitment, the returned blockhash is from a block that was finalized ~32 slots ago. You have already consumed roughly 32 of your 150 available slots before you even begin submitting. For time-sensitive transactions (Jito bundles, MEV, liquidations, arbitrage) this is a significant disadvantage.

The correct approach — and what TxSentinel uses — is to fetch at `confirmed` commitment (approximately 2 slots behind processed). This gives you a blockhash that is highly unlikely to be forked off (it has supermajority stake) while retaining almost the full 150-slot validity window.

The `processed` commitment gives the absolute freshest blockhash but that block could still be orphaned in a fork. `confirmed` is the optimal tradeoff: safe from forks, minimal lag.

---

### 3. What happens to your bundle if the Jito leader skips their slot?

Jito leaders are assigned by the Solana leader schedule — every validator gets 4 consecutive slots when they are the designated block producer. Jito leaders are validators that have installed the Jito-Solana client, which includes the block engine integration. **If a Jito leader skips their assigned slot** (they go offline, miss the slot, or produce an empty block), several things happen:

1. **The bundle is never included**: Jito bundles are routed specifically to the current leader's block engine. If the leader does not produce a block, the bundle cannot land in that slot. The block engine does not forward bundles to the next leader.

2. **The blockhash clock keeps advancing**: Every skipped slot still consumes 400ms and counts against your 150-slot blockhash window. A skipped slot is "dead time" — you can't recover it.

3. **The bundle must be resubmitted**: After detecting a leader skip (either via timeout or via monitoring the `getLeaderSchedule` and observing no block in the expected slot), the correct response is to fetch a new blockhash and resubmit the bundle to the *next* Jito leader in the schedule.

4. **Tip strategy changes**: The next Jito leader window may have different competition levels. TxSentinel's AI agent handles this: when `decide_retry()` is called with `FailureKind::LeaderSkipped`, it checks `slots_until_jito` in its context and may recommend waiting for the next Jito window rather than submitting to a non-Jito leader (which would be a regular priority-fee transaction, not a bundle).

In TxSentinel, the slots-until-next-Jito-leader is estimated from `current_slot % 4` and displayed in the LEADER WINDOW panel. The AI agent uses this to time submissions for maximum landing probability.

---

## Features

- **Live Yellowstone gRPC slot streaming** — real-time slot/TPS from Solana's Dragon's Mouth interface, with auto-reconnect
- **AI-driven tip decisions** — DeepSeek (configurable: V3 for speed, R1 for explicit chain-of-thought) evaluates tip percentiles, TPS, leader window, and historical landing rates before every submission
- **Jito bundle construction** — 2-transaction atomic bundles (main tx + tip tx) with base64 encoding for the Jito JSON-RPC API
- **Counterfactual ledger** — every bundle records both the AI tip and the p50 baseline; SQLite-persisted for post-session analysis
- **Full lifecycle tracking** — Submitted → Processed → Confirmed → Finalized with millisecond-precision timestamps at each transition
- **Fault injection** — `[f]` captures the current blockhash; next `[s]` submission uses it as a stale hash, triggering `BlockhashNotFound`, then the AI agent diagnoses and retries
- **Devnet-compatible** — when `NETWORK=devnet`, falls back to regular RPC `sendTransaction` while preserving all AI, bundle-construction, and lifecycle-tracking behavior
- **Ratatui TUI** — six-panel terminal dashboard, no browser required

---

## Architecture

See [ARCHITECTURE.md](./ARCHITECTURE.md) for the full system diagram, module breakdown, dependency analysis, and data flow documentation.

---

## Prerequisites

- Rust 1.75+ (`curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh`)
- A Solana keypair at `~/.config/solana/id.json` (generate: `solana-keygen new`)
- Devnet SOL (get from `https://faucet.solana.com`)
- A DeepSeek API key (`https://platform.deepseek.com`)
- A SolInfra account for Yellowstone gRPC (`https://solinfra.dev`)

---

## Setup

```bash
# 1. Clone and enter the project
cd txsentinel

# 2. Copy env template and fill in your keys
cp ../.env.example .env   # or edit .env directly

# 3. Get devnet SOL (run once)
solana airdrop 2 --url devnet
# or visit https://faucet.solana.com and paste your pubkey

# 4. Build
cargo build --release

# 5. Run
cargo run --release
```

---

## Configuration (`.env`)

```env
RPC_URL="https://api.devnet.solana.com"
GRPC_URL="fra.grpc.solinfra.dev:443"
GRPC_X_TOKEN="your_solinfra_grpc_key"
JITO_BLOCK_ENGINE_URL="https://mainnet.block-engine.jito.wtf"
NETWORK=devnet
DEEPSEEK_API_KEY=sk-your_deepseek_key
DEEPSEEK_MODEL=deepseek-chat   # or deepseek-reasoner for explicit chain-of-thought (slower)
KEYPAIR_PATH=/home/you/.config/solana/id.json
```

For mainnet operation: change `RPC_URL` to a mainnet RPC, change `NETWORK` to `mainnet-beta`, and ensure you have mainnet SOL in your wallet.

---

## Controls

| Key | Action |
|-----|--------|
| `s` | Submit a bundle — AI agent decides the tip, constructs the bundle, submits and tracks it |
| `f` | Inject a fault — queues an invalid blockhash; next `[s]` submission fails with `BlockhashNotFound`, triggering AI diagnosis and retry |
| `j` / `k` | Scroll the AI reasoning panel down / up |
| `q` | Quit |

---

## Tech Stack

| Component | Technology |
|-----------|-----------|
| Language | Rust (async, tokio) |
| Slot streaming | Yellowstone gRPC (`yellowstone-grpc-client 13.1`) |
| Bundle submission | Jito Block Engine JSON-RPC |
| AI agent | DeepSeek (`deepseek-chat` or `deepseek-reasoner`) via OpenAI-compatible API |
| Persistence | SQLite via `rusqlite` (bundled) |
| TUI | Ratatui 0.29 + Crossterm |
| Solana SDK | `solana-sdk 2.1` |

---

## Testing Guide

See [TESTING.md](./TESTING.md) for a complete step-by-step walkthrough of how to run, test every feature, generate lifecycle log entries, and record the demo video.

---

## Bounty Submission

Built for the **Advanced Infrastructure Challenge** on Superteam Earn.

Requirements checklist:
- [x] Rust implementation
- [x] Yellowstone/Geyser gRPC streaming with slot monitoring
- [x] Jito bundle construction with dynamic tip calculation
- [x] AI agent making real operational decisions (not decorative)
- [x] Transaction lifecycle tracking (Submitted → Processed → Confirmed → Finalized)
- [x] Failure classification and AI-driven retry logic
- [x] Counterfactual ledger (AI tip vs naive p50 baseline)
- [x] SQLite persistence for post-session analysis
- [x] Working prototype (devnet)
