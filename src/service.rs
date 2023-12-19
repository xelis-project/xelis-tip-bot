use std::{sync::Arc, path::Path};

use anyhow::Result;
use poise::serenity_prelude::User;
use thiserror::Error;
use xelis_common::{
    crypto::hash::{Hash, Hashable},
    api::{DataValue, DataElement},
    network::Network,
    crypto::address::Address,
    utils::format_xelis, config::XELIS_ASSET
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
    #[error("Not enough funds to pay {} XEL of fee", format_xelis(*.0))]
    NotEnoughFundsForFee(u64),
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

    fn get_balance_internal(&self, storage: &EncryptedStorage, user: &User) -> u64 {
        let balance = match storage.get_custom_data(BALANCES_TREE, &DataValue::U64(user.id.into())) {
            Ok(balance) => balance,
            Err(_) => return 0
        };
        let balance = balance.to_value().map(|v| v.to_u64().unwrap_or(0)).unwrap_or(0);
        balance
    }

    // Get the balance for a user based on its id
    pub async fn get_balance_for_user(&self, user: &User) -> u64 {
        let storage = self.wallet.get_storage().read().await;
        self.get_balance_internal(&storage, user)
    }

    // Generate a deposit address for a user based on its id
    pub fn get_address_for_user(&self, user: &User) -> Address {
        self.wallet.get_address_with(DataElement::Value(Some(DataValue::U64(user.id.into()))))
    }

    // Transfer XEL from one user to another
    pub async fn transfer(&self, from: &User, to: &User, amount: u64) -> Result<(), ServiceError> {
        if amount == 0 {
            return Err(ServiceError::Zero);
        }

        if from == to {
            return Err(ServiceError::SelfTip);
        }

        let mut storage = self.wallet.get_storage().write().await;
        let from_balance = self.get_balance_internal(&storage, &from);
        if amount > from_balance {
            return Err(ServiceError::NotEnoughFunds(amount));
        }

        let to_balance = self.get_balance_internal(&storage, to);

        // Update balances
        storage.set_custom_data(BALANCES_TREE, &DataValue::U64(from.id.into()), &DataElement::Value(Some(DataValue::U64(from_balance - amount))))?;
        storage.set_custom_data(BALANCES_TREE, &DataValue::U64(to.id.into()), &DataElement::Value(Some(DataValue::U64(to_balance + amount))))?;

        Ok(())
    }

    // Withdraw XEL from the service to an address
    pub async fn withdraw(&self, user: &User, to: Address, amount: u64) -> Result<Hash, ServiceError> {
        if amount == 0 {
            return Err(ServiceError::Zero);
        }

        let mut storage = self.wallet.get_storage().write().await;
        let balance = self.get_balance_internal(&storage, &user);
        if amount > balance {
            return Err(ServiceError::NotEnoughFunds(amount));
        }

        let transaction = self.wallet.send_to(&storage, XELIS_ASSET, to, amount)?;
        let fee = transaction.get_fee();
        // Verify if he has enough with fees included
        if fee + amount > balance {
            return Err(ServiceError::NotEnoughFundsForFee(fee));
        }

        match self.wallet.submit_transaction(&transaction).await {
            Ok(_) => {
                // Update balance
                storage.set_custom_data(BALANCES_TREE, &DataValue::U64(user.id.into()), &DataElement::Value(Some(DataValue::U64(balance - (fee + amount)))))?;
                Ok(transaction.hash())
            },
            Err(e) => Err(ServiceError::Any(e.into()))
        }
    }

    pub fn network(&self) -> &Network {
        self.wallet.get_network()
    }
}