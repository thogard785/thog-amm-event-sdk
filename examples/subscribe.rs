use event_driven_sdk::{Address, Config, Error, EventDrivenSdk, Result, U256};
use std::time::Duration;

async fn follow_quotes(
    http_rpc: &str,
    ws_rpc: &str,
    pool: Address,
    token_in: Address,
    token_out: Address,
    amount_in: U256,
    gas_price: U256,
) -> Result<()> {
    let mut delay = Duration::from_millis(250);
    loop {
        let mut sdk = match EventDrivenSdk::connect(http_rpc, ws_rpc, pool, Config::default()).await
        {
            Ok(sdk) => sdk,
            Err(error @ (Error::Transport(_) | Error::Rpc(_))) => {
                eprintln!("connect failed: {error}; retrying in {delay:?}");
                tokio::time::sleep(delay).await;
                delay = (delay * 2).min(Duration::from_secs(8));
                continue;
            }
            Err(error) => return Err(error),
        };
        loop {
            match sdk.next_update().await {
                Ok(update) => {
                    delay = Duration::from_millis(250);
                    let model = sdk.model()?;
                    match model
                        .quote_execution_exact_input(token_in, token_out, amount_in, gas_price)
                    {
                        Ok(q) => println!(
                            "block={} timestamp={} amount_out={}",
                            update.block.number, update.block.timestamp, q.amount_out
                        ),
                        Err(error) => eprintln!("quote unavailable: {error}"),
                    }
                }
                Err(
                    error @ (Error::Transport(_)
                    | Error::Rpc(_)
                    | Error::Discontinuous
                    | Error::RegistryChanged
                    | Error::ContractUpgraded
                    | Error::BalanceCheckpointMismatch { .. }),
                ) => {
                    eprintln!("subscription failed: {error}; reconnecting in {delay:?}");
                    break;
                }
                Err(error) => return Err(error),
            }
        }
        tokio::time::sleep(delay).await;
        delay = (delay * 2).min(Duration::from_secs(8));
    }
}

#[tokio::main]
async fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    follow_quotes(
        &std::env::var("THOGAMM_HTTP_RPC")?,
        &std::env::var("THOGAMM_WS_RPC")?,
        std::env::var("THOGAMM_PROXY")?.parse()?,
        std::env::var("TOKEN_IN")?.parse()?,
        std::env::var("TOKEN_OUT")?.parse()?,
        std::env::var("AMOUNT_IN")?.parse()?,
        std::env::var("EFFECTIVE_GAS_PRICE_WEI")?.parse()?,
    )
    .await?;
    Ok(())
}
