use crate::prover::{EthAddress, EthEvent, EthEventParams};
use ethabi::{ParamType, Token};
use hex::ToHex;
use near_sdk::{AccountId, Balance};
use std::convert::TryInto;

/// Data that was emitted by the Ethereum TransferToNearInitiated event.
#[derive(Debug, Eq, PartialEq)]
pub struct TransferToNearInitiatedEvent {
    pub e_near_address: EthAddress,
    pub sender: String,
    pub amount: Balance,
    pub recipient: AccountId,
}

impl TransferToNearInitiatedEvent {
    fn event_params() -> EthEventParams {
        vec![
            ("sender".to_string(), ParamType::Address, true),
            ("amount".to_string(), ParamType::Uint(256), false),
            ("account_id".to_string(), ParamType::String, false),
        ]
    }

    /// Parse raw log entry data.
    pub fn from_log_entry_data(data: &[u8]) -> Self {
        let event = EthEvent::from_log_entry_data(
            "TransferToNearInitiated",
            TransferToNearInitiatedEvent::event_params(),
            data,
        );
        let sender = event.log.params[0].value.clone().to_address().unwrap().0;
        let sender = (&sender).encode_hex::<String>();
        let amount = event.log.params[1]
            .value
            .clone()
            .to_uint()
            .unwrap()
            .as_u128();
        let recipient = event.log.params[2].value.clone().to_string().unwrap();
        Self {
            e_near_address: event.locker_address,
            sender,
            amount,
            recipient: recipient.try_into().unwrap(),
        }
    }

    pub fn to_log_entry_data(&self) -> Vec<u8> {
        EthEvent::to_log_entry_data(
            "TransferToNearInitiated",
            TransferToNearInitiatedEvent::event_params(),
            self.e_near_address,
            vec![hex::decode(self.sender.clone()).unwrap()],
            vec![
                Token::Uint(self.amount.into()),
                Token::String(self.recipient.to_string()),
            ],
        )
    }
}

impl std::fmt::Display for TransferToNearInitiatedEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "sender: {}; amount: {}; recipient: {}",
            self.sender, self.amount, self.recipient
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_event_data() {
        let event_data = TransferToNearInitiatedEvent {
            e_near_address: [0u8; 20],
            sender: "00005474e89094c44da98b954eedeac495271d0f".to_string(),
            amount: 1000,
            recipient: "123".to_string().try_into().unwrap(),
        };
        let data = event_data.to_log_entry_data();
        let result = TransferToNearInitiatedEvent::from_log_entry_data(&data);
        assert_eq!(result, event_data);
    }
}
