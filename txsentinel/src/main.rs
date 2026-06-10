mod ai;
mod bundle;
mod config;
mod failure;
mod lifecycle;
mod rpc;
mod slot_monitor;
mod tui;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::Result;
use crossterm::event::{self, Event, KeyCode};
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use crossterm::ExecutableCommand;
use ratatui::prelude::*;
use solana_sdk::pubkey::Pubkey;
use solana_sdk::signature::read_keypair_file;
use solana_sdk::signer::Signer;
use std::str::FromStr;

use ai::AiAgent;
use bundle::{BundleBuilder, JitoSubmitter, TipOracle};
use config::Config;
use failure::{FailureClassifier, FaultInjector};
use lifecycle::{BundleEntry, LifecycleLog, LifecycleTracker};
use rpc::SolanaRpc;
use slot_monitor::SlotMonitor;
use tui::{app::App, ui};

#[tokio::main]
async fn main() -> Result<()> {
    // Write logs to file — stderr bleeds through the alternate-screen TUI
    let log_file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open("txsentinel.log")
        .unwrap_or_else(|_| std::fs::File::create("txsentinel.log").unwrap());
    tracing_subscriber::fmt()
        .with_env_filter("txsentinel=debug,warn")
        .with_writer(std::sync::Mutex::new(log_file))
        .with_ansi(false)
        .init();

    let cfg = Config::from_env()?;

    enable_raw_mode()?;
    let mut stdout = std::io::stdout();
    stdout.execute(EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = run(&mut terminal, cfg).await;

    disable_raw_mode()?;
    terminal.backend_mut().execute(LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    if let Err(ref e) = result {
        eprintln!("Error: {e}");
    }
    result
}

async fn run(
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    cfg: Config,
) -> Result<()> {
    let solana_rpc = SolanaRpc::new(&cfg.rpc_url);
    let log = LifecycleLog::open("txsentinel.db")?;
    let tracker = LifecycleTracker::new(log);
    let tip_oracle = TipOracle::new(&cfg.jito_url);
    let submitter = JitoSubmitter::new(&cfg.jito_url);
    let slot_monitor = Arc::new(SlotMonitor::new(&cfg.grpc_endpoint, &cfg.grpc_x_token));
    let injector: Arc<Mutex<FaultInjector>> = Arc::new(Mutex::new(FaultInjector::new()));
    let app = Arc::new(Mutex::new(App::new(cfg.network.clone(), cfg.is_devnet)));

    // Verify wallet exists and has balance
    {
        let keypair = read_keypair_file(&cfg.keypair_path)
            .map_err(|e| anyhow::anyhow!("Keypair not found at {:?}: {e}", cfg.keypair_path))?;
        let pubkey = keypair.pubkey().to_string();
        let balance = solana_rpc.get_balance(&pubkey).await.unwrap_or(0);
        app.lock().unwrap().set_status(format!(
            "Wallet: {:.6}...  Balance: {:.6} SOL",
            &pubkey[..8],
            balance as f64 / 1_000_000_000.0
        ));
    }

    // Background: slot monitor (Yellowstone gRPC)
    let monitor_clone = slot_monitor.clone();
    tokio::spawn(async move {
        if let Err(e) = monitor_clone.run().await {
            tracing::error!("Slot monitor: {e}");
        }
    });

    // Background: tip oracle refresh every 10s
    let oracle_clone = tip_oracle.clone();
    let app_tip = app.clone();
    tokio::spawn(async move {
        loop {
            match oracle_clone.refresh().await {
                Ok(p) => { app_tip.lock().unwrap().tip_percentiles = p; }
                Err(e) => tracing::warn!("Tip oracle refresh: {e}"),
            }
            tokio::time::sleep(Duration::from_secs(10)).await;
        }
    });

    // Background: sync slot state into App every 200ms.
    // Primary: gRPC slot stream. Fallback: RPC polling with derived TPS estimate.
    let monitor_state = slot_monitor.clone();
    let app_slot = app.clone();
    let rpc_slot = solana_rpc.clone();
    tokio::spawn(async move {
        let mut rpc_tick: u8 = 0;
        let mut last_rpc_slot: u64 = 0;
        let mut last_rpc_time = std::time::Instant::now();

        loop {
            let grpc_slot = monitor_state.state().current_slot;
            if grpc_slot > 0 {
                // gRPC working — use its data directly
                let state = monitor_state.state();
                let mut a = app_slot.lock().unwrap();
                a.slot_state = state.clone();
                a.slots_until_jito = 4u64.saturating_sub(state.current_slot % 4);
            } else {
                // gRPC not connected — poll RPC every ~1s and derive TPS from slot delta
                rpc_tick = rpc_tick.wrapping_add(1);
                if rpc_tick % 5 == 0 {
                    if let Ok(slot) = rpc_slot.get_slot().await {
                        let now = std::time::Instant::now();
                        let elapsed = now.duration_since(last_rpc_time).as_secs_f64();
                        let tps = if last_rpc_slot > 0 && elapsed > 0.0 {
                            let slots_delta = slot.saturating_sub(last_rpc_slot);
                            // ~2500 avg txs per slot on Solana; scale by actual elapsed time
                            ((slots_delta as f64 / elapsed) * 2_500.0) as u64
                        } else {
                            0
                        };
                        last_rpc_slot = slot;
                        last_rpc_time = now;
                        let mut a = app_slot.lock().unwrap();
                        a.slot_state.current_slot = slot;
                        a.slot_state.current_tps = tps;
                        a.slots_until_jito = 4u64.saturating_sub(slot % 4);
                    }
                }
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    });

    // Background: sync lifecycle log into App every 500ms
    let tracker_log = tracker.clone();
    let app_log = app.clone();
    tokio::spawn(async move {
        loop {
            if let Ok(entries) = tracker_log.recent_from_log() {
                let mut a = app_log.lock().unwrap();
                a.active_bundles = tracker_log.all_active();
                a.recent_log = entries;
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    });

    // Main TUI event loop
    loop {
        {
            let a = app.lock().unwrap();
            terminal.draw(|f| ui::draw(f, &a))?;
            if a.should_quit {
                break;
            }
        }

        if event::poll(Duration::from_millis(100))? {
            if let Event::Key(key) = event::read()? {
                match key.code {
                    KeyCode::Char('q') | KeyCode::Char('Q') => {
                        app.lock().unwrap().should_quit = true;
                    }
                    KeyCode::Char('s') | KeyCode::Char('S') => {
                        app.lock().unwrap().clear_ai_lines();
                        let app_c = app.clone();
                        let oracle_c = tip_oracle.clone();
                        let sub_c = submitter.clone();
                        let rpc_c = solana_rpc.clone();
                        let tracker_c = tracker.clone();
                        let inj_c = injector.clone();
                        let api_key = cfg.deepseek_api_key.clone();
                        let model = cfg.deepseek_model.clone();
                        let kp_path = cfg.keypair_path.clone();
                        let is_devnet = cfg.is_devnet;

                        tokio::spawn(async move {
                            let agent = AiAgent::new(&api_key, &model);
                            if let Err(e) = submit_bundle(
                                app_c, oracle_c, sub_c, rpc_c, tracker_c, agent, inj_c, kp_path, is_devnet,
                            )
                            .await
                            {
                                tracing::error!("submit_bundle: {e}");
                            }
                        });
                    }
                    KeyCode::Char('k') | KeyCode::Char('K') => {
                        app.lock().unwrap().ai_scroll_up();
                    }
                    KeyCode::Char('j') | KeyCode::Char('J') => {
                        app.lock().unwrap().ai_scroll_down();
                    }
                    KeyCode::Char('f') | KeyCode::Char('F') => {
                        // Inject blockhash expiry fault — fetch current bh then it will be "stale" after 150 slots
                        match solana_rpc.get_latest_blockhash().await {
                            Ok(bh) => {
                                injector.lock().unwrap().inject_blockhash_expiry(bh);
                                app.lock().unwrap().set_status(
                                    "⚠  Fault injected — next [s]ubmit will use stale blockhash",
                                );
                            }
                            Err(e) => {
                                app.lock().unwrap().set_status(format!("Failed to get blockhash: {e}"));
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    Ok(())
}

async fn submit_bundle(
    app: Arc<Mutex<App>>,
    oracle: TipOracle,
    submitter: JitoSubmitter,
    rpc: SolanaRpc,
    tracker: LifecycleTracker,
    agent: AiAgent,
    injector: Arc<Mutex<FaultInjector>>,
    keypair_path: std::path::PathBuf,
    is_devnet: bool,
) -> Result<()> {
    let keypair = read_keypair_file(&keypair_path)
        .map_err(|e| anyhow::anyhow!("Failed to read keypair: {e}"))?;
    let payer_pubkey = keypair.pubkey();
    let builder = BundleBuilder::new(keypair);

    let current_bh = rpc.get_latest_blockhash().await?;
    let percentiles = oracle.cached();
    let baseline_tip = oracle.baseline_tip();

    let (tps, current_slot, slots_until_jito) = {
        let a = app.lock().unwrap();
        (a.slot_state.current_tps, a.slot_state.current_slot, a.slots_until_jito)
    };

    let avg_delta = {
        let a = app.lock().unwrap();
        a.recent_log
            .iter()
            .filter_map(|e| e.processed_to_confirmed_ms)
            .take(5)
            .reduce(|a, b| (a + b) / 2)
    };

    let landing_rate = {
        let a = app.lock().unwrap();
        let total = a.recent_log.len() as f64;
        if total == 0.0 {
            1.0
        } else {
            let landed = a.recent_log.iter().filter(|e| {
                matches!(e.stage, lifecycle::CommitmentStage::Finalized)
            }).count() as f64;
            landed / total
        }
    };

    app.lock().unwrap().set_status("AI agent deciding priority fee...");

    let decision = agent
        .decide_tip(&percentiles, tps, slots_until_jito, 4, current_slot, avg_delta, landing_rate)
        .await?;

    {
        let mut a = app.lock().unwrap();
        a.ai_summary = decision.summary.clone();
        for line in decision.reasoning.lines().take(5) {
            if !line.trim().is_empty() {
                a.push_ai_line(line.to_string());
            }
        }
        a.push_ai_line(format!(
            "-> Fee: {}L  [{}]  (baseline: {}L)",
            decision.tip_lamports, decision.percentile_used, baseline_tip
        ));
    }

    // Fault injection check
    let stale = injector.lock().unwrap().consume();
    let injected_fault = if stale.is_some() { Some("BlockhashExpiry".to_string()) } else { None };

    if is_devnet {
        // ── DEVNET PATH: submit via regular RPC sendTransaction ──────────────
        let stale_hash_used = stale.is_some();
        let bundle = if let Some(stale_hash) = stale {
            app.lock().unwrap().set_status("Fault injected — building bundle with stale blockhash...");
            builder.build_with_stale_blockhash(stale_hash, decision.tip_lamports, &payer_pubkey)?
        } else {
            builder.build_self_transfer(current_bh, decision.tip_lamports, &payer_pubkey, decision.tip_lamports.min(1_000_000))?
        };

        let main_tx = &bundle.transactions[0];
        let sig_preview = main_tx.signatures.first().map(|s| s.to_string())
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

        let mut entry = BundleEntry::new(sig_preview.clone(), decision.tip_lamports, current_slot);
        entry.ai_reasoning = Some(decision.reasoning.clone());
        entry.ai_tip_decision = Some(decision.tip_lamports);
        entry.baseline_tip = Some(baseline_tip);
        entry.injected_fault = injected_fault.clone();
        tracker.register(entry)?;

        app.lock().unwrap().set_status(format!("Submitting via RPC -> {}...", &sig_preview[..12]));

        match rpc.send_transaction(main_tx).await {
            Ok(signature) => {
                app.lock().unwrap().set_status(format!("Accepted -> {}", &signature[..16.min(signature.len())]));
                poll_rpc_status(&rpc, &signature, &tracker, app.clone()).await?;
            }
            Err(e) => {
                let err_str = e.to_string();
                let kind = FailureClassifier::classify_str(&err_str);
                tracker.mark_failed(&sig_preview, &err_str)?;
                app.lock().unwrap().set_status(format!("Failed: {}", kind.label()));

                if stale_hash_used {
                    app.lock().unwrap().set_status("AI reasoning about blockhash failure...");
                    let retry = agent
                        .decide_retry(&kind, 0, decision.tip_lamports, 0, &percentiles, tps, slots_until_jito, 4, current_slot)
                        .await?;
                    {
                        let mut a = app.lock().unwrap();
                        a.push_ai_line(format!("  {}", retry.failure_diagnosis));
                        for line in retry.reasoning.lines().take(3) {
                            if !line.trim().is_empty() {
                                a.push_ai_line(line.to_string());
                            }
                        }
                        a.push_ai_line(format!("-> Retry: {}  new fee: {}L", retry.summary, retry.new_tip_lamports));
                    }
                    if retry.should_retry {
                        if retry.wait_slots > 0 {
                            tokio::time::sleep(Duration::from_millis(retry.wait_slots * 400)).await;
                        }
                        app.lock().unwrap().set_status("Retrying with fresh blockhash...");
                        Box::pin(submit_bundle(app, oracle, submitter, rpc, tracker, agent, injector, keypair_path, is_devnet)).await?;
                    }
                }
            }
        }
    } else {
        // ── MAINNET PATH: submit via Jito bundle ─────────────────────────────
        let tip_accounts = submitter.get_tip_accounts().await?;
        let tip_account = Pubkey::from_str(
            tip_accounts.first().ok_or_else(|| anyhow::anyhow!("No tip accounts"))?,
        )?;

        let bundle = if let Some(stale_hash) = stale {
            app.lock().unwrap().set_status("Submitting with stale blockhash (fault)...");
            builder.build_with_stale_blockhash(stale_hash, decision.tip_lamports, &tip_account)?
        } else {
            builder.build_self_transfer(current_bh, decision.tip_lamports, &tip_account, 1000)?
        };

        let signature = bundle.transactions[0]
            .signatures
            .first()
            .map(|s| s.to_string())
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

        let mut entry = BundleEntry::new(signature.clone(), decision.tip_lamports, current_slot);
        entry.ai_reasoning = Some(decision.reasoning.clone());
        entry.ai_tip_decision = Some(decision.tip_lamports);
        entry.baseline_tip = Some(baseline_tip);
        entry.injected_fault = injected_fault.clone();
        tracker.register(entry)?;

        app.lock().unwrap().set_status(format!("Submitting bundle -> {}...", &signature[..12]));

        match submitter.send_bundle(&bundle).await {
            Ok(bundle_id) => {
                app.lock().unwrap().set_status(format!("Submitted — bundle: {bundle_id}"));
                poll_bundle_status(&submitter, &bundle_id, &signature, &tracker, app.clone()).await?;
            }
            Err(e) => {
                let err_str = e.to_string();
                let kind = FailureClassifier::classify_str(&err_str);
                tracker.mark_failed(&signature, &err_str)?;
                app.lock().unwrap().set_status(format!("Failed: {}", kind.label()));

                if injected_fault.is_some() {
                    app.lock().unwrap().set_status("AI reasoning about failure...");
                    let retry = agent
                        .decide_retry(&kind, 0, decision.tip_lamports, 0, &percentiles, tps, slots_until_jito, 4, current_slot)
                        .await?;
                    {
                        let mut a = app.lock().unwrap();
                        a.push_ai_line(format!("  {}", retry.failure_diagnosis));
                        for line in retry.reasoning.lines().take(3) {
                            if !line.trim().is_empty() {
                                a.push_ai_line(line.to_string());
                            }
                        }
                        a.push_ai_line(format!("-> Retry: {}  new tip: {}L", retry.summary, retry.new_tip_lamports));
                    }
                    if retry.should_retry {
                        if retry.wait_slots > 0 {
                            tokio::time::sleep(Duration::from_millis(retry.wait_slots * 400)).await;
                        }
                        app.lock().unwrap().set_status("Retrying with fresh blockhash (AI)...");
                        Box::pin(submit_bundle(app, oracle, submitter, rpc, tracker, agent, injector, keypair_path, is_devnet)).await?;
                    }
                }
            }
        }
    }

    Ok(())
}

async fn poll_rpc_status(
    rpc: &SolanaRpc,
    signature: &str,
    tracker: &LifecycleTracker,
    app: Arc<Mutex<App>>,
) -> Result<()> {
    let slot = { app.lock().unwrap().slot_state.current_slot };
    for i in 0..40u32 {
        tokio::time::sleep(Duration::from_millis(800)).await;

        match rpc.get_transaction_status(signature).await {
            Ok(Some(status)) if status.starts_with("failed:") => {
                let msg = status.trim_start_matches("failed:");
                let kind = FailureClassifier::classify_str(msg);
                tracker.mark_failed(signature, msg)?;
                app.lock().unwrap().set_status(format!("Failed on-chain: {}", kind.label()));
                return Ok(());
            }
            Ok(Some(status)) => {
                match status.as_str() {
                    "processed" => {
                        if i == 0 { tracker.advance_processed(signature, slot)?; }
                        app.lock().unwrap().set_status(format!("Processed -> {}", &signature[..16.min(signature.len())]));
                    }
                    "confirmed" => {
                        let _ = tracker.advance_processed(signature, slot);
                        tracker.advance_confirmed(signature, slot)?;
                        app.lock().unwrap().set_status(format!("Confirmed -> {}", &signature[..16.min(signature.len())]));
                    }
                    "finalized" => {
                        let _ = tracker.advance_processed(signature, slot);
                        let _ = tracker.advance_confirmed(signature, slot);
                        tracker.advance_finalized(signature, slot)?;
                        let mut a = app.lock().unwrap();
                        a.submission_count += 1;
                        a.set_status(format!("Finalized -> {}", &signature[..16.min(signature.len())]));
                        return Ok(());
                    }
                    _ => {}
                }
            }
            Ok(None) => {
                app.lock().unwrap().set_status(format!("Pending... ({}/40)", i + 1));
            }
            Err(e) => {
                tracing::warn!("poll_rpc_status error: {e}");
            }
        }
    }

    tracker.mark_failed(signature, "timeout after 40 polls")?;
    app.lock().unwrap().set_status("Timed out waiting for confirmation");
    Ok(())
}

async fn poll_bundle_status(
    submitter: &JitoSubmitter,
    bundle_id: &str,
    signature: &str,
    tracker: &LifecycleTracker,
    app: Arc<Mutex<App>>,
) -> Result<()> {
    for _ in 0..30u32 {
        tokio::time::sleep(Duration::from_secs(2)).await;

        if let Ok(Some(status)) = submitter.get_bundle_status(bundle_id).await {
            match status.status.as_str() {
                "processed" => {
                    tracker.advance_processed(signature, status.landed_slot.unwrap_or(0))?;
                }
                "confirmed" => {
                    tracker.advance_processed(signature, status.landed_slot.unwrap_or(0))?;
                    tracker.advance_confirmed(signature, status.landed_slot.unwrap_or(0))?;
                    app.lock().unwrap().set_status(format!("✓ Confirmed at slot {:?}", status.landed_slot));
                }
                "finalized" => {
                    tracker.advance_processed(signature, status.landed_slot.unwrap_or(0))?;
                    tracker.advance_confirmed(signature, status.landed_slot.unwrap_or(0))?;
                    tracker.advance_finalized(signature, status.landed_slot.unwrap_or(0))?;
                    let mut a = app.lock().unwrap();
                    a.submission_count += 1;
                    a.set_status(format!("✅ Finalized at slot {:?}", status.landed_slot));
                    return Ok(());
                }
                _ => {
                    if let Some(err) = &status.err {
                        let kind = FailureClassifier::classify(err);
                        tracker.mark_failed(signature, &err.to_string())?;
                        app.lock().unwrap().set_status(format!("✗ {}", kind.label()));
                        return Ok(());
                    }
                }
            }
        }
    }

    tracker.mark_failed(signature, "timeout")?;
    app.lock().unwrap().set_status("✗ Bundle timed out");
    Ok(())
}
