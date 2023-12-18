mod service;

use anyhow::Result;
use clap::Parser;
use serenity::{
    Client,
    all::{GatewayIntents, Message},
    framework::{
        standard::{
            Configuration, macros::{group, command}, CommandResult
        },
        StandardFramework
    },
    client::Context, prelude::TypeMapKey,
    builder::{CreateMessage, CreateEmbed, CreateEmbedFooter},
    model::Colour,
};
use service::WalletService;
use xelis_common::{network::Network, utils::format_xelis, config::COIN_DECIMALS};

const ICON: &str = "https://github.com/xelis-project/xelis-assets/raw/master/icons/png/square/green_background_black_logo.png?raw=true";
const COLOR: u32 = 196559;

#[group]
#[commands(balance, deposit, withdraw, tip)]
struct General;

impl TypeMapKey for WalletService {
    type Value = WalletService;
}

#[derive(Parser)]
#[clap(version = "1.0.0", about = "XELIS Tip Bot")]
pub struct Config {
    /// Network selected for wallet
    #[clap(long, arg_enum, default_value_t = Network::Mainnet)]
    network: Network,
    // Password for wallet
    #[clap(short, long)]
    password: String,
    // Name for the wallet
    #[clap(short, long)]
    wallet_name: String,
    // Discord bot token
    #[clap(long)]
    token: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    let config = Config::parse();
    let framework = StandardFramework::new().group(&GENERAL_GROUP);
    framework.configure(Configuration::new().prefix("/")); // set the bot's prefix to "~"

    let service = WalletService::new(config.wallet_name, config.password, config.network)?;
    let intents = GatewayIntents::non_privileged() | GatewayIntents::MESSAGE_CONTENT;
    let mut client = Client::builder(config.token, intents)
        .framework(framework)
        .type_map_insert::<WalletService>(service)
        .await?;

    // start listening for events by starting a single shard
    if let Err(why) = client.start().await {
        println!("An error occurred while running the client: {:?}", why);
    }

    Ok(())
}

#[command]
async fn balance(ctx: &Context, msg: &Message) -> CommandResult {
    let channel = msg.channel_id.to_channel(&ctx.http).await?;
    // Only allow this command in DM
    if channel.private().is_none() {
        return Ok(());
    }

    let typing = msg.channel_id.start_typing(&ctx.http);
    let data = ctx.data.read().await;
    let service = data.get::<WalletService>().unwrap();
    let balance = service.get_balance_for_user(msg.author.to_string()).await;
    msg.channel_id.send_message(&ctx.http, CreateMessage::new()
        .add_embed(CreateEmbed::default()
            .title("Balance")
            .field("Your balance is", format_xelis(balance), false)
            .thumbnail(ICON)
            .colour(COLOR)
        )
    ).await?;
    typing.stop();

    Ok(())
}

#[command]
async fn deposit(ctx: &Context, msg: &Message) -> CommandResult {
    let channel = msg.channel_id.to_channel(&ctx.http).await?;
    // Only allow this command in DM
    if channel.private().is_none() {
        return Ok(());
    }

    let typing = msg.channel_id.start_typing(&ctx.http);
    let data = ctx.data.read().await;
    let service = data.get::<WalletService>().unwrap();
    let address = service.get_address_for_user(msg.author.to_string());

    msg.channel_id.send_message(&ctx.http, CreateMessage::new()
        .add_embed(CreateEmbed::default()
            .title("Deposit")
            .field("Your deposit address is", address.to_string(), true)
            .footer(CreateEmbedFooter::new("Please do not send any other coins than XELIS to this address"))
            .thumbnail(ICON)
            .colour(COLOR)
        )
    ).await?;
    typing.stop();

    Ok(())
}


#[command]
async fn withdraw(ctx: &Context, msg: &Message) -> CommandResult {
    let channel = msg.channel_id.to_channel(&ctx.http).await?;
    // Only allow this command in DM
    if channel.private().is_none() {
        return Ok(());
    }

    let typing = msg.channel_id.start_typing(&ctx.http);
    let data = ctx.data.read().await;
    let _ = data.get::<WalletService>().unwrap();
    // TODO: implement withdraw

    typing.stop();

    Ok(())
}

#[command]
async fn tip(ctx: &Context, msg: &Message) -> CommandResult {
    let channel = msg.channel_id.to_channel(&ctx.http).await?;
    // restrict this command in servers only
    if channel.private().is_some() {
        msg.channel_id.send_message(&ctx.http, CreateMessage::new()
            .add_embed(CreateEmbed::default()
                .title("Tip")
                .description("This command is only available in servers")
                .thumbnail(ICON)
                .colour(Colour::RED)
            )
        ).await?;
        return Ok(());
    }

    let typing = msg.channel_id.start_typing(&ctx.http);
    let args: Vec<&str> = msg.content.split_whitespace().collect();

    if args.len() != 3 {
        msg.channel_id.send_message(&ctx.http, CreateMessage::new()
            .add_embed(CreateEmbed::default()
                .title("Tip")
                .description("Please provide a user and an amount to tip")
                .thumbnail(ICON)
                .colour(Colour::RED)
            )
        ).await?;
        return Ok(());
    }

    let user = match msg.mentions.first() {
        Some(user) if !user.bot => user.id.to_string(),
        _ => {
            msg.channel_id.send_message(&ctx.http, CreateMessage::new()
                .add_embed(CreateEmbed::default()
                    .title("Tip")
                    .description("Please provide a valid user to tip")
                    .thumbnail(ICON)
                    .colour(Colour::RED)
                )
            ).await?;
            return Ok(());
        }
    };

    let amount = match from_xelis(args[2]) {
        Some(amount) => amount,
        None => {
            msg.channel_id.send_message(&ctx.http, CreateMessage::new()
                .add_embed(CreateEmbed::default()
                    .title("Tip")
                    .description("Please provide a valid amount to tip")
                    .thumbnail(ICON)
                    .colour(Colour::RED)
                )
            ).await?;
            return Ok(());
        }
    };

    let data = ctx.data.read().await;
    let service = data.get::<WalletService>().unwrap();

    match service.transfer(msg.author.id.to_string(), user.clone(), amount).await {
        Ok(_) => {
            msg.channel_id.send_message(&ctx.http, CreateMessage::new()
                .add_embed(CreateEmbed::default()
                    .title("Tip")
                    .description(format!("You have tipped {} XEL to {}", format_xelis(amount), user))
                    .thumbnail(ICON)
                    .colour(COLOR)
                )
            ).await?;
        },
        Err(e) => {
            msg.channel_id.send_message(&ctx.http, CreateMessage::new()
                .add_embed(CreateEmbed::default()
                    .title("Tip")
                    .field("An error occured while tipping", e.to_string(), false)
                    .thumbnail(ICON)
                    .colour(Colour::RED)
                )
            ).await?;
        }
    };

    typing.stop();

    Ok(())
}

// Convert a XELIS amount from string to a u64
pub fn from_xelis(value: impl Into<String>) -> Option<u64> {
    let value = value.into();
    let mut split = value.split('.');
    let xelis: u64 = split.next()?.parse::<u64>().ok()?;
    let decimals = split.next().unwrap_or("0");
    if decimals.len() > COIN_DECIMALS as usize {
        return None;
    }

    let mut decimals = decimals.parse::<u64>().ok()?;
    while decimals > 0 && decimals % 10 == 0 {
        decimals /= 10;
    }

    Some(xelis * 10u64.pow(COIN_DECIMALS as u32) + decimals)
}
