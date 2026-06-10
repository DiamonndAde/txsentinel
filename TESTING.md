# TxSentinel — Step-by-Step Testing Guide

This guide walks you through running TxSentinel from zero and exercising every feature.

---

## What this project does (plain English)

TxSentinel is a smart transaction tool for the Solana blockchain. It:

1. **Watches the blockchain in real time** using a streaming connection (Yellowstone gRPC, with automatic RPC fallback). You see live slot numbers and estimated transactions-per-second.
2. **Asks an AI** (DeepSeek) how much priority fee to attach to each transaction, based on current network congestion and the live Jito tip market.
3. **Builds and submits transactions** to Solana. On mainnet it uses Jito bundles (atomic submissions with tips). On devnet it falls back to regular RPC submission.
4. **Tracks each transaction** through its four confirmation stages: Submitted → Processed → Confirmed → Finalized — recording the exact time of each transition.
5. **Shows everything in a live terminal dashboard** with six panels.

---

## Step 1: Prerequisites

- Rust 1.75+ (`curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh`)
- A Solana keypair: `solana-keygen new` (created at `~/.config/solana/id.json`)
- Devnet SOL: go to **https://faucet.solana.com**, paste your wallet address (`solana address`), select "Devnet", request 2 SOL

Verify your balance:

```bash
solana balance --url devnet
```

---

## Step 2: Configure `.env`

Copy the template at the project root and fill in your keys:

```bash
cp .env.example .env
```

```env
RPC_URL="https://api.devnet.solana.com"
NETWORK=devnet
GRPC_URL="fra.grpc.solinfra.dev:443"
GRPC_X_TOKEN="<your gRPC x-token>"
JITO_BLOCK_ENGINE_URL="https://mainnet.block-engine.jito.wtf"
DEEPSEEK_API_KEY=<your DeepSeek API key>
DEEPSEEK_MODEL=deepseek-chat
KEYPAIR_PATH=~/.config/solana/id.json
```

`NETWORK=devnet` activates devnet mode (regular RPC submission instead of Jito bundles).

---

## Step 3: Build and run

```bash
cd txsentinel
cargo build
cargo run
```

The terminal switches to a full-screen dashboard. Within a few seconds you should see:

- **SLOT STREAM** (top left): live slot number (via gRPC, or RPC fallback within ~1s), estimated TPS, load level
- **TIP MARKET** (top middle): live p25/p50/p75/p95/p99 Jito tip percentiles in lamports — real mainnet fee-market data, refreshed every 10s
- **LEADER WINDOW** (top right): on devnet shows "Mode: RPC Fallback / READY (devnet)"; on mainnet shows slots until the next Jito leader
- **Status bar** (bottom): current operation, then your wallet balance

Diagnostic logs are written to `txsentinel.log` (tail it from another terminal if needed).

---

## Step 4: Test a normal submission (`s` key)

Press `s`. What happens:

1. Status bar: "AI agent deciding priority fee..."
2. TxSentinel calls DeepSeek with the live tip percentiles, TPS, leader window, and recent landing rate (~3–8 seconds)
3. The AI REASONING panel fills with the model's step-by-step analysis: which TPS band applies, which percentile to target, and the interpolation math if the target falls between known percentiles
4. The final line shows the decision: `-> Fee: 12000L  [p75]  (baseline: 5000L)`
5. The bundle is built and submitted; ACTIVE BUNDLES gains a row
6. The stage advances: SUBMITTED → PROCESSED → CONFIRMED → FINALIZED
7. On finalization, the "Bundles" counter increments

**In the lifecycle log**, the `→Proc`, `→Conf`, `→Fin` columns show per-stage timing. Typical devnet values: submit→processed ~1,100ms, processed→confirmed ~50–100ms, confirmed→finalized ~11–14s. The tiny processed→confirmed delta is the network-health signal discussed in the README.

Scroll the AI panel with `j` (down) and `k` (up) to read the full reasoning.

---

## Step 5: Test fault injection (`f` then `s`)

This exercises the AI failure-recovery path by deliberately breaking a transaction.

1. Press `f` — an invalid blockhash is queued (one that cannot exist on the ledger, so failure is guaranteed)
2. Press `s` — the bundle is built and signed with the bad blockhash
3. The submission fails: status shows `Failed: BlockhashExpired`, and the lifecycle log gets a red FAILED entry with `BlockhashExpiry` in the Fault column
4. The AI diagnoses the failure in the reasoning panel — identifying the blockhash expiry, deciding to retry, and often bumping the fee
5. The retry resubmits with a fresh blockhash and lands on-chain as a new entry

The panel keeps both the failure diagnosis and the retry reasoning visible — the full story of fail → diagnose → recover from one keypress.

---

## Step 6: Build up the lifecycle log

For a representative session, generate 10+ entries with a mix of outcomes:

1. Submit 4 normal bundles (`s`, waiting for each to confirm)
2. Inject a fault and watch the retry (`f`, `s`)
3. Submit 3 more (`s` × 3)
4. Inject another fault (`f`, `s`)
5. Submit 2 more (`s` × 2)

Watch the **AI Tip vs Baseline** columns: the baseline is always the naive p50, while the AI's choice varies with conditions. Under high TPS the gap can be 10–100× — that spread is the counterfactual ledger demonstrating the AI adds measurable value over a fixed strategy.

---

## Step 7: Inspect the SQLite persistence (optional)

The lifecycle log is persisted to `txsentinel.db`. From a separate terminal:

```bash
cd txsentinel
sqlite3 txsentinel.db "SELECT signature, stage, tip_lamports, ai_tip_decision, baseline_tip, processed_to_confirmed_ms, injected_fault FROM bundle_entries ORDER BY submitted_at DESC LIMIT 10;"
```

Every row stores the AI decision, the p50 baseline it was compared against, full stage timings, and any injected fault.

You can also verify any signature on-chain at `https://explorer.solana.com/?cluster=devnet`.

---

## Step 8: Quit

Press `q`. The terminal returns to normal. (If the display ever looks corrupted, run `reset`.)

---

## Troubleshooting

**"Missing env var: RPC_URL"**
→ Ensure `.env` exists at the project root (the app searches parent directories automatically).

**Slot number stuck at "connecting..."**
→ Both gRPC and RPC are unreachable — check your network. Normally the RPC fallback populates the slot within ~1 second even if gRPC is down.

**TPS shows a value but gRPC errors appear in `txsentinel.log`**
→ Expected when the gRPC provider is degraded: TPS is being derived from RPC slot-delta polling. The gRPC stream takes over automatically once it connects.

**"Failed to read keypair"**
→ Run `solana-keygen new`, or point `KEYPAIR_PATH` in `.env` at your keypair file.

**Transaction fails with "insufficient funds"**
→ Get devnet SOL from `https://faucet.solana.com`.

**AI call times out**
→ Calls have a 45s hard timeout. `deepseek-chat` typically responds in 3–8s; `deepseek-reasoner` (R1) can take 1–2 minutes due to its chain-of-thought. Use `deepseek-chat` for interactive sessions.

---

## Feature → Requirement Map

| Bounty requirement | Where to see it |
|---|---|
| Yellowstone gRPC slot streaming | SLOT STREAM panel (live slot/TPS); `src/slot_monitor.rs` |
| Jito bundle construction + dynamic tips | `src/bundle/builder.rs`, TIP MARKET panel (live tip-floor percentiles) |
| AI agent making real decisions | AI REASONING panel — decisions vary with live conditions, never hardcoded |
| Full lifecycle tracking | LIFECYCLE LOG with per-stage millisecond timings |
| Failure classification + retry | Fault injection demo (`f` + `s`) |
| Counterfactual ledger | AI Tip vs Baseline columns; persisted in `txsentinel.db` |
