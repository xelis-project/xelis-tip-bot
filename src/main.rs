mod service;

use anyhow::{Result, Error};
use clap::Parser;
use poise::{
    serenity_prelude::{
        GatewayIntents,
        ClientBuilder,
        CreateEmbed,
        CreateEmbedFooter,
        User,
        Colour
    },
    CreateReply
};
use service::{
    WalletService,
    WalletServiceImpl
};
use xelis_common::{
    network::Network,
    utils::{format_xelis, from_xelis},
    crypto::Address
};
use xelis_wallet::config::DEFAULT_DAEMON_ADDRESS;

// Context type for poise with our data type
type Context<'a> = poise::Context<'a, WalletService, Error>;

// Icon URL for thumbnail
const ICON: &str = "https://github.com/xelis-project/xelis-assets/raw/master/icons/png/square/green_background_black_logo.png?raw=true";
// Color of the embed
const COLOR: u32 = 196559;

#[derive(Parser)]
#[clap(version = "1.0.0", about = "XELIS Tip Bot")]
pub struct Config {
    /// Network selected for wallet
    #[clap(long, default_value_t = Network::Mainnet)]
    network: Network,
    /// Password for wallet
    #[clap(short, long)]
    password: String,
    /// Name for the wallet
    #[clap(short, long)]
    wallet_name: String,
    /// Daemon address for wallet
    #[clap(short, long, default_value_t = String::from(DEFAULT_DAEMON_ADDRESS))]
    daemon_address: String,
    /// Discord bot token
    #[clap(long)]
    token: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    let config = Config::parse();

    let service = WalletServiceImpl::new(config.wallet_name, config.password, config.daemon_address, config.network).await?;
    let intents = GatewayIntents::non_privileged() | GatewayIntents::MESSAGE_CONTENT;

    // Create the framework
    let framework = {
        let service = service.clone();
        poise::Framework::builder()
            .options(poise::FrameworkOptions {
                commands: vec![balance(), deposit(), withdraw(), tip()],
                ..Default::default()
            })
            .setup(|ctx, _ready, framework| {
                Box::pin(async move {
                    poise::builtins::register_globally(ctx, &framework.options().commands).await?;
                    Ok(service)
                })
            })
            .build()
    };

    // Create the client using token and intents
    let mut client = ClientBuilder::new(config.token, intents)
        .framework(framework)
        .await?;

    // start the service
    service.start(client.http.clone()).await?;

    // start listening for events by starting a single shard
    if let Err(why) = client.start().await {
        println!("An error occurred while running the client: {:?}", why);
    }

    Ok(())
}

/// Show your current balance
#[poise::command(slash_command, broadcast_typing)]
async fn balance(ctx: Context<'_>) -> Result<(), Error> {
    // Retrieve balance for user
    let service = ctx.data();
    let balance = service.get_balance_for_user(ctx.author()).await;

    let embed = CreateEmbed::default()
        .title("Balance")
        .field("Your balance is", format_xelis(balance), false)
        .thumbnail(ICON)
        .colour(COLOR);
    let mut reply = CreateReply::default()
        .embed(embed);

    // Set reply to ephemeral if command was not used in DM
    if ctx.channel_id().to_channel(ctx.http()).await?.private().is_none() {
        reply = reply.ephemeral(true);
    }

    // Send reply
    ctx.send(reply).await?;

    Ok(())
}

/// Show your deposit address
#[poise::command(slash_command, broadcast_typing)]
async fn deposit(ctx: Context<'_>) -> Result<(), Error> {
    // Retrieve address for user
    let service = ctx.data();
    let address = service.get_address_for_user(ctx.author());

    let embed = CreateEmbed::default()
        .title("Deposit")
        .field("Your deposit address is", address.to_string(), true)
        .footer(CreateEmbedFooter::new("Please do not send any other coins than XELIS to this address"))
        .thumbnail(ICON)
        .colour(COLOR);

    let mut reply = CreateReply::default()
        .embed(embed);

    // Set reply to ephemeral if command was not used in DM
    if ctx.channel_id().to_channel(ctx.http()).await?.private().is_none() {
        reply = reply.ephemeral(true);
    }

    // Send reply
    ctx.send(reply).await?;

    Ok(())
}

/// Withdraw from your balance
#[poise::command(slash_command, broadcast_typing)]
async fn withdraw(ctx: Context<'_>, address: String, amount: f64) -> Result<(), Error> {
    let service = ctx.data();
    let ephemeral = ctx.channel_id().to_channel(ctx.http()).await?.private().is_none();

    // Parse address in correct format
    let to = match Address::from_string(&address) {
        Ok(address) => address,
        Err(e) => {
            ctx.send(CreateReply::default().ephemeral(ephemeral).embed(
                CreateEmbed::default()
                    .title("Withdraw")
                    .field("An error occured while withdrawing", e.to_string(), false)
                    .thumbnail(ICON)
                    .colour(Colour::RED)
                )
            ).await?;
            return Ok(());
        }    
    };

    // Verify the address is in good network
    if to.is_mainnet() != service.network().is_mainnet() {
        ctx.send(CreateReply::default().ephemeral(ephemeral).embed(
            CreateEmbed::default()
                .title("Withdraw")
                .field("An error occured while withdrawing", "Invalid network", false)
                .thumbnail(ICON)
                .colour(Colour::RED)
            )
        ).await?;
        return Ok(());
    }

    // Parse amount in correct format
    let amount = match from_xelis(amount.to_string()) {
        Some(amount) => amount,
        None => {
            ctx.send(CreateReply::default().ephemeral(true).embed(
                CreateEmbed::default()
                    .title("Tip")
                    .field("An error occured while tipping", "Invalid amount", false)
                    .thumbnail(ICON)
                    .colour(Colour::RED)
                )
            ).await?;
            return Ok(());
        }
    };

    match service.withdraw(ctx.author(), to, amount).await {
        Ok(hash) => {
            ctx.send(CreateReply::default().ephemeral(ephemeral).embed(
                CreateEmbed::default()
                    .title("Withdraw")
                    .description(format!("You have withdrawn {} XEL", format_xelis(amount)))
                    .field("Transaction", hash.to_string(), false)
                    .thumbnail(ICON)
                    .colour(COLOR)
                )
            ).await?;
        },
        Err(e) => {
            ctx.send(CreateReply::default().ephemeral(ephemeral).embed(
                CreateEmbed::default()
                    .title("Withdraw")
                    .field("An error occured while withdrawing", e.to_string(), false)
                    .thumbnail(ICON)
                    .colour(Colour::RED)
                )
            ).await?;
        }
    };

    Ok(())
}

/// Tip a user with XELIS
#[poise::command(slash_command, broadcast_typing)]
async fn tip(ctx: Context<'_>, #[description = "User to tip"] user: User, #[description = "Amount to tip"] amount: f64) -> Result<(), Error> {
    let amount = match from_xelis(amount.to_string()) {
        Some(amount) => amount,
        None => {
            ctx.send(CreateReply::default().ephemeral(true).embed(
                CreateEmbed::default()
                    .title("Tip")
                    .field("An error occured while tipping", "Invalid amount", false)
                    .thumbnail(ICON)
                    .colour(Colour::RED)
                )
            ).await?;
            return Ok(());
        }
    };

    // Retrieve address for user
    let service = ctx.data();

    match service.transfer(ctx.author(), &user, amount).await {
        Ok(_) => {
            ctx.send(CreateReply::default().embed(
                CreateEmbed::default()
                    .title("Tip")
                    .description(format!("{} have tipped {} XEL to {}", ctx.author(), format_xelis(amount), user))
                    .thumbnail(ICON)
                    .colour(COLOR)
                )
            ).await?;
        },
        Err(e) => {
            ctx.send(CreateReply::default().ephemeral(true).embed(
                CreateEmbed::default()
                    .title("Tip")
                    .field("An error occured while tipping", e.to_string(), false)
                    .thumbnail(ICON)
                    .colour(Colour::RED)
                )
            ).await?;
        }
    };

    Ok(())
}