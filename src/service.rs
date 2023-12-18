use std::{sync::Arc, path::Path};

use anyhow::Result;
use thiserror::Error;
use xelis_common::{
    api::{DataValue, DataElement},
    network::Network,
    crypto::address::Address,
    utils::format_xelis
};
use xelis_wallet::{wallet::Wallet, storage::EncryptedStorage};

const BALANCES_TREE: &str = "balances";

#[derive(Debug, Error)]
pub enum ServiceError {
    #[error("Cannot transfer 0 XEL")]
    Zero,
    #[error("You can't tip yourself")]
    SelfTip,
    #[error("Not enough funds to transfer {} XEL", format_xelis(*.0))]
    NotEnoughFunds(u64),
    #[error(transparent)]
    Any(#[from] anyhow::Error)
}

pub struct WalletService {
    wallet: Arc<Wallet>
}

impl WalletService {
    pub fn new(name: String, password: String, network: Network) -> Result<Self> {
        let wallet = if Path::new(&name).is_dir() {
            Wallet::open(name, password, network)?
        } else {
            Wallet::create(name, password, None, network)?
        };

        // TODO: start a tokio task to detect incoming txs

        Ok(Self {
            wallet
        })
    }

    fn get_balance_internal(&self, storage: &EncryptedStorage, user_id: String) -> u64 {
        let balance = match storage.get_custom_data(BALANCES_TREE, &DataValue::String(user_id)) {
            Ok(balance) => balance,
            Err(_) => return 0
        };
        let balance = balance.to_value().map(|v| v.to_u64().unwrap_or(0)).unwrap_or(0);
        balance
    }

    // Get the balance for a user based on its id
    pub async fn get_balance_for_user(&self, user_id: String) -> u64 {
        let storage = self.wallet.get_storage().read().await;
        self.get_balance_internal(&storage, user_id)
    }

    // Generate a deposit address for a user based on its id
    pub fn get_address_for_user(&self, user_id: String) -> Address {
        self.wallet.get_address_with(DataElement::Value(Some(DataValue::String(user_id))))
    }

    // Transfer XEL from one user to another
    pub async fn transfer(&self, from: String, to: String, amount: u64) -> Result<(), ServiceError> {
        if amount == 0 {
            return Err(ServiceError::Zero);
        }

        if from == to {
            return Err(ServiceError::SelfTip);
        }

        let mut storage = self.wallet.get_storage().write().await;
        let from_balance = self.get_balance_internal(&storage, from.clone());
        if amount > from_balance {
            return Err(ServiceError::NotEnoughFunds(amount));
        }

        let to_balance = self.get_balance_internal(&storage, to.clone());

        // Update balances
        storage.set_custom_data(BALANCES_TREE, &DataValue::String(from), &DataElement::Value(Some(DataValue::U64(from_balance - amount))))?;
        storage.set_custom_data(BALANCES_TREE, &DataValue::String(to), &DataElement::Value(Some(DataValue::U64(to_balance + amount))))?;

        Ok(())
    }
}