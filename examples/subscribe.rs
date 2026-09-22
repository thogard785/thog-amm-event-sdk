use event_driven_sdk::{Config, EventDrivenSdk, U256};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let sell = std::env::var("TOKEN_IN")?.parse()?;
    let buy = std::env::var("TOKEN_OUT")?.parse()?;
    let amount: U256 = std::env::var("AMOUNT_IN")?.parse()?;
    let gas_price: U256 = std::env::var("EFFECTIVE_GAS_PRICE_WEI")?.parse()?;
    let mut sdk = EventDrivenSdk::connect(
        std::env::var("THOGAMM_HTTP_RPC")?,
        std::env::var("THOGAMM_WS_RPC")?,
        std::env::var("THOGAMM_PROXY")?.parse()?,
        Config::default(),
    )
    .await?;

    loop {
        let model = sdk.model();
        match model.quote_execution_exact_input(sell, buy, amount, gas_price) {
            Ok(quote) => println!(
                "block={} amount_out={} last_posted_block={}",
                model.state().block.number,
                quote.amount_out,
                quote.last_posted_block
            ),
            Err(error) => eprintln!("block={} unavailable={error}", model.state().block.number),
        }
        sdk.next_update().await?;
    }
}
