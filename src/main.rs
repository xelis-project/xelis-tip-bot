mod service;

use std::{sync::Arc, time::Duration};

use anyhow::{Error, Result};
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
    async_handler,
    crypto::Address,
    network::Network,
    prompt::{
        argument::ArgumentManager,
        command::{
            Command,
            CommandError,
            CommandHandler,
            CommandManager
        },
        LogLevel,
        ModuleConfig,
        Prompt,
        PromptError
    },
    utils::{format_xelis, from_xelis}
};
use xelis_wallet::config::DEFAULT_DAEMON_ADDRESS;
use log::error;

// Context type for poise with our data type
type Context<'a> = poise::Context<'a, WalletService, Error>;

// Icon URL for thumbnail
const ICON: &str = "https://github.com/xelis-project/xelis-assets/raw/master/icons/png/square/green_background_black_logo.png?raw=true";
// Color of the embed
const COLOR: u32 = 196559;

#[derive(Parser)]
#[clap(version = "1.0.0", about = "XELIS Tip Bot")]
#[command(styles = xelis_common::get_cli_styles())]
pub struct Config {
    /// Network selected for wallet
    #[clap(long, value_enum, default_value_t = Network::Mainnet)]
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
    /// Set log level
    #[clap(long, value_enum, default_value_t = LogLevel::Info)]
    log_level: LogLevel,
    /// Set file log level
    #[clap(long, value_enum, default_value_t = LogLevel::Info)]
    file_log_level: LogLevel,
    /// Disable the log file
    #[clap(long)]
    disable_file_logging: bool,
    /// Disable the log filename date based
    /// If disabled, the log file will be named xelis-tipbot.log instead of YYYY-MM-DD.xelis-tipbot.log
    #[clap(long)]
    disable_file_log_date_based: bool,
    /// Disable the usage of colors in log
    #[clap(long)]
    disable_log_color: bool,
    /// Disable terminal interactive mode
    /// You will not be able to write CLI commands in it or to have an updated prompt
    #[clap(long)]
    disable_interactive_mode: bool,
    /// Log filename
    /// 
    /// By default filename is xelis-wallet.log.
    /// File will be stored in logs directory, this is only the filename, not the full path.
    /// Log file is rotated every day and has the format YYYY-MM-DD.xelis-tipbot.log.
    #[clap(long, default_value_t = String::from("xelis-tipbot.log"))]
    filename_log: String,
    /// Logs directory
    /// 
    /// By default it will be logs/ of the current directory.
    /// It must end with a / to be a valid folder.
    #[clap(long, default_value_t = String::from("logs/"))]
    logs_path: String,
    /// Module configuration for logs
    #[clap(long)]
    logs_modules: Vec<ModuleConfig>,
    /// Set the path for wallet storage to open/create a wallet at this location
    #[clap(long)]
    wallet_path: Option<String>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let mut config = Config::parse();

    let service = WalletServiceImpl::new(config.wallet_name, config.password, config.daemon_address, config.network).await?;
    let intents = GatewayIntents::non_privileged() | GatewayIntents::MESSAGE_CONTENT;

    // Create the framework
    let framework = {
        let service = service.clone();
        poise::Framework::builder()
            .options(poise::FrameworkOptions {
                commands: vec![status(), balance(), deposit(), withdraw(), tip()],
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
    Arc::clone(&service).start(client.http.clone()).await?;

    config.logs_modules.push(ModuleConfig { module: "serenity".to_string(), level: LogLevel::Warn });
    let prompt = Prompt::new(config.log_level, &config.logs_path, &config.filename_log, config.disable_file_logging, config.disable_file_log_date_based, config.disable_log_color, !config.disable_interactive_mode, config.logs_modules, config.file_log_level)?;
    let command_manager = CommandManager::new(prompt.clone());
    command_manager.store_in_context(service)?;

    command_manager.register_default_commands()?;
    command_manager.add_command(Command::new("rescan", "Rescan the wallet", CommandHandler::Async(async_handler!(rescan))))?;
    command_manager.display_commands()?;

    tokio::select! {
        // start listening for events by starting a single shard
        res = client.start() => {
            if let Err(e) = res {
                error!("An error occurred while running the client: {:?}", e);
            }
        },
        res = prompt.start(Duration::from_millis(1000), Box::new(async_handler!(prompt_message_builder)), Some(&command_manager)) => {
            if let Err(e) = res {
                error!("An error occurred while running the prompt: {:?}", e);
            }
        }
    };

    Ok(())
}

// Default prompt message builder
async fn prompt_message_builder(_: &Prompt, _: Option<&CommandManager>) -> Result<String, PromptError> {
    Ok("XELIS Tip Bot >>".to_string())
}

async fn rescan(manager: &CommandManager, _: ArgumentManager) -> Result<(), CommandError> {
    let context = manager.get_context().lock()?;
    let service: &WalletService = context.get()?;
    if let Err(e) = service.rescan().await {
        manager.error(format!("An error occurred while rescanning the wallet: {}", e.to_string()));
    } else {
        manager.message("Wallet has been rescanned");
    }

    Ok(())
}


/// Show your current balance
#[poise::command(slash_command, broadcast_typing)]
async fn status(ctx: Context<'_>) -> Result<(), Error> {
    // Retrieve balance for user
    let service = ctx.data();
    let balance = service.get_wallet_balance().await?;
    let topoheight = service.get_wallet_topoheight().await?;
    let network = service.network();
    let online = service.is_wallet_online().await;

    let embed = CreateEmbed::default()
        .title("Status")
        .field("Balance: ", format_xelis(balance), false)
        .field("Synced TopoHeight: ", topoheight.to_string(), false)
        .field("Network: ", network.to_string(), false)
        .field("Is Online: ", online.to_string(), false)
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