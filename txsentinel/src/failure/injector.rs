use solana_sdk::hash::Hash;

/// Controls deliberate fault injection for demo purposes
pub struct FaultInjector {
    pub next_should_expire: bool,
    pub stale_blockhash: Option<Hash>,
}

impl FaultInjector {
    pub fn new() -> Self {
        Self {
            next_should_expire: false,
            stale_blockhash: None,
        }
    }

    /// Queue a blockhash expiry fault for the next submission.
    /// Uses a fixed invalid hash — guaranteed to fail regardless of when [s] is pressed.
    pub fn inject_blockhash_expiry(&mut self, _current_hash: Hash) {
        self.next_should_expire = true;
        // All-0x01 bytes: not a real blockhash, will always fail preflight validation
        self.stale_blockhash = Some(Hash::new_from_array([1u8; 32]));
    }

    /// Consume the fault — returns the stale hash if a fault is queued
    pub fn consume(&mut self) -> Option<Hash> {
        if self.next_should_expire {
            self.next_should_expire = false;
            self.stale_blockhash.take()
        } else {
            None
        }
    }
}
