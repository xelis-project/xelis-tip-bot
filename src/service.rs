use std::{sync::{Arc, atomic::{AtomicBool, Ordering}}, path::Path};

use anyhow::Result;
use poise::serenity_prelude::{User, Http, CreateMessage, CreateEmbed};
use thiserror::Error;
use xelis_common::{
    api::{
        wallet::EntryType,
        DataElement,
        DataValue
    },
    config::XELIS_ASSET,
    crypto::{
        ecdlp::NoOpProgressTableGenerationReportFunction,
        Address,
        Hash,
        Hashable
    },
    network::Network,
    serializer::{Reader, ReaderError, Serializer, Writer},
    transaction::builder::{
        FeeBuilder,
        TransactionTypeBuilder,
        TransferBuilder
    },
    utils::format_xelis
};
use xelis_wallet::{
    error::WalletError,
    storage::EncryptedStorage,
    wallet::{Event, Wallet}
};

use crate::{ICON, COLOR};

const BALANCES_TREE: &str = "balances";
const HISTORY_TREE: &str = "history";

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
    #[error("Service is already running")]
    AlreadyRunning,
    #[error(transparent)]
    Any(#[from] anyhow::Error),
    #[error(transparent)]
    WalletError(#[from] WalletError)
}

pub type WalletService = Arc<WalletServiceImpl>;

pub struct WalletServiceImpl {
    wallet: Arc<Wallet>,
    running: AtomicBool
}

pub struct Deposit {
    pub user_id: u64,
    pub amount: u64,
    pub transaction_hash: Hash
}

impl WalletServiceImpl {
    pub async fn new(name: String, password: String, daemon_address: String, network: Network) -> Result<WalletService> {
        let precomputed_tables = Wallet::read_or_generate_precomputed_tables(None, NoOpProgressTableGenerationReportFunction)?;

        let wallet = if Path::new(&name).is_dir() {
            Wallet::open(name, password, network, precomputed_tables)?
        } else {
            Wallet::create(name, password, None, network, precomputed_tables)?
        };

        wallet.set_online_mode(&daemon_address, true).await?;

        let service = Arc::new(Self {
            wallet,
            running: AtomicBool::new(false)
        });

        Ok(service)
    }

    // Start the service to scan all incoming TXs
    pub async fn start(self: WalletService, http: Arc<Http>) -> Result<(), ServiceError> {
        if self.running.swap(true, Ordering::SeqCst) {
            return Err(ServiceError::AlreadyRunning);
        }

        tokio::spawn(async move {
            if let Err(e) = self.event_loop(http).await {
                println!("Error in event loop: {:?}", e);
            }
        });
    
        Ok(())
    }

    // Notify a user of a deposit
    async fn notify_deposit(&self, http: &Http, user_id: u64, amount: u64, transaction_hash: &Hash) -> Result<()> {
        let user = http.get_user(user_id.into()).await?;
        let channel = user.create_dm_channel(&http).await?;

        let embed = CreateEmbed::default()
            .title("Deposit")
            .description(format!("You received {} XEL", format_xelis(amount)))
            .field("Transaction", transaction_hash.to_string(), false)
            .thumbnail(ICON)
            .colour(COLOR);

        channel.send_message(&http, CreateMessage::default().embed(embed)).await?;
        Ok(())
    }

    // this function is called one time at WalletService creation,
    // and is notified by the wallet of any new transaction
    async fn event_loop(self: WalletService, http: Arc<Http>) -> Result<()> {
        let mut receiver = self.wallet.subscribe_events().await;
        loop {
            let event = receiver.recv().await?;
            match event {
                Event::NewTransaction(transaction) => match transaction.entry {
                    EntryType::Incoming { from: _, transfers } => {
                        // Check if there is any transfer that is for us
                        for transfer in transfers.iter().filter(|t| t.asset == XELIS_ASSET) {
                            if let Some(data) = &transfer.extra_data {
                                if let Some(user_id) = data.as_value().and_then(|v| v.as_u64()).ok() {
                                    let amount = transfer.amount;
                                    {
                                        let mut storage = self.wallet.get_storage().write().await;
                                        // Calculate new balance
                                        let balance = self.get_balance_internal(&storage, user_id);
                                        let new_balance = balance + amount;
                                        // Update balance
                                        storage.set_custom_data(BALANCES_TREE, &user_id.into(), &new_balance.into())?;

                                        // Store the TX hash in the history
                                        storage.set_custom_data(HISTORY_TREE, &transaction.hash.clone().into(), &user_id.into())?;
                                    }

                                    // Send message to user
                                    if let Err(e) = self.notify_deposit(&http, user_id, amount, &transaction.hash).await {
                                        println!("Error while notifying user of deposit: {:?}", e);
                                    }
                                }
                            }
                        }
                    },
                    _ => {}
                }
                _ => {}
            }
        }
    }

    // Get the balance for a user based on its id
    fn get_balance_internal(&self, storage: &EncryptedStorage, user: u64) -> u64 {
        let balance = match storage.get_custom_data(BALANCES_TREE, &DataValue::U64(user)) {
            Ok(balance) => balance,
            Err(_) => return 0
        };
        let balance = balance.to_value().map(|v| v.to_u64().unwrap_or(0)).unwrap_or(0);
        balance
    }

    // Get the balance for a user based on its id
    pub async fn get_balance_for_user(&self, user: &User) -> u64 {
        let storage = self.wallet.get_storage().read().await;
        self.get_balance_internal(&storage, user.id.into())
    }

    // Generate a deposit address for a user based on its id
    pub fn get_address_for_user(&self, user: &User) -> Address {
        self.wallet.get_address_with(DataElement::Value(DataValue::U64(user.id.into())))
    }

    // Transfer XEL from one user to another
    pub async fn transfer(&self, from: &User, to: &User, amount: u64) -> Result<(), ServiceError> {
        if amount == 0 {
            return Err(ServiceError::Zero);
        }

        if from == to {
            return Err(ServiceError::SelfTip);
        }

        let from = from.id.into();
        let to = to.id.into();
        let mut storage = self.wallet.get_storage().write().await;
        let from_balance = self.get_balance_internal(&storage, from);
        if amount > from_balance {
            return Err(ServiceError::NotEnoughFunds(amount));
        }

        let to_balance = self.get_balance_internal(&storage, to);

        // Update balances
        storage.set_custom_data(BALANCES_TREE, &from.into(), &(from_balance - amount).into())?;
        storage.set_custom_data(BALANCES_TREE, &to.into(), &(to_balance + amount).into())?;

        Ok(())
    }

    // Withdraw XEL from the service to an address
    pub async fn withdraw(&self, user: &User, to: Address, amount: u64) -> Result<Hash, ServiceError> {
        if amount == 0 {
            return Err(ServiceError::Zero);
        }

        let user = user.id.into();
        let mut storage = self.wallet.get_storage().write().await;
        let (balance, fee, mut state, transaction) = {
            let balance = self.get_balance_internal(&storage, user);
            if amount > balance {
                return Err(ServiceError::NotEnoughFunds(amount));
            }

            let builder = TransactionTypeBuilder::Transfers(vec![TransferBuilder {
                    amount,
                    asset: XELIS_ASSET,
                    destination: to,
                    extra_data: None
                }
            ]);

            let fee = self.wallet.estimate_fees(builder.clone()).await?;
            // Verify if he has enough with fees included
            if fee + amount > balance {
                return Err(ServiceError::NotEnoughFundsForFee(fee));
            }

            let (state, transaction) = self.wallet.create_transaction_with_storage(&storage, builder, FeeBuilder::Value(fee)).await?;
            (balance, fee, state, transaction)
        };

        self.wallet.submit_transaction(&transaction).await?;

        // Update balance
        storage.set_custom_data(BALANCES_TREE, &user.into(), &(balance - (fee + amount)).into())?;
        state.apply_changes(&mut storage).await?;

        Ok(transaction.hash())
    }

    pub fn network(&self) -> &Network {
        self.wallet.get_network()
    }
}

impl Serializer for Deposit {
    fn write(&self, writer: &mut Writer) {
        writer.write_u64(&self.user_id);
        writer.write_u64(&self.amount);
        writer.write_hash(&self.transaction_hash);
    }

    fn read(reader: &mut Reader) -> Result<Self, ReaderError> {
        let user_id = reader.read_u64()?;
        let amount = reader.read_u64()?;
        let transaction_hash = reader.read_hash()?;
        Ok(Self { user_id, amount, transaction_hash })
    }
}