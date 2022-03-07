use near_plugins::{only, pause, FullAccessKeyFallback, Ownable, Pausable, Upgradable};
/**
* Bridge for Near Native token
*/
use near_sdk::borsh::{self, BorshDeserialize, BorshSerialize};
use near_sdk::collections::UnorderedSet;
use near_sdk::{env, ext_contract, near_bindgen, AccountId, Balance, Gas, PanicOnDefault, Promise};
use prover::ext_prover;
pub use prover::{get_eth_address, is_valid_eth_address, EthAddress, Proof};
pub use transfer_to_near_event::TransferToNearInitiatedEvent;

pub mod prover;
mod transfer_to_near_event;

/// Gas to call finalise method.
const FINISH_FINALISE_GAS: Gas = Gas(50 * Gas::ONE_TERA.0);

const NO_DEPOSIT: Balance = 0;

/// Gas to call verify_log_entry on prover.
const VERIFY_LOG_ENTRY_GAS: Gas = Gas(50 * Gas::ONE_TERA.0);

#[derive(Debug, Eq, PartialEq, BorshSerialize, BorshDeserialize)]
pub enum ResultType {
    MigrateNearToEthereum {
        amount: Balance,
        recipient: EthAddress,
    },
}

#[near_bindgen]
#[derive(
    BorshDeserialize,
    BorshSerialize,
    PanicOnDefault,
    Ownable,
    Pausable,
    FullAccessKeyFallback,
    Upgradable,
)]
pub struct NearBridge {
    /// The account of the prover that we can use to prove
    pub prover_account: AccountId,

    /// Address of the associated Ethereum eNear ERC20 contract.
    pub e_near_address: EthAddress,

    /// Hashes of the events that were already used.
    pub used_events: UnorderedSet<Vec<u8>>,

    // Deprecated field. Requires migration to be removed.
    __paused: u128,
}

#[near_bindgen]
impl NearBridge {
    #[init]
    pub fn new(prover_account: AccountId, e_near_address: String) -> Self {
        Self {
            prover_account,
            e_near_address: get_eth_address(e_near_address),
            used_events: UnorderedSet::new(b"u".to_vec()),
            __paused: Default::default(),
        }
    }

    /// Deposit NEAR for bridging from the predecessor account ID
    /// Requirements:
    /// * `eth_recipient` must be a valid eth account
    /// * `amount` must be a positive integer
    /// * Caller of the method has to attach deposit enough to cover:
    ///   * The `amount` of Near tokens being bridged, and
    ///   * The storage difference at the fixed storage price defined in the contract.
    #[pause]
    #[payable]
    #[result_serializer(borsh)]
    // TODO: how much GAS is required to execute this method with sending the tokens back and ensure we have enough
    pub fn migrate_to_ethereum(&mut self, eth_recipient: String) -> ResultType {
        // Predecessor must attach Near to migrate to ETH
        let attached_deposit = env::attached_deposit();
        assert_ne!(
            attached_deposit, 0,
            "Attached deposit must be greater than zero"
        );

        // If the method is paused or the eth recipient address is invalid, then we need to:
        //  1) Return the attached deposit
        //  2) Panic and tell the user why
        let eth_recipient_clone = eth_recipient.clone();
        assert!(
            is_valid_eth_address(eth_recipient_clone),
            "ETH address is invalid"
        );

        ResultType::MigrateNearToEthereum {
            amount: attached_deposit,
            recipient: get_eth_address(eth_recipient),
        }
    }

    #[pause]
    #[payable]
    pub fn finalise_eth_to_near_transfer(&mut self, #[serializer(borsh)] proof: Proof) {
        let event = TransferToNearInitiatedEvent::from_log_entry_data(&proof.log_entry_data);
        assert_eq!(
            event.e_near_address,
            self.e_near_address,
            "Event's address {} does not match locker address of this token {}",
            hex::encode(&event.e_near_address),
            hex::encode(&self.e_near_address),
        );

        let proof_1 = proof.clone();

        ext_prover::verify_log_entry(
            proof.log_index,
            proof.log_entry_data,
            proof.receipt_index,
            proof.receipt_data,
            proof.header_data,
            proof.proof,
            false, // Do not skip bridge call. This is only used for development and diagnostics.
            self.prover_account.clone(),
            NO_DEPOSIT,
            VERIFY_LOG_ENTRY_GAS,
        )
        .then(ext_self::finish_eth_to_near_transfer(
            event.recipient,
            event.amount,
            proof_1,
            env::current_account_id(),
            env::attached_deposit(),
            FINISH_FINALISE_GAS,
        ));
    }

    /// Finish depositing once the proof was successfully validated. Can only be called by the contract
    /// itself.
    #[only(self)]
    #[payable]
    pub fn finish_eth_to_near_transfer(
        &mut self,
        #[callback]
        #[serializer(borsh)]
        verification_success: bool,
        #[serializer(borsh)] new_owner_id: AccountId,
        #[serializer(borsh)] amount: Balance,
        #[serializer(borsh)] proof: Proof,
    ) {
        assert!(verification_success, "Failed to verify the proof");

        let required_deposit = self.record_proof(&proof);

        assert!(
            env::attached_deposit() >= required_deposit,
            "Attached deposit is not sufficient to record proof"
        );

        Promise::new(new_owner_id).transfer(amount);
    }

    /// Checks whether the provided proof is already used
    pub fn is_used_proof(&self, #[serializer(borsh)] proof: Proof) -> bool {
        self.used_events.contains(&proof.get_key())
    }

    /// Record proof to make sure it is not re-used later for anther deposit.
    #[only(self)]
    fn record_proof(&mut self, proof: &Proof) -> Balance {
        // TODO: Instead of sending the full proof (clone only relevant parts of the Proof)
        //       log_index / receipt_index / header_data
        let initial_storage = env::storage_usage();
        let proof_key = proof.get_key();
        assert!(
            !self.used_events.contains(&proof_key),
            "Event cannot be reused for depositing."
        );
        self.used_events.insert(&proof_key);
        let current_storage = env::storage_usage();

        let required_deposit =
            Balance::from(current_storage - initial_storage) * env::storage_byte_cost();

        required_deposit
    }
}

#[ext_contract(ext_self)]
pub trait ExtNearBridge {
    #[result_serializer(borsh)]
    fn finish_eth_to_near_transfer(
        &mut self,
        #[callback]
        #[serializer(borsh)]
        verification_success: bool,
        #[serializer(borsh)] new_owner_id: AccountId,
        #[serializer(borsh)] amount: Balance,
        #[serializer(borsh)] proof: Proof,
    ) -> Promise;
}

#[cfg(not(target_arch = "wasm32"))]
#[cfg(test)]
mod tests {
    use near_sdk::test_utils::VMContextBuilder;
    use near_sdk::{testing_env, MockedBlockchain};

    use super::*;
    use near_sdk::env::sha256;
    use std::convert::TryInto;
    use std::panic;
    use uint::rustc_hex::{FromHex, ToHex};

    const UNPAUSE_ALL: Mask = 0;

    macro_rules! inner_set_env {
        ($builder:ident) => {
            $builder
        };

        ($builder:ident, $key:ident:$value:expr $(,$key_tail:ident:$value_tail:expr)*) => {
            {
               $builder.$key($value.try_into().unwrap());
               inner_set_env!($builder $(,$key_tail:$value_tail)*)
            }
        };
    }

    macro_rules! set_env {
        ($($key:ident:$value:expr),* $(,)?) => {
            let mut builder = VMContextBuilder::new();
            let mut builder = &mut builder;
            builder = inner_set_env!(builder, $($key: $value),*);
            testing_env!(builder.build());
        };
    }

    fn alice_near_account() -> AccountId {
        "alice.near".to_string().try_into().unwrap()
    }
    fn prover_near_account() -> AccountId {
        "prover".to_string().try_into().unwrap()
    }
    fn e_near_eth_address() -> String {
        "68a3637ba6e75c0f66b61a42639c4e9fcd3d4824".to_string()
    }
    fn alice_eth_address() -> String {
        "25ac31a08eba29067ba4637788d1dbfb893cebf1".to_string()
    }
    fn invalid_eth_address() -> String {
        "25Ac31A08EBA29067Ba4637788d1DbFB893cEBf".to_string()
    }

    /// Generate a valid ethereum address
    fn ethereum_address_from_id(id: u8) -> String {
        let mut buffer = vec![id];
        sha256(buffer.as_mut())
            .into_iter()
            .take(20)
            .collect::<Vec<_>>()
            .to_hex()
    }

    fn sample_proof() -> Proof {
        Proof {
            log_index: 0,
            log_entry_data: vec![],
            receipt_index: 0,
            receipt_data: vec![],
            header_data: vec![],
            proof: vec![],
        }
    }

    fn create_proof(e_near: String) -> Proof {
        let event_data = TransferToNearInitiatedEvent {
            e_near_address: e_near
                .from_hex::<Vec<_>>()
                .unwrap()
                .as_slice()
                .try_into()
                .unwrap(),
            sender: "00005474e89094c44da98b954eedeac495271d0f".to_string(),
            amount: 1000,
            recipient: "123".to_string().try_into().unwrap(),
        };

        Proof {
            log_index: 0,
            log_entry_data: event_data.to_log_entry_data(),
            receipt_index: 0,
            receipt_data: vec![],
            header_data: vec![],
            proof: vec![],
        }
    }

    #[test]
    fn can_migrate_near_to_eth_with_valid_params() {
        set_env!(predecessor_account_id: alice_near_account());

        let mut contract = NearBridge::new(prover_near_account(), e_near_eth_address());

        // lets deposit 1 Near
        let deposit_amount = 1_000_000_000_000_000_000_000_000u128;
        set_env!(
            predecessor_account_id: alice_near_account(),
            attached_deposit: deposit_amount,
        );

        contract.migrate_to_ethereum(alice_eth_address());
    }

    #[test]
    #[should_panic]
    fn migrate_near_to_eth_panics_when_attached_deposit_is_zero() {
        set_env!(predecessor_account_id: alice_near_account());

        let mut contract = NearBridge::new(prover_near_account(), e_near_eth_address());

        contract.migrate_to_ethereum(alice_eth_address());
    }

    #[test]
    #[should_panic]
    fn migrate_near_to_eth_panics_when_contract_is_paused() {
        set_env!(predecessor_account_id: alice_near_account());

        let mut contract = NearBridge::new(prover_near_account(), e_near_eth_address());

        contract.set_paused(PAUSE_MIGRATE_TO_ETH);

        // lets deposit 1 Near
        let deposit_amount = 1_000_000_000_000_000_000_000_000u128;
        set_env!(
            predecessor_account_id: alice_near_account(),
            attached_deposit: deposit_amount,
        );

        contract.migrate_to_ethereum(alice_eth_address());
    }

    #[test]
    #[should_panic]
    fn migrate_near_to_eth_panics_when_eth_address_is_invalid() {
        set_env!(predecessor_account_id: alice_near_account());

        let mut contract = NearBridge::new(prover_near_account(), e_near_eth_address());

        contract.migrate_to_ethereum(invalid_eth_address());
    }

    #[test]
    #[should_panic]
    fn finalise_eth_to_near_transfer_panics_when_contract_is_paused() {
        set_env!(predecessor_account_id: alice_near_account());

        let mut contract = NearBridge::new(prover_near_account(), e_near_eth_address());

        contract.set_paused(PAUSE_ETH_TO_NEAR_TRANSFER);

        contract.finalise_eth_to_near_transfer(sample_proof());
    }

    #[test]
    #[should_panic]
    fn finalise_eth_to_near_transfer_panics_when_event_originates_from_wrong_contract() {
        set_env!(predecessor_account_id: alice_near_account());

        let mut contract = NearBridge::new(prover_near_account(), e_near_eth_address());

        contract.finalise_eth_to_near_transfer(create_proof(alice_eth_address()));
    }

    #[test]
    #[should_panic]
    fn finish_eth_to_near_transfer_panics_if_attached_deposit_is_not_sufficient_to_record_proof() {
        set_env!(predecessor_account_id: alice_near_account());

        let mut contract = NearBridge::new(prover_near_account(), e_near_eth_address());

        contract.finalise_eth_to_near_transfer(create_proof(e_near_eth_address()));
    }

    #[test]
    fn finalise_eth_to_near_transfer_works_with_valid_params() {
        set_env!(predecessor_account_id: alice_near_account());

        let mut contract = NearBridge::new(prover_near_account(), e_near_eth_address());

        // Alice deposit 1 Near to migrate to eth
        let deposit_amount = 1_000_000_000_000_000_000_000_000u128;
        set_env!(
            predecessor_account_id: alice_near_account(),
            attached_deposit: deposit_amount,
        );

        contract.migrate_to_ethereum(alice_eth_address());

        // todo adjust attached deposit down

        // Lets suppose Alice migrates back
        contract.finalise_eth_to_near_transfer(create_proof(e_near_eth_address()))

        // todo asserts i.e. that alice has received the 1 near back etc.
    }
}
