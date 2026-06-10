use anyhow::{Context, Result};
use solana_sdk::{
    compute_budget::ComputeBudgetInstruction,
    hash::Hash,
    pubkey::Pubkey,
    signature::Keypair,
    signer::Signer,
    system_instruction,
    transaction::Transaction,
};
use std::str::FromStr;

/// A Jito bundle: up to 5 transactions, last one is the tip payment
pub struct Bundle {
    pub transactions: Vec<Transaction>,
    pub tip_lamports: u64,
    pub tip_account: Pubkey,
}

pub struct BundleBuilder {
    pub keypair: Keypair,
}

impl BundleBuilder {
    pub fn new(keypair: Keypair) -> Self {
        Self { keypair }
    }

    /// Build a self-transfer bundle (for testing/demo — sends 1 lamport to self)
    pub fn build_self_transfer(
        &self,
        blockhash: Hash,
        tip_lamports: u64,
        tip_account: &Pubkey,
        compute_unit_price: u64,
    ) -> Result<Bundle> {
        let payer = self.keypair.pubkey();

        // Main transaction: 1 lamport self-transfer with priority fee
        let main_tx = {
            let set_cu_price = ComputeBudgetInstruction::set_compute_unit_price(compute_unit_price);
            // 5_000 CUs: enough for 2 ComputeBudget ixs (~150 each) + 1 transfer (~450)
            let set_cu_limit = ComputeBudgetInstruction::set_compute_unit_limit(5_000);
            let transfer = system_instruction::transfer(&payer, &payer, 1);

            let mut tx = Transaction::new_with_payer(
                &[set_cu_price, set_cu_limit, transfer],
                Some(&payer),
            );
            tx.sign(&[&self.keypair], blockhash);
            tx
        };

        // Tip transaction: pay the Jito tip account
        let tip_tx = {
            let transfer = system_instruction::transfer(&payer, tip_account, tip_lamports);
            let mut tx = Transaction::new_with_payer(&[transfer], Some(&payer));
            tx.sign(&[&self.keypair], blockhash);
            tx
        };

        Ok(Bundle {
            transactions: vec![main_tx, tip_tx],
            tip_lamports,
            tip_account: *tip_account,
        })
    }

    /// Build a bundle with a deliberately stale blockhash (fault injection)
    pub fn build_with_stale_blockhash(
        &self,
        stale_hash: Hash,
        tip_lamports: u64,
        tip_account: &Pubkey,
    ) -> Result<Bundle> {
        let payer = self.keypair.pubkey();
        let transfer = system_instruction::transfer(&payer, &payer, 1);
        let mut tx = Transaction::new_with_payer(&[transfer], Some(&payer));
        tx.sign(&[&self.keypair], stale_hash);

        let tip_transfer = system_instruction::transfer(&payer, tip_account, tip_lamports);
        let mut tip_tx = Transaction::new_with_payer(&[tip_transfer], Some(&payer));
        tip_tx.sign(&[&self.keypair], stale_hash);

        Ok(Bundle {
            transactions: vec![tx, tip_tx],
            tip_lamports,
            tip_account: *tip_account,
        })
    }

    pub fn encode_transactions(bundle: &Bundle) -> Result<Vec<String>> {
        bundle
            .transactions
            .iter()
            .map(|tx| {
                let encoded = bincode::serialize(tx).context("serialize tx")?;
                Ok(base64::Engine::encode(
                    &base64::engine::general_purpose::STANDARD,
                    &encoded,
                ))
            })
            .collect()
    }
}
