# TxSentinel — Step-by-Step Testing Guide

This guide walks you through running TxSentinel from zero, testing every feature, and recording a demo video for the hackathon submission.

---

## Before You Start

### What this project does (plain English)

TxSentinel is a smart transaction tool for the Solana blockchain. It:
1. **Watches the blockchain in real time** using a streaming connection (like a WebSocket, but for Solana — called Yellowstone gRPC). You see live slot numbers and estimated transactions-per-second.
2. **Asks an AI** (DeepSeek R1) how much priority fee to attach to each transaction, based on current network congestion.
3. **Builds and submits transactions** to Solana. On mainnet it uses Jito bundles (advanced atomic submissions). On devnet it uses regular RPC submission.
4. **Tracks each transaction** through its four confirmation stages: Submitted → Processed → Confirmed → Finalized — recording the exact time each stage happens.
5. **Shows everything in a live terminal dashboard** with six panels updating every 100ms.

---

## Step 1: Get devnet SOL

Your wallet address is: `8NRB2qCeHS7NFUQBi5zUXqCWQgfmuSrsTY8eDke34aTq`

Go to **https://faucet.solana.com**, paste that address, select "Devnet", and request 2 SOL. Do this before running the project.

You can verify your balance with:
```bash
solana balance --url devnet
```

---

## Step 2: Verify your `.env` file

Open `/home/diamondayo/work/hackathon/advanced-infrastructure-challenge/.env` and confirm it looks like this:

```env
RPC_URL="https://api.devnet.solana.com"
GRPC_URL="fra.grpc.solinfra.dev:443"
GRPC_X_TOKEN="<your SolInfra gRPC token>"
JITO_BLOCK_ENGINE_URL="https://mainnet.block-engine.jito.wtf"
NETWORK=devnet
DEEPSEEK_API_KEY=<your DeepSeek API key>
DEEPSEEK_MODEL=deepseek-chat
KEYPAIR_PATH=~/.config/solana/id.json
```

The `NETWORK=devnet` line is what tells the program to use devnet mode (regular RPC instead of Jito).

---

## Step 3: Build the project

```bash
cd /home/diamondayo/work/hackathon/advanced-infrastructure-challenge/txsentinel
cargo build 2>&1 | tail -5
```

You should see: `Finished dev profile [unoptimized + debuginfo] target(s) in ...`

If you see errors, check the error message — most likely a missing env var or network issue.

---

## Step 4: Run the TUI

```bash
cd /home/diamondayo/work/hackathon/advanced-infrastructure-challenge/txsentinel
cargo run
```

The terminal will switch to a full-screen dashboard. You should see:

**SLOT STREAM panel (top left)**
- The slot number will start at 0 and begin updating once the gRPC connection to SolInfra establishes (takes 2-5 seconds)
- TPS will show an estimated value once slots are flowing

**TIP MARKET panel (top middle)**
- Shows p25/p50/p75/p95/p99 tip percentiles in lamports
- These are fetched from the Jito mainnet tip oracle (still works even on devnet — it's just showing you the current fee market data)
- If the values are all 0, wait 10 seconds for the first fetch

**LEADER WINDOW panel (top right)**
- On devnet shows "Mode: RPC Fallback" and "READY (devnet)"
- This would show Jito leader timing on mainnet

**Status bar (bottom)**
- Shows the current operation: "Starting TxSentinel...", then your wallet balance

---

## Step 5: Test a normal submission (`[s]` key)

Press `s` on your keyboard.

What happens step by step:
1. The status bar shows "AI agent deciding priority fee..."
2. TxSentinel calls DeepSeek R1 with the current tip percentiles, TPS, and network health. This takes 3-15 seconds depending on DeepSeek API latency.
3. The AI REASONING panel fills with DeepSeek's chain-of-thought (you can see it reasoning about congestion levels and which percentile to use)
4. The last line in the reasoning panel shows: `-> Fee: 12000L  [p75]  (baseline: 5000L)` (numbers will vary)
5. Status shows "Submitting via RPC -> [signature]..."
6. The ACTIVE BUNDLES table gains a new row showing the bundle in SUBMITTED state
7. The system polls for confirmation every 800ms
8. The stage column updates: SUBMITTED → PROCESSED → CONFIRMED → FINALIZED
9. When finalized, the "Bundles" counter in the slot panel increments

**What to look for in the lifecycle log:**
- The `->Proc`, `->Conf`, `->Fin` columns show the timing in milliseconds between each stage
- Typical devnet timings: SUBMITTED→PROCESSED ~400ms, PROCESSED→CONFIRMED ~800ms, CONFIRMED→FINALIZED ~4000ms

**If the submission fails:**
- Usually means insufficient SOL — go to step 1 and get devnet SOL
- Could also be a transient RPC error — wait 5 seconds and try again

---

## Step 6: Submit 3-4 more normal bundles

Press `s` three more times (wait for each one to finish or at least reach CONFIRMED before the next). This builds up a history in the lifecycle log that shows meaningful data.

After 4-5 submissions you'll see:
- Multiple rows in the LIFECYCLE LOG panel
- Consistent timing patterns in the Δ columns
- The AI reasoning adapting its tip recommendation based on recent landing rates

---

## Step 7: Test fault injection (`[f]` then `[s]`)

This tests the AI retry path — it deliberately breaks a transaction to show the system can detect and recover from failures.

**How it works:**
1. Press `f` — the status bar shows "Fault injected — next [s]ubmit will use stale blockhash"
   - Internally, the program captures the *current* blockhash and holds onto it
   - On Solana, a blockhash is only valid for ~150 slots (about 60-90 seconds)
   - By using this "stale" blockhash for the next submission, the transaction will be rejected with a `BlockhashNotFound` error

2. Wait a few seconds (or not — even a recent blockhash being "stale" to the intent of the fault)

3. Press `s` — the status shows "Fault injected — building bundle with stale blockhash..."

4. The submission fails. You'll see in the status bar: `Failed: BlockhashExpired` (or similar)

5. The AI REASONING panel shows DeepSeek's analysis of the failure:
   - It identifies this as a blockhash expiry
   - It decides whether to retry immediately or wait
   - It may recommend bumping the fee

6. The AI decides to retry with a fresh blockhash — you'll see "Retrying with fresh blockhash..."

7. The retry submission goes through normally and lands on-chain

**In the LIFECYCLE LOG:**
- The failed entry shows `FAILED` in red
- The `Fault` column shows `BlockhashExpiry`
- The successful retry shows as a new row with no fault

---

## Step 8: Generate 10+ lifecycle log entries

For a compelling demo, aim for at least 10 entries in the lifecycle log (mix of successful and a couple of fault-injection failures):

1. Submit 4 normal bundles with `s`
2. Inject a fault and retry: `f`, then `s`
3. Submit 3 more normal bundles: `s`, `s`, `s`
4. Inject another fault: `f`, then `s`
5. Submit 2 more: `s`, `s`

This gives you 10+ entries in the log with a variety of stages and timing data. The counterfactual ledger columns (AI Tip vs Baseline) will show the AI varying its recommendation based on conditions.

---

## Step 9: Read the SQLite database (optional, shows persistence)

The lifecycle log is persisted to `txsentinel.db` in the project directory. To inspect it from a separate terminal (while TxSentinel is running):

```bash
cd /home/diamondayo/work/hackathon/advanced-infrastructure-challenge/txsentinel
sqlite3 txsentinel.db "SELECT signature, stage, tip_lamports, ai_tip_decision, baseline_tip, processed_to_confirmed_ms, injected_fault FROM bundle_entries ORDER BY submitted_at DESC LIMIT 10;" 2>/dev/null || \
sqlite3 txsentinel.db ".tables"
```

This shows the counterfactual ledger — you can see every AI tip decision alongside the p50 baseline it was compared against.

---

## Step 10: Quit

Press `q` to exit the TUI cleanly. The terminal returns to normal.

---

## Recording the Demo Video

Yes, you should record a video for the submission. Judges watch videos before reading code. A 2-3 minute video showing the live dashboard is more compelling than screenshots.

### What to record

**Section 1 (30 seconds): Show the dashboard loading**
- Run `cargo run` and narrate: "This is TxSentinel — a smart transaction stack built in Rust. You can see the slot stream connecting to Solana via Yellowstone gRPC on the left, the live Jito tip market in the middle, and the leader window on the right."

**Section 2 (60 seconds): First submission**
- Press `s`, narrate: "I'm pressing s to submit a bundle. TxSentinel is now calling the DeepSeek R1 AI to decide the optimal priority fee based on current network conditions."
- While DeepSeek responds: "Watch the AI REASONING panel — this is the actual chain-of-thought from DeepSeek R1, evaluating the congestion level and tip percentiles."
- When the decision appears: "The AI chose p75 — 12,000 lamports — because TPS is elevated. The p50 baseline would be 5,000 lamports. That difference is the counterfactual ledger in action."
- When it finalizes: "Transaction finalized in 5.2 seconds. The lifecycle log shows the exact millisecond timing at each confirmation stage."

**Section 3 (30 seconds): Fault injection**
- Press `f`: "Now I'm injecting a fault — deliberately using a stale blockhash."
- Press `s`: "Submitting with the stale hash..."
- When it fails: "BlockhashExpired — expected. Now watch the AI diagnose this and retry."
- When it retries successfully: "The AI identified the failure, fetched a fresh blockhash, and resubmitted. All of this is autonomous."

**Section 4 (20 seconds): Show the lifecycle log**
- Scroll through the entries if possible, narrate: "The lifecycle log persists everything to SQLite. Every bundle has its full timing data, AI decision, and the p50 baseline for comparison."

### How to record

**Option A — OBS Studio (recommended):**
```bash
# Install if needed
sudo apt install obs-studio
# Open OBS, add a "Window Capture" source pointing to your terminal
# Start recording before running cargo run
```

**Option B — built-in screen record (Linux):**
```bash
# Using ffmpeg to record screen
ffmpeg -video_size 1920x1080 -framerate 30 -f x11grab -i :0.0 demo.mp4
# Press Ctrl+C to stop recording
```

**Option C — WSL with Windows:**
- Use Windows Game Bar: Win+G → click record
- Or Xbox Game Bar: Win+Alt+R to start/stop recording
- The recording will be in `~/Videos/Captures/`

### Video tips
- Use a terminal with a dark theme (the TUI looks best on dark background)
- Increase font size so the panels are readable in the recording
- Record at 1080p minimum
- Keep narration calm and technical — judges are engineers
- Upload to YouTube (unlisted is fine) or Loom and include the link in your Superteam submission

---

## Troubleshooting

**"Missing env var: RPC_URL"**
→ Make sure `.env` is in the `txsentinel/` directory (not the parent). Copy it: `cp ../.env .env`

**Slot numbers stuck at 0**
→ The gRPC connection to SolInfra may be failing. Check your `GRPC_X_TOKEN`. The program will keep retrying every 2 seconds — you may see an error in the status bar. The rest of the app still works (tip oracle and submissions don't need gRPC).

**"Failed to read keypair"**
→ Run `solana-keygen new` to create a keypair at the default path, or update `KEYPAIR_PATH` in `.env`.

**Transaction fails with "insufficient funds"**
→ Get devnet SOL: visit `https://faucet.solana.com` and airdrop 2 SOL to `8NRB2qCeHS7NFUQBi5zUXqCWQgfmuSrsTY8eDke34aTq`

**DeepSeek API taking too long (> 30 seconds)**
→ This is normal for the `deepseek-reasoner` model's chain-of-thought. If it times out, the submission will fail. You can try again — the AI call typically completes in 5-15 seconds.

**Terminal looks corrupted after quit**
→ Run `reset` in the terminal to restore normal display.

---

## What the Judges Are Looking For

Based on the bounty requirements:

1. **Real Yellowstone gRPC integration** — shown by the live slot numbers updating in real time
2. **Jito bundle construction with dynamic tips** — shown by the bundle builder code and the TIP MARKET panel displaying live percentiles
3. **AI agent making real decisions** — shown by the DeepSeek reasoning panel with actual chain-of-thought, and tip values that vary based on network conditions
4. **Full lifecycle tracking** — shown by the LIFECYCLE LOG with timestamps at each stage
5. **Failure handling and retry** — shown by the fault injection demo
6. **Counterfactual ledger** — shown by the AI Tip vs Baseline columns in the tables

The key thing to emphasize: **the AI is not decorative**. It reads real on-chain data (tip percentiles, TPS, landing rates) and produces decisions that demonstrably differ from a naive p50 strategy.
